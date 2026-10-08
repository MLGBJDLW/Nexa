//! Resolve public skill packages without borrowing an unrelated asset tool.

use std::{collections::HashMap, fs, io::Cursor, path::PathBuf, time::Duration};

use super::{fetch_url_tool, path_utils, ToolExecutionContext};
use crate::{
    error::CoreError,
    skills::{DiscoveredSkillBundle, SkillInstallSelection},
};

const MAX_DOWNLOAD_BYTES: usize = 64 * 1024 * 1024;

struct DownloadSpec {
    url: reqwest::Url,
    github_directory: Option<String>,
}

pub(super) struct PreparedSources {
    // Own every downloaded file until inspection/installation has finished.
    _temporary: tempfile::TempDir,
    pub paths: Vec<PathBuf>,
    pub previews: Vec<DiscoveredSkillBundle>,
    identities: HashMap<String, String>,
}

impl PreparedSources {
    pub fn selection(
        &self,
        selected: &[SkillInstallSelection],
    ) -> Result<Vec<SkillInstallSelection>, CoreError> {
        selected
            .iter()
            .map(|item| {
                let skill_file = self.identities.get(&item.skill_file).ok_or_else(|| {
                    CoreError::InvalidInput(
                        "Selected skill is not present in these sources; inspect them again."
                            .into(),
                    )
                })?;
                Ok(SkillInstallSelection {
                    skill_file: skill_file.clone(),
                    content_digest: item.content_digest.clone(),
                })
            })
            .collect()
    }
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::InvalidInput(message.into())
}

fn decode_segment(value: &str) -> Result<String, CoreError> {
    let mut result = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes
                .get(index + 1..index + 3)
                .ok_or_else(|| invalid("Invalid URL path escape"))?;
            let hex = std::str::from_utf8(hex).map_err(|_| invalid("Invalid URL path escape"))?;
            result
                .push(u8::from_str_radix(hex, 16).map_err(|_| invalid("Invalid URL path escape"))?);
            index += 3;
        } else {
            result.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(result).map_err(|_| invalid("Skill source path must be UTF-8"))
}

fn download_spec(source: &str, github_ref: Option<&str>) -> Result<DownloadSpec, CoreError> {
    let url = reqwest::Url::parse(source).map_err(|_| invalid("Invalid skill source URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid(
            "Skill downloads require an HTTP(S) URL without embedded credentials",
        ));
    }
    let host = url.host_str().unwrap_or_default();
    if !matches!(
        host,
        "github.com" | "www.github.com" | "raw.githubusercontent.com"
    ) {
        return Ok(DownloadSpec {
            url,
            github_directory: None,
        });
    }
    let parts = url
        .path_segments()
        .ok_or_else(|| invalid("Invalid GitHub source path"))?
        .filter(|part| !part.is_empty())
        .map(decode_segment)
        .collect::<Result<Vec<_>, _>>()?;
    if parts.len() < 2
        || parts[..2].iter().any(|part| {
            part == "."
                || part == ".."
                || !part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        })
    {
        return Err(invalid("GitHub sources need an owner and repository"));
    }
    let mut revision = github_ref.unwrap_or("HEAD").to_string();
    let mut directory = String::new();
    if parts.len() > 2 {
        let offset = if host == "raw.githubusercontent.com" {
            2
        } else {
            if !matches!(parts[2].as_str(), "tree" | "blob") {
                // Direct archive/download URLs still use the normal package loader.
                return Ok(DownloadSpec {
                    url,
                    github_directory: None,
                });
            }
            3
        };
        let rest = parts.get(offset..).unwrap_or_default().join("/");
        if let Some(explicit) = github_ref {
            directory = rest
                .strip_prefix(&format!("{explicit}/"))
                .or_else(|| (rest == explicit).then_some(""))
                .ok_or_else(|| invalid("github_ref must match the reference in the GitHub URL"))?
                .to_string();
        } else {
            revision = parts
                .get(offset)
                .filter(|part| !part.is_empty())
                .ok_or_else(|| invalid("Missing GitHub reference"))?
                .clone();
            directory = parts.get(offset + 1..).unwrap_or_default().join("/");
        }
        if directory.ends_with("/SKILL.md") {
            directory.truncate(directory.len() - "/SKILL.md".len());
        } else if directory == "SKILL.md" {
            directory.clear();
        } else if host == "raw.githubusercontent.com"
            || parts.get(2).is_some_and(|kind| kind == "blob")
        {
            return Err(invalid("Use a GitHub skill directory or SKILL.md URL"));
        }
    }
    if revision.is_empty()
        || revision.chars().any(char::is_control)
        || revision.contains('\\')
        || directory
            .split('/')
            .any(|part| matches!(part, "." | "..") || part.contains('\\'))
    {
        return Err(invalid("Invalid GitHub reference or skill directory"));
    }
    let mut archive = reqwest::Url::parse("https://codeload.github.com").expect("constant URL");
    archive
        .path_segments_mut()
        .expect("hierarchical URL")
        .extend([
            parts[0].as_str(),
            parts[1].trim_end_matches(".git"),
            "zip",
            revision.as_str(),
        ]);
    Ok(DownloadSpec {
        url: archive,
        github_directory: Some(directory),
    })
}

