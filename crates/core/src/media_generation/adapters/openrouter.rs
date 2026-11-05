use super::*;
use serde_json::{json, Value};

const ID: &str = "openrouter";
const BASE: &str = "https://openrouter.ai/api/v1";

pub struct OpenRouterVideoAdapter {
    client: reqwest::Client,
    base: url::Url,
    api_key: String,
    source: String,
}

impl OpenRouterVideoAdapter {
    pub fn new(
        api_key: impl Into<String>,
        credential_scope: &str,
    ) -> Result<Self, NormalizedProviderError> {
        Self::build(
            api_key.into(),
            credential_scope,
            url::Url::parse(BASE).expect("official URL"),
        )
    }

    pub(crate) fn build(
        api_key: String,
        credential_scope: &str,
        base: url::Url,
    ) -> Result<Self, NormalizedProviderError> {
        if api_key.trim().is_empty()
            || api_key.contains(['\r', '\n'])
            || credential_scope.trim().is_empty()
        {
            return Err(http::local_error(
                ID,
                "invalid_configuration",
                "OpenRouter requires an API key and account scope.",
            ));
        }
        Ok(Self {
            client: http::client()?,
            source: provider_source(ID, &base, credential_scope, "videos-v1"),
            base,
            api_key,
        })
    }

    fn job_url(
        &self,
        id: &str,
        content_index: Option<usize>,
    ) -> Result<url::Url, NormalizedProviderError> {
        if id.is_empty()
            || id.len() > 160
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(http::local_error(
                ID,
                "invalid_task_id",
                "Invalid OpenRouter video job ID.",
            ));
        }
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("hierarchical URL")
            .push("videos")
            .push(id);
        if let Some(index) = content_index {
            url.path_segments_mut()
                .expect("hierarchical URL")
                .push("content");
            url.query_pairs_mut()
                .append_pair("index", &index.to_string());
        }
        Ok(url)
    }

    fn body(request: &NormalizedVideoRequest) -> Value {
        let mut body = json!({"model":request.model_id,"prompt":request.prompt,
            "duration":request.duration_seconds,"resolution":request.resolution});
        if request.aspect_ratio != "adaptive" {
            body["aspect_ratio"] = json!(request.aspect_ratio);
        }
        if let Some(seed) = request.seed {
            body["seed"] = json!(seed);
        }
        if let Some(audio) = request.generate_audio {
            body["generate_audio"] = json!(audio);
        }
        if request.input_assets.is_empty() {
            return body;
        }
        if matches!(
            request.operation,
            MediaOperation::VideoToVideo | MediaOperation::ReferenceToVideo
        ) {
            body["input_references"] = json!(request
                .input_assets
                .iter()
                .map(|asset| {
                    match asset.role {
                        VideoInputRole::InputVideo | VideoInputRole::ReferenceVideo => {
                            json!({"type":"video_url","video_url":{"url":asset.uri}})
                        }
                        VideoInputRole::ReferenceAudio => {
                            json!({"type":"audio_url","audio_url":{"url":asset.uri}})
                        }
                        _ => json!({"type":"image_url","image_url":{"url":asset.uri}}),
                    }
                })
                .collect::<Vec<_>>());
        } else {
            body["frame_images"] = json!(request
                .input_assets
                .iter()
                .map(|asset| {
                    let frame = if asset.role == VideoInputRole::LastFrame {
                        "last_frame"
                    } else {
                        "first_frame"
                    };
                    json!({"type":"image_url","image_url":{"url":asset.uri},"frame_type":frame})
                })
                .collect::<Vec<_>>());
        }
        body
    }
}

