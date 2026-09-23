//! Public OpenRouter image discovery. Capabilities belong to the image endpoint,
//! not to similarly named chat models or direct vendor APIs.
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::error::CoreError;
use crate::image_provider_catalog::ImageModelPreset;

type CatalogCache = Option<(Instant, Vec<ImageModelPreset>)>;

pub fn parse_models(value: &Value) -> Vec<ImageModelPreset> {
    value["data"].as_array().into_iter().flatten().filter_map(|model| {
        let id = model["id"].as_str()?.trim();
        if id.is_empty() { return None; }
        let parameters = &model["supported_parameters"];
        // The current tool produces raster images from text. Do not offer
        // vector-only models or models requiring style reference uploads.
        if parameters.pointer("/input_references/min").and_then(Value::as_u64).unwrap_or(0) > 0 { return None; }
        let formats: Vec<String> = parameters.get("output_format").map(|format| {
            format["values"].as_array().into_iter().flatten().filter_map(Value::as_str)
                .filter(|format| matches!(*format, "png" | "jpeg" | "webp")).map(str::to_string).collect()
        }).unwrap_or_else(|| vec!["png".into()]);
        if formats.is_empty() { return None; }
        if !model.pointer("/architecture/output_modalities").and_then(Value::as_array)?.iter().any(|v| v == "image") { return None; }
        let values = |name: &str| -> Vec<String> { parameters[name]["values"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect() };
        let ratios = values("aspect_ratio");
        let resolutions = values("resolution");
        let mut sizes = vec![json!({"value":"auto", "label":"Auto"})];
        for ratio in &ratios {
            if resolutions.is_empty() { sizes.push(json!({"value":ratio,"label":ratio})); }
            else { for resolution in &resolutions { sizes.push(json!({"value":format!("{ratio}|{resolution}"),"label":format!("{ratio} · {resolution}")})); } }
        }
        if ratios.is_empty() { for resolution in resolutions { sizes.push(json!({"value":resolution,"label":resolution})); } }
        Some(ImageModelPreset {
            id:id.into(), name:model["name"].as_str().unwrap_or(id).into(),
            recommended:id == "google/gemini-3.1-flash-image", quality_options:Some(values("quality")),
            metadata:json!({"source":"official", "productReadiness":"known", "sizeOptions":sizes,
                "inputModalities":["text"], "outputModalities":["image"], "outputFormats":formats, "supportedParameters":parameters,
                "lastVerifiedAt":chrono::Utc::now().format("%Y-%m-%d").to_string()}).as_object().unwrap().clone(),
        })
    }).collect()
}

pub async fn discover_models() -> Result<Vec<ImageModelPreset>, CoreError> {
    static CACHE: OnceLock<Mutex<CatalogCache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(|| Mutex::new(None)).lock().await;
    if let Some((time, models)) = cache.as_ref() {
        if time.elapsed() < Duration::from_secs(600) {
            return Ok(models.clone());
        }
    }
    let mut response = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| CoreError::Llm(e.to_string()))?
        .get("https://openrouter.ai/api/v1/images/models")
        .send()
        .await
        .map_err(|e| CoreError::TransientLlm(format!("OpenRouter image discovery failed: {e}")))?
        .error_for_status()
        .map_err(|e| CoreError::Llm(e.to_string()))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| CoreError::Parse(e.to_string()))?
    {
        if bytes.len().saturating_add(chunk.len()) > 4 * 1024 * 1024 {
            return Err(CoreError::Parse(
                "OpenRouter image catalog exceeds the size limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| CoreError::Parse(e.to_string()))?;
    let models = parse_models(&value);
    if models.is_empty() {
        return Err(CoreError::Parse(
            "OpenRouter returned no supported image models".into(),
        ));
    }
    *cache = Some((Instant::now(), models.clone()));
    Ok(models)
}

pub fn catalog_fallback() -> Vec<ImageModelPreset> {
    crate::image_provider_catalog::load_image_provider_presets()
        .unwrap_or_default()
        .into_iter()
        .find(|preset| preset.api_style == "openrouter_images")
        .map(|preset| preset.models)
        .unwrap_or_default()
}

pub fn generation_body(
    model: &ImageModelPreset,
    prompt: &str,
    size: Option<&str>,
    quality: Option<&str>,
    output_format: &str,
) -> Result<Value, CoreError> {
    let parameters = &model.metadata["supportedParameters"];
    let mut body = json!({"model":model.id,"prompt":prompt});
    let mut put = |name: &str, value: &str| -> Result<(), CoreError> {
        if value.is_empty() {
            return Ok(());
        }
        let values = parameters[name]["values"].as_array();
        if values.is_some_and(|values| values.iter().any(|v| v == value)) {
            body[name] = json!(value);
            Ok(())
        } else {
            Err(CoreError::InvalidInput(format!(
                "{} does not support {name}={value}",
                model.id
            )))
        }
    };
    if let Some(size) = size.filter(|s| !s.is_empty() && *s != "auto") {
        if let Some((ratio, resolution)) = size.split_once('|') {
            put("aspect_ratio", ratio)?;
            put("resolution", resolution)?;
        } else if size.contains(':') {
            put("aspect_ratio", size)?;
        } else {
            put("resolution", size)?;
        }
    }
    if let Some(quality) = quality.filter(|q| !q.is_empty()) {
        put("quality", quality)?;
    }
    if parameters.get("output_format").is_some() {
        put("output_format", output_format)?;
    } else if output_format != "png" {
        return Err(CoreError::InvalidInput(
            "This OpenRouter model returns PNG and does not expose an output format option".into(),
        ));
    }
    Ok(body)
}

pub fn set_parameter(
    body: &mut Value,
    model: &ImageModelPreset,
    name: &str,
    value: Value,
) -> Result<(), CoreError> {
    let descriptor = &model.metadata["supportedParameters"][name];
    let valid = match descriptor["type"].as_str() {
        Some("enum") => descriptor["values"]
            .as_array()
            .is_some_and(|values| values.contains(&value)),
        Some("range") => value.as_u64().is_some_and(|n| {
            n >= descriptor["min"].as_u64().unwrap_or(0)
                && n <= descriptor["max"].as_u64().unwrap_or(0)
        }),
        _ => false,
    };
    if !valid {
        return Err(CoreError::InvalidInput(format!(
            "{} does not support {name}={value}",
            model.id
        )));
    }
    body[name] = value;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jpeg_only_models_expose_the_format_accepted_by_the_endpoint() {
        let models = parse_models(
            &json!({"data":[{"id":"sourceful/riverflow-v2.5-fast","architecture":{"output_modalities":["image"]},"supported_parameters":{"output_format":{"type":"enum","values":["jpeg"]}}}]}),
        );
        assert_eq!(models[0].metadata["outputFormats"], json!(["jpeg"]));
        assert_eq!(
            generation_body(&models[0], "a river", None, None, "jpeg").unwrap()["output_format"],
            "jpeg"
        );
        assert!(generation_body(&models[0], "a river", None, None, "png").is_err());
        let bundled = catalog_fallback()
            .into_iter()
            .find(|m| m.id == "sourceful/riverflow-v2.5-fast")
            .unwrap();
        assert_eq!(bundled.metadata["outputFormats"], json!(["jpeg"]));
    }
    #[test]
    fn image_discovery_filters_unsupported_outputs_and_drives_request_options() {
        let mut source = json!({"id":"vendor/new-image", "name":"New image", "architecture":{"output_modalities":["image"]},"supported_parameters":{"resolution":{"type":"enum","values":["2K"]},"quality":{"type":"enum","values":["high"]}}});
        let models = parse_models(&json!({"data":[source]}));
        assert_eq!(models.len(), 1);
        assert_eq!(
            generation_body(&models[0], "a cat", Some("2K"), Some("high"), "png").unwrap(),
            json!({"model":"vendor/new-image","prompt":"a cat","resolution":"2K","quality":"high"})
        );
        assert!(generation_body(&models[0], "a cat", Some("4K"), None, "png").is_err());
        source["supported_parameters"]["output_format"] = json!({"type":"enum","values":["svg"]});
        assert!(parse_models(&json!({"data":[source]})).is_empty());
        source["supported_parameters"]["output_format"] = json!({"type":"enum","values":["png"]});
        source["supported_parameters"]["input_references"] =
            json!({"type":"range","min":1,"max":5});
        assert!(parse_models(&json!({"data":[source]})).is_empty());
    }
}