pub(super) fn validate_sources(
    sources: &[String],
    github_ref: Option<&str>,
) -> Result<(), CoreError> {
    let mut github = false;
    for source in sources {
        if source.contains("://") {
            let spec = download_spec(source, github_ref)?;
            github |= spec.github_directory.is_some();
        }
    }
    if github_ref.is_some() && !github {
        return Err(invalid(
            "github_ref is only valid with a GitHub repository or skill URL",
        ));
    }
    Ok(())
}

fn filter_github_archive(bytes: &[u8], directory: &str) -> Result<Vec<u8>, CoreError> {
    let mut source = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| invalid(format!("Invalid GitHub archive: {error}")))?;
    if source.len() > 100_000 {
        return Err(invalid(
            "GitHub archive exceeds 100,000 entries; use a local checkout of the skill directory",
        ));
    }
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut found = false;
    let mut wrapper = None;
    for index in 0..source.len() {
        let entry = source
            .by_index(index)
            .map_err(|error| invalid(error.to_string()))?;
        let path = entry
            .enclosed_name()
            .ok_or_else(|| invalid("Unsafe path in GitHub archive"))?;
        let path = path.to_string_lossy().replace('\\', "/");
        let (root, relative) = path.split_once('/').unwrap_or((&path, ""));
        if wrapper.as_ref().is_some_and(|previous| previous != root) {
            return Err(invalid("GitHub archive has multiple root directories"));
        }
        wrapper.get_or_insert_with(|| root.to_string());
        if entry.is_dir() || relative.is_empty() {
            continue;
        }
        if directory.is_empty() || relative.starts_with(&format!("{directory}/")) {
            output
                .raw_copy_file_rename(entry, relative)
                .map_err(|error| invalid(error.to_string()))?;
            found = true;
        }
    }
    if !found {
        return Err(invalid("No files found at that GitHub skill directory. For branch names containing '/', pass github_ref explicitly."));
    }
    output
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|error| invalid(error.to_string()))
}