#[async_trait]
impl VideoGenerationAdapter for OpenRouterVideoAdapter {
    fn provider_id(&self) -> &'static str {
        ID
    }
    fn provider_source(&self) -> &str {
        &self.source
    }
    fn get_capabilities(&self) -> Vec<VideoModelManifest> {
        find_capabilities(ID)
    }

    fn validate(&self, request: &NormalizedVideoRequest) -> ValidationResult {
        let mut issues = common_validation(request, false);
        let models = self.get_capabilities();
        let operation = models
            .iter()
            .find(|model| model.selectable && model.model_id == request.model_id)
            .and_then(|model| {
                model
                    .operation_capabilities
                    .iter()
                    .find(|op| op.operation == request.operation)
            });
        let Some(operation) = operation else {
            issues.push(issue(
                "operation",
                "unsupported_operation",
                "OpenRouter does not list this model and operation as supported in Nexa.",
            ));
            return ValidationResult::new(issues);
        };
        if !operation.duration_options.iter().any(|option| {
            option.resolution == request.resolution
                && option.durations_seconds.contains(&request.duration_seconds)
        }) {
            issues.push(issue(
                "durationSeconds",
                "unsupported_duration_or_resolution",
                "Choose a duration and resolution from the model's video catalog.",
            ));
        }
        if !operation.aspect_ratios.contains(&request.aspect_ratio)
            && !(operation.aspect_ratios.is_empty() && request.aspect_ratio == "adaptive")
        {
            issues.push(issue(
                "aspectRatio",
                "unsupported_aspect_ratio",
                "This OpenRouter model does not support the requested aspect ratio.",
            ));
        }
        if (request.seed.is_some() && !operation.supports_seed)
            || (request.generate_audio.is_some() && !operation.supports_audio)
            || request.callback_url.is_some()
        {
            issues.push(issue("options", "unsupported_option", "This OpenRouter route does not accept the supplied seed, audio toggle or callback."));
        }
        let roles = request
            .input_assets
            .iter()
            .map(|asset| asset.role)
            .collect::<Vec<_>>();
        let valid_roles = match request.operation {
            MediaOperation::TextToVideo => roles.is_empty(),
            MediaOperation::ImageToVideo => roles == [VideoInputRole::FirstFrame],
            MediaOperation::FirstLastFrame => {
                roles.len() == 2
                    && roles.contains(&VideoInputRole::FirstFrame)
                    && roles.contains(&VideoInputRole::LastFrame)
            }
            MediaOperation::ReferenceToVideo => {
                !roles.is_empty()
                    && roles.len() <= 10
                    && roles.iter().all(|role| {
                        matches!(
                            role,
                            VideoInputRole::ReferenceImage
                                | VideoInputRole::ReferenceVideo
                                | VideoInputRole::ReferenceAudio
                        )
                    })
            }
            MediaOperation::VideoToVideo => {
                roles.len() <= 10
                    && roles.iter().any(|role| {
                        matches!(
                            role,
                            VideoInputRole::InputVideo | VideoInputRole::ReferenceVideo
                        )
                    })
                    && roles.iter().all(|role| {
                        matches!(
                            role,
                            VideoInputRole::InputVideo
                                | VideoInputRole::ReferenceVideo
                                | VideoInputRole::ReferenceImage
                                | VideoInputRole::ReferenceAudio
                        )
                    })
            }
            _ => false,
        };
        if !valid_roles
            || request.input_assets.iter().any(|asset| {
                let kind = match asset.role {
                    VideoInputRole::InputVideo | VideoInputRole::ReferenceVideo => "video/",
                    VideoInputRole::ReferenceAudio => "audio/",
                    _ => "image/",
                };
                !valid_public_https_media_url(&asset.uri) || !asset.media_type.starts_with(kind)
            })
        {
            issues.push(issue(
                "inputAssets",
                "invalid_assets",
                "Supply the required OpenRouter input roles and public HTTPS media URLs.",
            ));
        }
        ValidationResult::new(issues)
    }

    async fn estimate_cost(
        &self,
        request: &NormalizedVideoRequest,
    ) -> Result<CostEstimate, NormalizedProviderError> {
        let validation = self.validate(request);
        if !validation.valid {
            return Err(invalid_request_error(ID, validation));
        }
        Ok(CostEstimate {
            kind: CostEstimateKind::Unavailable,
            amount_micros: None,
            currency: Some("USD".into()),
            note: "OpenRouter reports final cost in completed job usage.".into(),
        })
    }

    async fn submit(
        &self,
        request: &NormalizedVideoRequest,
    ) -> Result<SubmittedJob, NormalizedProviderError> {
        let estimated_cost = self.estimate_cost(request).await?;
        let value: Value = http::execute_json(
            ID,
            &self.api_key,
            self.client
                .post(format!("{}/videos", self.base))
                .bearer_auth(&self.api_key)
                .json(&Self::body(request)),
        )
        .await
        .map_err(submission_error)?;
        let id = value["id"].as_str().unwrap_or_default();
        self.job_url(id, None).map_err(submission_response_error)?;
        Ok(SubmittedJob {
            provider_task_id: id.to_string(),
            provider_source: self.source.clone(),
            estimated_cost,
        })
    }

    async fn get_status(
        &self,
        provider_task_id: &str,
    ) -> Result<ProviderJobStatus, NormalizedProviderError> {
        let value: Value = http::execute_json(
            ID,
            &self.api_key,
            self.client
                .get(self.job_url(provider_task_id, None)?)
                .bearer_auth(&self.api_key),
        )
        .await?;
        if value["id"] != provider_task_id {
            return Err(http::local_error(
                ID,
                "invalid_provider_response",
                "OpenRouter returned a different job ID.",
            ));
        }
        let raw = value["status"].as_str().unwrap_or("unknown");
        let state = match raw {
            "pending" => ProviderJobState::Queued,
            "in_progress" => ProviderJobState::Running,
            "completed" => ProviderJobState::Succeeded,
            "failed" | "expired" => ProviderJobState::Failed,
            "cancelled" => ProviderJobState::Cancelled,
            _ => ProviderJobState::ProviderUnknown,
        };
        let result = if state == ProviderJobState::Succeeded {
            let count = value["unsigned_urls"]
                .as_array()
                .or_else(|| value["urls"].as_array())
                .map_or(1, Vec::len)
                .clamp(1, 16);
            let outputs = (0..count)
                .map(|index| {
                    Ok(ProviderOutputLocator {
                        uri: self.job_url(provider_task_id, Some(index))?.to_string(),
                        media_type: "video/mp4".into(),
                        expires_hint: None,
                    })
                })
                .collect::<Result<Vec<_>, NormalizedProviderError>>()?;
            Some(ProviderJobResult {
                provider_task_id: provider_task_id.into(),
                outputs,
                width: None,
                height: None,
                duration_ms: None,
            })
        } else {
            None
        };
        let error = (state == ProviderJobState::Failed).then(|| {
            let detail = value
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| value.pointer("/error/message").and_then(Value::as_str))
                .unwrap_or("OpenRouter video generation did not complete.");
            http::local_error(
                ID,
                if raw == "expired" {
                    "expired"
                } else {
                    "generation_failed"
                },
                &http::sanitize_message(detail, Some(&self.api_key)),
            )
        });
        let final_cost_micros = value
            .pointer("/usage/cost")
            .and_then(Value::as_f64)
            .filter(|cost| {
                cost.is_finite() && *cost >= 0.0 && *cost < u64::MAX as f64 / 1_000_000.0
            })
            .map(|cost| (cost * 1_000_000.0).round() as u64);
        Ok(ProviderJobStatus {
            provider_task_id: provider_task_id.into(),
            state,
            raw_status: raw.into(),
            result,
            error,
            billed_usage: None,
            final_cost_micros,
        })
    }

    async fn cancel(
        &self,
        request: &ProviderCancellationRequest,
    ) -> Result<CancellationResult, NormalizedProviderError> {
        self.job_url(&request.provider_task_id, None)?;
        Ok(CancellationResult {
            provider_task_id: request.provider_task_id.clone(),
            confirmed: false,
            detail: "OpenRouter does not expose a video cancellation endpoint.".into(),
        })
    }

    async fn download_outputs(
        &self,
        result: &ProviderJobResult,
        destination: &Path,
        max_total_bytes: u64,
    ) -> Result<Vec<DownloadedAsset>, NormalizedProviderError> {
        for (index, output) in result.outputs.iter().enumerate() {
            if output.uri
                != self
                    .job_url(&result.provider_task_id, Some(index))?
                    .as_str()
            {
                return Err(http::local_error(ID, "invalid_output_url", "OpenRouter video outputs must use the official authenticated job content path."));
            }
        }
        http::download_outputs_with_auth(
            ID,
            &self.client,
            result,
            destination,
            max_total_bytes,
            cfg!(test) && self.base.scheme() == "http",
            Some(&self.api_key),
        )
        .await
    }

    fn normalize_error(
        &self,
        error: &(dyn std::error::Error + Send + Sync),
    ) -> NormalizedProviderError {
        http::local_error(
            ID,
            "provider_error",
            &http::sanitize_message(&error.to_string(), Some(&self.api_key)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unverified_routes_and_projects_documented_first_frame() {
        let adapter = OpenRouterVideoAdapter::new("test-key", "scope").unwrap();
        let mut request = NormalizedVideoRequest {
            idempotency_key: "attempt-1".into(),
            model_id: "tbc/mischief-1".into(),
            operation: MediaOperation::ImageToVideo,
            prompt: "A moving cloud".into(),
            duration_seconds: 5,
            resolution: "720p".into(),
            aspect_ratio: "16:9".into(),
            input_assets: vec![VideoInputAsset {
                role: VideoInputRole::FirstFrame,
                uri: "https://cdn.example/image.png".into(),
                media_type: "image/png".into(),
                metadata_verified: true,
                byte_length: Some(1200),
                width: Some(1280),
                height: Some(720),
                duration_ms: None,
                frame_rate: None,
                video_codec: None,
            }],
            seed: Some(123),
            generate_audio: None,
            callback_url: None,
        };
        assert!(adapter.validate(&request).valid);
        let body = OpenRouterVideoAdapter::body(&request);
        assert_eq!(body["frame_images"][0]["frame_type"], "first_frame");
        assert_eq!(body["seed"], 123);
        assert_eq!(body["resolution"], "720p");
        assert!(body.get("input_references").is_none());
        request.operation = MediaOperation::TextToVideo;
        request.input_assets.clear();
        assert!(!adapter.validate(&request).valid);
        assert!(adapter.job_url("../../token", Some(0)).is_err());
        assert_eq!(
            adapter.job_url("gen-vid-valid", Some(0)).unwrap().as_str(),
            "https://openrouter.ai/api/v1/videos/gen-vid-valid/content?index=0"
        );
    }

    #[tokio::test]
    async fn uses_bearer_only_on_trusted_job_paths() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (path, authorization, status, headers, body) in [
                ("/api/v1/videos/test-job", true, "200 OK", "Content-Type: application/json\r\n",
                    br#"{"id":"test-job","status":"completed","unsigned_urls":["https://untrusted.example/malicious"],"usage":{"cost":0.125}}"#.as_slice()),
                ("/api/v1/videos/test-job/content?index=0", true, "302 Found", "Location: /final.mp4\r\n", b"".as_slice()),
                ("/final.mp4", false, "200 OK", "Content-Type: video/mp4\r\n", b"\0\0\0\x18ftypmp42\0\0\0\0".as_slice()),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                }
                let request = String::from_utf8(request).unwrap().to_lowercase();
                assert!(request.starts_with(&format!("get {path} ")));
                assert_eq!(request.contains("authorization: bearer test-key"), authorization);
                stream.write_all(format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                stream.write_all(body).await.unwrap();
            }
        });
        let adapter = OpenRouterVideoAdapter::build(
            "test-key".into(),
            "scope",
            url::Url::parse(&base).unwrap(),
        )
        .unwrap();
        let status = adapter.get_status("test-job").await.unwrap();
        assert_eq!(status.state, ProviderJobState::Succeeded);
        assert_eq!(status.final_cost_micros, Some(125_000));
        let mut result = status.result.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let files = adapter
            .download_outputs(&result, dir.path(), 1000)
            .await
            .unwrap();
        assert_eq!(files[0].byte_length, 16);
        result.outputs[0].uri = "https://untrusted.example/asset".into();
        assert!(adapter
            .download_outputs(&result, dir.path(), 1000)
            .await
            .is_err());
        server.await.unwrap();
    }
}
