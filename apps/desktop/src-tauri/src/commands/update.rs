use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{Manager, ResourceId, Runtime, Webview};
use tauri_plugin_updater::UpdaterExt;
use url::Url;

const GITHUB_UPDATE_ENDPOINT: &str =
    "https://github.com/MLGBJDLW/Nexa/releases/latest/download/latest.json";
const GHFAST_BASE_URL: &str = "https://ghfast.top";

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateSource {
    Github,
    Ghfast,
    Custom,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetadata {
    rid: ResourceId,
    current_version: String,
    version: String,
    date: Option<String>,
    body: Option<String>,
    raw_json: serde_json::Value,
}

fn parse_endpoint(value: &str) -> Result<Url, String> {
    Url::parse(value).map_err(|error| format!("Invalid update endpoint {value}: {error}"))
}

fn mirror_for_source(
    source: UpdateSource,
    custom_mirror: Option<&str>,
) -> Result<Option<String>, String> {
    let value = match source {
        UpdateSource::Github => return Ok(None),
        UpdateSource::Ghfast => GHFAST_BASE_URL,
        UpdateSource::Custom => custom_mirror.unwrap_or_default().trim(),
    };
    let url = parse_endpoint(value)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an HTTPS mirror base URL without credentials, query, or fragment".into());
    }
    Ok(Some(url.as_str().trim_end_matches('/').to_owned()))
}

fn mirrored_url(original: &Url, mirror: Option<&str>) -> Result<Url, String> {
    match mirror {
        None => Ok(original.clone()),
        Some(base) => {
            // A mirror changes transport only. Manifest assets must still name
            // this repository, and the updater retains its pinned signing key.
            if original.scheme() != "https"
                || original.host_str() != Some("github.com")
                || !original.path().starts_with("/MLGBJDLW/Nexa/releases/")
                || !original.username().is_empty()
                || original.password().is_some()
                || original.port().is_some()
            {
                return Err("Update asset is not an official Nexa GitHub release URL".into());
            }
            parse_endpoint(&format!("{base}/{original}"))
        }
    }
}

#[tauri::command]
pub async fn check_update_from_source_cmd<R: Runtime>(
    webview: Webview<R>,
    source: UpdateSource,
    timeout: Option<u64>,
    custom_mirror: Option<String>,
) -> Result<Option<UpdateMetadata>, String> {
    let mirror = mirror_for_source(source, custom_mirror.as_deref())?;
    let endpoints = vec![mirrored_url(
        &parse_endpoint(GITHUB_UPDATE_ENDPOINT)?,
        mirror.as_deref(),
    )?];
    let mut builder = webview
        .updater_builder()
        .endpoints(endpoints)
        .map_err(|error| error.to_string())?;

    if let Some(timeout) = timeout {
        builder = builder.timeout(Duration::from_millis(timeout));
    }

    let updater = builder.build().map_err(|error| error.to_string())?;
    let mut update = updater.check().await.map_err(|error| error.to_string())?;
    if let Some(update) = update.as_mut() {
        update.download_url = mirrored_url(&update.download_url, mirror.as_deref())?;
    }

    Ok(update.map(|update| {
        let metadata = UpdateMetadata {
            current_version: update.current_version.clone(),
            version: update.version.clone(),
            date: update.date.as_ref().map(ToString::to_string),
            body: update.body.clone(),
            raw_json: update.raw_json.clone(),
            rid: webview.resources_table().add(update),
        };
        metadata
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_cover_manifest_and_asset_without_changing_the_origin() {
        for source in [UpdateSource::Ghfast, UpdateSource::Custom] {
            let mirror = mirror_for_source(source, Some("https://mirror.example/proxy/")).unwrap();
            for original in [
                GITHUB_UPDATE_ENDPOINT,
                "https://github.com/MLGBJDLW/Nexa/releases/download/v1.0.0/Nexa.nsis.zip",
            ] {
                let result =
                    mirrored_url(&parse_endpoint(original).unwrap(), mirror.as_deref()).unwrap();
                assert_eq!(
                    result.as_str(),
                    format!("{}/{original}", mirror.as_ref().unwrap())
                );
            }
        }
        assert!(mirror_for_source(UpdateSource::Github, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn invalid_mirrors_and_foreign_assets_are_rejected() {
        for value in [
            "",
            "http://mirror.example",
            "https://u:p@mirror.example",
            "https://mirror.example?token=x",
            "https://mirror.example/#x",
        ] {
            assert!(mirror_for_source(UpdateSource::Custom, Some(value)).is_err());
        }
        let foreign = parse_endpoint("https://example.com/payload.zip").unwrap();
        assert!(mirrored_url(&foreign, Some(GHFAST_BASE_URL)).is_err());
    }
}