async fn download(spec: &DownloadSpec) -> Result<Vec<u8>, CoreError> {
    crate::privacy::runtime::ensure_invocation_current()?;
    let url = fetch_url_tool::validate_url_for_fetch(spec.url.as_str())
        .await
        .map_err(invalid)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(crate::USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| invalid(error.to_string()))?;
    crate::privacy::runtime::ensure_invocation_current()?;
    let (response, _, _) = fetch_url_tool::send_with_safe_redirects(&client, url)
        .await
        .map_err(invalid)?;
    if !response.status().is_success() {
        return Err(invalid(format!("Skill download returned HTTP {}. For private repositories, authenticate a local checkout and import its directory. For rate limits, retry later.", response.status().as_u16())));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_DOWNLOAD_BYTES as u64)
    {
        return Err(invalid(
            "Skill download exceeds 64 MiB; import a smaller package or local skill directory",
        ));
    }
    let (bytes, truncated) = fetch_url_tool::read_limited_body(response, MAX_DOWNLOAD_BYTES)
        .await
        .map_err(invalid)?;
    if truncated {
        return Err(invalid(
            "Skill download exceeds 64 MiB; import a smaller package or local skill directory",
        ));
    }
    Ok(bytes)
}

fn persist_download(
    bytes: &[u8],
    spec: &DownloadSpec,
    base: PathBuf,
) -> Result<PathBuf, CoreError> {
    if let Some(directory) = &spec.github_directory {
        let path = base.with_extension("zip");
        fs::write(&path, filter_github_archive(bytes, directory)?)?;
        return Ok(path);
    }
    let (extension, body) = if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
    {
        ("zip", bytes)
    } else {
        let body = std::str::from_utf8(bytes)
            .map_err(|_| invalid("Skill source is neither a ZIP package nor UTF-8 Markdown"))?;
        if !body
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with("---")
        {
            return Err(invalid("Skill source did not return SKILL.md frontmatter or a ZIP archive. Use a repository/skill URL, not an HTML page or login page."));
        }
        ("md", bytes)
    };
    let path = base.with_extension(extension);
    fs::write(&path, body)?;
    Ok(path)
}

