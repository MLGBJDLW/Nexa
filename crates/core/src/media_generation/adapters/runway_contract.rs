//! The shared catalog owns Runway's operation and wire contracts together.
use super::*;
use crate::video_provider_catalog::VideoOperationCapability;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

#[cfg(test)]
mod tests {
    use super::*;
    fn conforms(value: &Value, schema: &Value) -> bool {
        for key in ["anyOf", "oneOf"] {
            if let Some(options) = schema[key].as_array() {
                return options.iter().any(|option| conforms(value, option));
            }
        }
        if let Some(values) = schema["enum"].as_array() {
            if !values.contains(value) {
                return false;
            }
        }
        if schema
            .get("const")
            .is_some_and(|expected| expected != value)
        {
            return false;
        }
        match schema["type"].as_str() {
            Some("string") => value.as_str().is_some_and(|value| {
                let n = value.encode_utf16().count() as u64;
                schema["minLength"].as_u64().is_none_or(|min| n >= min)
                    && schema["maxLength"].as_u64().is_none_or(|max| n <= max)
            }),
            Some("integer") => value.as_i64().is_some_and(|n| {
                schema["minimum"].as_i64().is_none_or(|min| n >= min)
                    && schema["maximum"].as_i64().is_none_or(|max| n <= max)
            }),
            Some("boolean") => value.is_boolean(),
            Some("array") => value.as_array().is_some_and(|items| {
                let n = items.len() as u64;
                schema["minItems"].as_u64().is_none_or(|min| n >= min)
                    && schema["maxItems"].as_u64().is_none_or(|max| n <= max)
                    && items.iter().all(|item| conforms(item, &schema["items"]))
            }),
            Some("object") => value.as_object().is_some_and(|object| {
                schema["required"].as_array().is_none_or(|keys| {
                    keys.iter()
                        .all(|key| object.contains_key(key.as_str().unwrap()))
                }) && object.iter().all(|(key, value)| {
                    schema["properties"]
                        .get(key)
                        .is_none_or(|schema| conforms(value, schema))
                })
            }),
            _ => true,
        }
    }
    fn input(role: VideoInputRole, duration: u32) -> VideoInputAsset {
        let video = role == VideoInputRole::InputVideo;
        VideoInputAsset {
            role,
            uri: "runway://fixture-media".into(),
            media_type: if video { "video/mp4" } else { "image/jpeg" }.into(),
            metadata_verified: true,
            byte_length: Some(1000),
            width: Some(1280),
            height: Some(720),
            duration_ms: video.then_some(u64::from(duration) * 1000),
            frame_rate: video.then_some(24.0),
            video_codec: video.then(|| "h264".into()),
        }
    }
    #[test]
    fn every_enabled_runway_operation_matches_the_publisher_schema() {
        let snapshot: Value =
            serde_json::from_str(include_str!("fixtures/runway-video-contracts.json")).unwrap();
        let adapter = RunwayVideoAdapter::for_test("http://127.0.0.1");
        for model in find_capabilities("runway") {
            for capability in model.operation_capabilities {
                for option in &capability.duration_options {
                    let duration = option
                        .durations_seconds
                        .first()
                        .copied()
                        .or(option.min_duration_seconds)
                        .unwrap();
                    let mut request = NormalizedVideoRequest {
                        idempotency_key: "fixture".into(),
                        model_id: model.model_id.clone(),
                        operation: capability.operation,
                        prompt: "A cinematic scene.".into(),
                        duration_seconds: duration,
                        resolution: option.resolution.clone(),
                        aspect_ratio: String::new(),
                        input_assets: vec![],
                        seed: capability.supports_seed.then_some(42),
                        generate_audio: capability.supports_audio.then_some(true),
                        callback_url: None,
                    };
                    let wire = &operation(&request).unwrap().wire_contract;
                    request.aspect_ratio = wire.ratios[&request.resolution]
                        .keys()
                        .next()
                        .unwrap()
                        .clone();
                    request.input_assets = match request.operation {
                        MediaOperation::ImageToVideo => {
                            vec![input(VideoInputRole::FirstFrame, duration)]
                        }
                        MediaOperation::FirstLastFrame => vec![
                            input(VideoInputRole::FirstFrame, duration),
                            input(VideoInputRole::LastFrame, duration),
                        ],
                        MediaOperation::VideoToVideo => {
                            vec![input(VideoInputRole::InputVideo, duration)]
                        }
                        _ => vec![],
                    };
                    let validated = adapter.validate(&request);
                    assert!(
                        validated.valid,
                        "{} {:?} {}: {:?}",
                        request.model_id, request.operation, request.resolution, validated.issues
                    );
                    let body = payload(&request).unwrap();
                    let path = RunwayVideoAdapter::submit_path(request.operation).unwrap();
                    let contract = snapshot["contracts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|c| c["model"] == request.model_id && c["path"] == path)
                        .unwrap();
                    for required in contract["required"].as_array().unwrap() {
                        assert!(
                            body.get(required.as_str().unwrap()).is_some(),
                            "{} {path} missing {required}",
                            request.model_id
                        );
                    }
                    for (key, value) in body.as_object().unwrap() {
                        if key == "model" {
                            assert_eq!(*value, contract["model"]);
                            continue;
                        }
                        let schema = contract["fields"].get(key).unwrap_or_else(|| {
                            panic!("{} {path}: unsupported field {key}", request.model_id)
                        });
                        assert!(
                            conforms(value, schema),
                            "{} {path}: {key}={value} violates publisher schema",
                            request.model_id
                        );
                    }
                    let required_prompt = contract["required"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|field| field == "promptText");
                    for prompt in ["", "x", "ab"] {
                        request.prompt = prompt.into();
                        let expected = if prompt.is_empty() {
                            !required_prompt
                        } else {
                            conforms(&json!(prompt), &contract["fields"]["promptText"])
                        };
                        assert_eq!(
                            adapter.validate(&request).valid,
                            expected,
                            "{} {path}: optional/required prompt {prompt:?}",
                            request.model_id
                        );
                    }
                    request.aspect_ratio = "unsupported".into();
                    assert!(!adapter.validate(&request).valid);
                }
            }
        }
    }
    #[test]
    fn video_edits_omit_unpublished_fields_and_wrong_optional_controls_are_rejected() {
        let mut request:NormalizedVideoRequest=serde_json::from_value(json!({
            "idempotencyKey":"fixture","modelId":"klingO3_pro","operation":"video_to_video","prompt":"Edit the scene",
            "durationSeconds":5,"resolution":"SOURCE","aspectRatio":"adaptive"
        })).unwrap();
        request
            .input_assets
            .push(input(VideoInputRole::InputVideo, 5));
        let body = payload(&request).unwrap();
        assert!(body.get("promptVideo").is_some());
        for key in ["ratio", "duration", "mode", "audio", "seed"] {
            assert!(body.get(key).is_none());
        }
        let adapter = RunwayVideoAdapter::for_test("http://127.0.0.1");
        request.seed = Some(42);
        assert!(!adapter.validate(&request).valid);
        request.seed = None;
        request.generate_audio = Some(false);
        assert!(!adapter.validate(&request).valid);
        request.generate_audio = None;
        request.duration_seconds = 4;
        assert!(!adapter.validate(&request).valid);
        request.model_id = "aleph2".into();
        request.duration_seconds = 5;
        let body = payload(&request).unwrap();
        assert!(body.get("videoUri").is_some() && body.get("promptVideo").is_none());
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogModel {
    model_id: String,
    operation_capabilities: Vec<Operation>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Operation {
    #[serde(flatten)]
    pub capability: VideoOperationCapability,
    pub wire_contract: WireContract,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireContract {
    pub ratios: BTreeMap<String, BTreeMap<String, String>>,
    pub ratio_field: Option<String>,
    pub resolution_field: Option<String>,
    pub duration_field: bool,
    pub video_field: Option<String>,
    pub video_mode: Option<String>,
    pub prompt_min: usize,
    pub prompt_max: usize,
    pub prompt_required: bool,
    pub max_reference_images: usize,
    pub max_reference_videos: usize,
    pub max_reference_audio: usize,
    pub max_total_video_ms: Option<u64>,
    pub max_total_audio_ms: Option<u64>,
    pub min_video_edge: Option<u32>,
    pub max_video_edge: Option<u32>,
    pub min_image_edge: Option<u32>,
}

pub(super) fn operation(request: &NormalizedVideoRequest) -> Option<&'static Operation> {
    static MODELS: OnceLock<Vec<CatalogModel>> = OnceLock::new();
    MODELS
        .get_or_init(|| {
            let catalog: Value = serde_json::from_str(include_str!(
                "../../../../../shared/video-provider-presets.json"
            ))
            .expect("video catalog");
            let models = catalog
                .as_array()
                .unwrap()
                .iter()
                .find(|provider| provider["providerId"] == "runway")
                .unwrap()["models"]
                .clone();
            serde_json::from_value(models).expect("Runway operation contracts")
        })
        .iter()
        .find(|model| model.model_id == request.model_id)?
        .operation_capabilities
        .iter()
        .find(|operation| operation.capability.operation == request.operation)
}

pub(super) fn payload(request: &NormalizedVideoRequest) -> Result<Value, NormalizedProviderError> {
    let operation = operation(request)
        .ok_or_else(|| configuration_error("Runway model does not support this operation"))?;
    let wire = &operation.wire_contract;
    let ratio = wire
        .ratios
        .get(&request.resolution)
        .and_then(|ratios| ratios.get(&request.aspect_ratio))
        .ok_or_else(|| {
            configuration_error("Resolution and aspect ratio do not map to this Runway model")
        })?;
    let mut payload = Map::new();
    payload.insert("model".into(), json!(request.model_id));
    if !request.prompt.is_empty() {
        payload.insert("promptText".into(), json!(request.prompt));
    }
    if let Some(field) = &wire.ratio_field {
        payload.insert(field.clone(), json!(ratio));
    }
    if let Some(field) = &wire.resolution_field {
        payload.insert(
            field.clone(),
            json!(request.resolution.to_ascii_lowercase()),
        );
    }
    if wire.duration_field {
        payload.insert("duration".into(), json!(request.duration_seconds));
    }
    if let Some(seed) = request.seed {
        payload.insert("seed".into(), json!(seed));
    }
    if let Some(audio) = request.generate_audio {
        payload.insert("audio".into(), json!(audio));
    }
    match request.operation {
        MediaOperation::ImageToVideo => {
            let first = find_role(request, VideoInputRole::FirstFrame)
                .ok_or_else(|| configuration_error("Missing first frame"))?;
            payload.insert("promptImage".into(), json!(first.uri));
        }
        MediaOperation::FirstLastFrame => {
            let first = find_role(request, VideoInputRole::FirstFrame)
                .ok_or_else(|| configuration_error("Missing first frame"))?;
            let last = find_role(request, VideoInputRole::LastFrame)
                .ok_or_else(|| configuration_error("Missing last frame"))?;
            payload.insert(
                "promptImage".into(),
                json!([{"uri":first.uri,"position":"first"},{"uri":last.uri,"position":"last"}]),
            );
        }
        MediaOperation::VideoToVideo => {
            let input = find_role(request, VideoInputRole::InputVideo)
                .ok_or_else(|| configuration_error("Missing input video"))?;
            let field = wire
                .video_field
                .as_ref()
                .ok_or_else(|| configuration_error("Missing video wire contract"))?;
            payload.insert(field.clone(), json!(input.uri));
            if let Some(mode) = &wire.video_mode {
                payload.insert("mode".into(), json!(mode));
            }
        }
        _ => {}
    }
    insert_references(&mut payload, request);
    Ok(Value::Object(payload))
}

pub(super) fn validate(
    request: &NormalizedVideoRequest,
    issues: &mut Vec<super::super::ValidationIssue>,
) {
    let Some(operation) = operation(request) else {
        issues.push(issue(
            "modelId",
            "unsupported_operation",
            "Runway model does not support this operation",
        ));
        return;
    };
    let capability = &operation.capability;
    let wire = &operation.wire_contract;
    // Replace the generic prompt heuristic with the publisher's exact UTF-16
    // contract, including image-only inputs and WAN's longer prompt field.
    issues.retain(|issue| issue.code != "invalid_prompt");
    let length = request.prompt.encode_utf16().count();
    if (length == 0 && wire.prompt_required)
        || (length > 0 && length < wire.prompt_min)
        || length > wire.prompt_max
    {
        issues.push(issue(
            "prompt",
            "unsupported_prompt_length",
            &format!(
                "This operation permits {}–{} UTF-16 code units",
                wire.prompt_min, wire.prompt_max
            ),
        ));
    }
    if !wire
        .ratios
        .get(&request.resolution)
        .is_some_and(|ratios| ratios.contains_key(&request.aspect_ratio))
    {
        issues.push(issue(
            "aspectRatio",
            "unsupported_aspect_ratio",
            "Resolution and aspect ratio are not supported together for this operation",
        ));
    }
    let valid_duration = capability.duration_options.iter().any(|option| {
        option.resolution == request.resolution
            && option
                .min_duration_seconds
                .is_none_or(|minimum| request.duration_seconds >= minimum)
            && option
                .max_duration_seconds
                .is_none_or(|maximum| request.duration_seconds <= maximum)
            && (option.durations_seconds.is_empty()
                || option.durations_seconds.contains(&request.duration_seconds))
    });
    if !valid_duration {
        issues.push(issue(
            "durationSeconds",
            "unsupported_duration",
            "Duration is not supported at this resolution",
        ));
    }
    if request.generate_audio.is_some() && !capability.supports_audio {
        issues.push(issue(
            "generateAudio",
            "unsupported_audio",
            "This operation has no audio toggle",
        ));
    }
    if request.seed.is_some() && !capability.supports_seed {
        issues.push(issue(
            "seed",
            "unsupported_seed",
            "This operation has no seed parameter",
        ));
    }
    if request.callback_url.is_some() {
        issues.push(issue(
            "callbackUrl",
            "unsupported_webhook",
            "Runway task API does not publish a webhook parameter",
        ));
    }
    let first = count_role(request, VideoInputRole::FirstFrame);
    let last = count_role(request, VideoInputRole::LastFrame);
    let videos = count_role(request, VideoInputRole::InputVideo);
    let primary_valid = match request.operation {
        MediaOperation::TextToVideo => first == 0 && last == 0 && videos == 0,
        MediaOperation::ImageToVideo => first == 1 && last == 0 && videos == 0,
        MediaOperation::FirstLastFrame => first == 1 && last == 1 && videos == 0,
        MediaOperation::VideoToVideo => first == 0 && last == 0 && videos == 1,
        _ => false,
    };
    if !primary_valid {
        issues.push(issue(
            "inputAssets",
            "invalid_primary_input",
            "The operation requires exactly its declared primary image/frame/video inputs",
        ));
    }
    for asset in &request.input_assets {
        let role = match asset.role {
            VideoInputRole::FirstFrame => "first_frame",
            VideoInputRole::LastFrame => "last_frame",
            VideoInputRole::InputVideo => "input_video",
            VideoInputRole::ReferenceImage => "reference_image",
            VideoInputRole::ReferenceVideo => "reference_video",
            VideoInputRole::ReferenceAudio => "reference_audio",
        };
        if !capability.input_roles.iter().any(|allowed| allowed == role) {
            issues.push(issue(
                "inputAssets",
                "unsupported_input_role",
                "This operation does not accept the supplied media role",
            ));
        }
        if matches!(
            asset.role,
            VideoInputRole::InputVideo | VideoInputRole::ReferenceVideo
        ) {
            if asset.width.zip(asset.height).is_some_and(|(w, h)| {
                wire.min_video_edge
                    .is_some_and(|minimum| w.min(h) < minimum)
                    || wire
                        .max_video_edge
                        .is_some_and(|maximum| w.max(h) > maximum)
            }) {
                issues.push(issue(
                    "inputAssets",
                    "unsupported_video_dimensions",
                    "Input video dimensions fall outside the model contract",
                ));
            }
        } else if asset.width.zip(asset.height).is_some_and(|(w, h)| {
            wire.min_image_edge
                .is_some_and(|minimum| w.min(h) < minimum)
        }) {
            issues.push(issue(
                "inputAssets",
                "unsupported_image_dimensions",
                "Input image dimensions fall outside the model contract",
            ));
        }
    }
    if count_role(request, VideoInputRole::ReferenceImage) > wire.max_reference_images
        || count_role(request, VideoInputRole::ReferenceVideo) > wire.max_reference_videos
        || count_role(request, VideoInputRole::ReferenceAudio) > wire.max_reference_audio
    {
        issues.push(issue(
            "inputAssets",
            "too_many_references",
            "Reference counts exceed the operation contract",
        ));
    }
    let video_ms = request
        .input_assets
        .iter()
        .filter(|asset| {
            matches!(
                asset.role,
                VideoInputRole::InputVideo | VideoInputRole::ReferenceVideo
            )
        })
        .filter_map(|asset| asset.duration_ms)
        .fold(0u64, u64::saturating_add);
    let audio_ms = request
        .input_assets
        .iter()
        .filter(|asset| asset.role == VideoInputRole::ReferenceAudio)
        .filter_map(|asset| asset.duration_ms)
        .fold(0u64, u64::saturating_add);
    if wire
        .max_total_video_ms
        .is_some_and(|maximum| video_ms > maximum)
    {
        issues.push(issue(
            "inputAssets",
            "video_duration_exceeded",
            "Combined input and reference video duration exceeds the model contract",
        ));
    }
    if wire
        .max_total_audio_ms
        .is_some_and(|maximum| audio_ms > maximum)
    {
        issues.push(issue(
            "inputAssets",
            "audio_duration_exceeded",
            "Combined reference audio duration exceeds the model contract",
        ));
    }
    if !wire.duration_field
        && find_role(request, VideoInputRole::InputVideo)
            .and_then(|asset| asset.duration_ms)
            .is_some_and(|duration| duration.div_ceil(1000) != u64::from(request.duration_seconds))
    {
        issues.push(issue(
            "durationSeconds",
            "source_duration_required",
            "This edit inherits the input video duration",
        ));
    }
}