pub(super) async fn prepare(
    context: &ToolExecutionContext<'_>,
    sources: &[String],
    github_ref: Option<&str>,
) -> Result<PreparedSources, CoreError> {
    let policy = super::file_access_policy_for_context(context)?;
    let temporary = tempfile::Builder::new()
        .prefix("nexa-skill-download-")
        .tempdir()?;
    let mut result = PreparedSources {
        _temporary: temporary,
        paths: Vec::new(),
        previews: Vec::new(),
        identities: HashMap::new(),
    };
    let mut seen_sources = std::collections::HashSet::new();
    for (index, source) in sources.iter().enumerate() {
        if !seen_sources.insert(source) {
            continue;
        }
        crate::privacy::runtime::ensure_invocation_current()?;
        let remote = source.starts_with("https://") || source.starts_with("http://");
        let path = if remote {
            let spec = download_spec(source, github_ref)?;
            let bytes = download(&spec).await?;
            persist_download(
                &bytes,
                &spec,
                result._temporary.path().join(format!("source-{index}")),
            )?
        } else {
            path_utils::resolve_path_for_file_access(
                &PathBuf::from(source),
                &policy.sources,
                path_utils::PathKind::Any,
                false,
                policy.allow_unregistered_absolute_paths,
            )
            .map_err(invalid)?
        };
        let mut previews = crate::skills::inspect_skill_install_source(&path)?;
        for preview in &mut previews {
            let original = preview.skill_file.clone();
            if remote {
                let suffix = original
                    .split_once("!/")
                    .map(|(_, suffix)| format!("!/{suffix}"))
                    .unwrap_or_default();
                preview.skill_file = format!("{source}{suffix}");
                preview.skill_dir = preview
                    .skill_file
                    .rsplit_once("/SKILL.md")
                    .map(|(directory, _)| directory.to_string())
                    .unwrap_or_else(|| source.clone());
            }
            if result
                .identities
                .insert(preview.skill_file.clone(), original)
                .is_none()
            {
                result.previews.push(preview.clone());
            }
        }
        result.paths.push(path);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    #[ignore = "Live public GitHub probe; explicitly run without account credentials"]
    async fn public_github_skill_roundtrips_through_install_activation_and_resource_readback() {
        use super::super::{manage_skill_tool::ManageSkillTool, Tool};
        let db = crate::db::Database::open_memory().unwrap();
        let sources = vec![
            "https://github.com/openai/skills/tree/main/skills/.system/skill-installer".to_string(),
        ];
        let context = ToolExecutionContext::new("probe", "{}", &db, &[]);
        let prepared = prepare(&context, &sources, None).await.unwrap();
        assert_eq!(prepared.previews.len(), 1);
        let selected = vec![SkillInstallSelection {
            skill_file: prepared.previews[0].skill_file.clone(),
            content_digest: prepared.previews[0].content_digest.clone(),
        }];
        assert!(selected[0].skill_file.starts_with(&sources[0]));
        let destination = tempfile::tempdir().unwrap();
        let installed = crate::skills::install_skills_from_sources(
            &db,
            &prepared.paths,
            Some(&prepared.selection(&selected).unwrap()),
            false,
            true,
            destination.path(),
        )
        .unwrap();
        let skill = &installed[0];
        assert!(destination
            .path()
            .join("skill-installer/scripts/install-skill-from-github.py")
            .is_file());
        for (action, resource) in [
            ("activate_skill", None),
            (
                "view_resource",
                Some("scripts/install-skill-from-github.py"),
            ),
        ] {
            let mut args = serde_json::json!({"action":action,"skill_id":skill.id});
            if let Some(resource) = resource {
                args["resource_path"] = serde_json::json!(resource);
            }
            let args = args.to_string();
            let result = ManageSkillTool
                .execute(ToolExecutionContext::new("readback", &args, &db, &[]))
                .await
                .unwrap();
            assert!(!result.is_error);
            assert!(result.content.len() > 200);
        }
    }

    #[test]
    fn github_urls_resolve_complete_packages_and_explicit_slash_references() {
        for source in [
            "https://github.com/acme/repo/tree/main/skills/demo",
            "https://github.com/acme/repo/blob/main/skills/demo/SKILL.md",
            "https://raw.githubusercontent.com/acme/repo/main/skills/demo/SKILL.md",
        ] {
            let spec = download_spec(source, None).unwrap();
            assert_eq!(
                spec.url.as_str(),
                "https://codeload.github.com/acme/repo/zip/main"
            );
            assert_eq!(spec.github_directory.as_deref(), Some("skills/demo"));
        }
        let spec = download_spec(
            "https://github.com/acme/repo/tree/feature/import/skills/demo",
            Some("feature/import"),
        )
        .unwrap();
        assert!(spec.url.as_str().ends_with("/feature%2Fimport"));
        assert_eq!(spec.github_directory.as_deref(), Some("skills/demo"));
        assert!(download_spec("https://user:password@github.com/acme/repo", None).is_err());
    }

    #[test]
    fn github_subtree_preserves_resources_without_importing_unrelated_skills() {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, body) in [
            (
                "repo-main/skills/demo/SKILL.md",
                "---\nname: demo\ndescription: Example skill\n---\nUse templates/report.md",
            ),
            ("repo-main/skills/demo/templates/report.md", "Full template"),
            (
                "repo-main/skills/broken/SKILL.md",
                "Invalid unrelated skill",
            ),
        ] {
            archive
                .start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(body.as_bytes()).unwrap();
        }
        let bytes = archive.finish().unwrap().into_inner();
        let temp = tempfile::tempdir().unwrap();
        let spec =
            download_spec("https://github.com/acme/repo/tree/main/skills/demo", None).unwrap();
        let path = persist_download(&bytes, &spec, temp.path().join("download")).unwrap();
        let previews = crate::skills::inspect_skill_install_source(&path).unwrap();
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].name, "demo");
        assert_eq!(previews[0].resources[0].path, "templates/report.md");
        assert!(persist_download(
            b"<html>Sign in</html>",
            &DownloadSpec {
                url: reqwest::Url::parse("https://example.com/skill.md").unwrap(),
                github_directory: None
            },
            temp.path().join("html")
        )
        .is_err());
    }
}
