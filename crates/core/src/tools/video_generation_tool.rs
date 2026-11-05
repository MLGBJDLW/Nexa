use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::CoreError;
use crate::media_generation::adapters::{
    NormalizedVideoRequest, OpenRouterVideoAdapter, ProviderJobState, ProviderJobStatus,
    VideoGenerationAdapter, VideoInputAsset,
};
use crate::media_generation::*;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

const PROVIDER: &str = "openrouter";
const BASE_URL: &str = "https://openrouter.ai/api/v1";
const MAX_OUTPUT_BYTES: u64 = 512 * 1024 * 1024;

pub struct GenerateVideoTool(pub MediaGenerationRuntime);

#[derive(Deserialize)]
struct Arguments {
    action: String,
    provider_config_id: Option<String>,
    job_id: Option<String>,
    request: Option<VideoRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoRequest {
    model_id: String,
    operation: MediaOperation,
    prompt: String,
    duration_seconds: u32,
    resolution: String,
    aspect_ratio: String,
    #[serde(default)]
    input_assets: Vec<VideoInputAsset>,
    seed: Option<u32>,
    generate_audio: Option<bool>,
}

#[async_trait]
impl Tool for GenerateVideoTool {
    fn name(&self) -> &str {
        "generate_video"
    }

    fn description(&self) -> &str {
        "Generate an OpenRouter video in a durable job. Use models for supported operations and settings, submit once after approval, and status with the returned local job ID to retrieve the finished video. A pending job can be checked again later."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {"type": "string", "enum": ["models", "submit", "status"]},
                "provider_config_id": {"type": "string", "description": "Saved official OpenRouter model connection ID. Reuse the same connection when checking status. Optional when exactly one OpenRouter connection exists."},
                "job_id": {"type": "string", "description": "Nexa job ID returned by submit."},
                "request": {
                    "type": "object",
                    "required": ["modelId", "operation", "prompt", "durationSeconds", "resolution", "aspectRatio"],
                    "properties": {
                        "modelId": {"type": "string"},
                        "operation": {"type": "string"},
                        "prompt": {"type": "string"},
                        "durationSeconds": {"type": "integer"},
                        "resolution": {"type": "string"},
                        "aspectRatio": {"type": "string"},
                        "seed": {"type": "integer"},
                        "generateAudio": {"type": "boolean"},
                        "inputAssets": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["role", "uri", "mediaType"],
                                "properties": {
                                    "role": {"type": "string", "enum": ["first_frame", "last_frame", "input_video", "reference_image", "reference_video", "reference_audio"]},
                                    "uri": {"type": "string", "description": "Public HTTPS media URL. Local files and data URLs are unsupported."},
                                    "mediaType": {"type": "string"}
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    fn requires_confirmation(&self, args: &Value) -> bool {
        args["action"] == "submit"
    }

    fn confirmation_message(&self, args: &Value) -> Option<String> {
        self.requires_confirmation(args).then(|| {
            "Send the video prompt and any supplied media URLs to OpenRouter. Generation can incur provider charges.".into()
        })
    }

    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let args: Arguments = serde_json::from_str(context.arguments)
            .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        let data = match args.action.as_str() {
            "models" => {
                let models = crate::video_provider_catalog::load_video_provider_presets()
                    .map_err(|error| CoreError::Internal(error.to_string()))?
                    .into_iter()
                    .filter(|preset| preset.provider_id == PROVIDER)
                    .flat_map(|preset| preset.models)
                    .filter(|model| model.selectable)
                    .map(|model| json!({"modelId": model.model_id, "operations": model.operation_capabilities}))
                    .collect::<Vec<_>>();
                json!({"models": models})
            }
            "submit" | "status" => {
                let adapter = resolve_adapter(context.db, args.provider_config_id.as_deref())?;
                let conversation_id = context.conversation_id.ok_or_else(|| {
                    CoreError::InvalidInput(
                        "A chat conversation is required for video jobs.".into(),
                    )
                })?;
                if args.action == "submit" {
                    let request = args.request.ok_or_else(|| missing("request"))?;
                    self.submit(&adapter, &context, conversation_id, request)
                        .await?
                } else {
                    let id = args.job_id.as_deref().ok_or_else(|| missing("job_id"))?;
                    self.status(&adapter, conversation_id, id).await?
                }
            }
            _ => return Err(CoreError::InvalidInput("Unknown video action.".into())),
        };
        Ok(ToolResult {
            call_id: context.call_id.into(),
            content: data.to_string(),
            is_error: false,
            artifacts: None,
        })
    }
}

impl GenerateVideoTool {
    async fn submit(
        &self,
        adapter: &OpenRouterVideoAdapter,
        context: &ToolExecutionContext<'_>,
        conversation_id: &str,
        input: VideoRequest,
    ) -> Result<Value, CoreError> {
        let turn_id = context
            .turn_id
            .ok_or_else(|| missing("trusted turn identity"))?;
        let execution_id = context
            .execution_id
            .ok_or_else(|| missing("trusted execution identity"))?;
        let identity = json!([conversation_id, turn_id, execution_id]).to_string();
        let cancellation = context.cancel_token.cloned().unwrap_or_default();
        let privacy = context.db.privacy_lease(&cancellation)?;
        let request = NormalizedVideoRequest {
            idempotency_key: format!(
                "generate_video:{}",
                blake3::hash(identity.as_bytes()).to_hex()
            ),
            model_id: input.model_id,
            operation: input.operation,
            prompt: privacy.policy.redact_text(&input.prompt),
            duration_seconds: input.duration_seconds,
            resolution: input.resolution,
            aspect_ratio: input.aspect_ratio,
            input_assets: input.input_assets,
            seed: input.seed,
            generate_audio: input.generate_audio,
            callback_url: None,
        };
        let cost = adapter
            .estimate_cost(&request)
            .await
            .map_err(provider_error)?;
        // Persist reference identity without storing possibly signed media URLs.
        let mut parameters = serde_json::to_value(&request)
            .map_err(|error| CoreError::Internal(error.to_string()))?;
        parameters["inputAssets"] = json!(request
            .input_assets
            .iter()
            .map(|asset| {
                json!({
                    "role": asset.role,
                    "mediaType": asset.media_type,
                    "urlHash": blake3::hash(asset.uri.as_bytes()).to_hex().to_string()
                })
            })
            .collect::<Vec<_>>());
        let mut job = self
            .0
            .create_job(CreateMediaJobRequest {
                idempotency_key: request.idempotency_key.clone(),
                project_id: None,
                conversation_id: Some(conversation_id.into()),
                provider_id: PROVIDER.into(),
                provider_source: adapter.provider_source().into(),
                model_id: request.model_id.clone(),
                api_version: None,
                operation: request.operation,
                input_asset_ids: Vec::new(),
                raw_parameters: parameters.clone(),
                normalized_parameters: parameters,
                provider_extras: json!({}),
                observation_mode: MediaObservationMode::Polling,
                estimated_cost_micros: None,
                currency: cost.currency,
                data_region: None,
                remote_retention_expires_at: None,
                allow_cross_provider_fallback: false,
                max_attempts: 1,
            })
            .await?;
        for (current, next) in [
            (MediaJobState::Draft, MediaJobState::Validating),
            (MediaJobState::Validating, MediaJobState::UploadingAssets),
            (MediaJobState::UploadingAssets, MediaJobState::Submitting),
        ] {
            if job.job.state == current {
                job = self.transition(&job, next).await?;
            }
        }
        if job.job.state != MediaJobState::Submitting || job.job.current_attempt_id.is_some() {
            return self.summary(&job);
        }
        privacy.ensure_current()?;
        if cancellation.is_cancelled() {
            return Err(CoreError::Cancelled("Video submission cancelled.".into()));
        }
        job = self
            .0
            .begin_attempt(BeginMediaJobAttemptRequest {
                job_id: job.job.id.clone(),
                expected_revision: job.job.revision,
                idempotency_key: uuid::Uuid::new_v4().to_string(),
                provider_id: PROVIDER.into(),
                provider_source: adapter.provider_source().into(),
                model_id: request.model_id.clone(),
                api_version: None,
                data_region: None,
                remote_retention_expires_at: None,
                provider_unknown_reconciliation: None,
            })
            .await?;

        let submitted = tokio::select! {
            biased;
            _ = cancellation.cancelled() => None,
            result = adapter.submit(&request) => Some(result),
        };
        let (state, attempt, task_id, error) = match submitted {
            Some(Ok(submitted)) => (
                MediaJobState::Queued,
                MediaJobAttemptState::Accepted,
                Some(submitted.provider_task_id),
                None,
            ),
            Some(Err(error)) if error.code != "submission_outcome_unknown" => (
                MediaJobState::Failed,
                MediaJobAttemptState::Failed,
                None,
                Some(json!(error)),
            ),
            error => (
                MediaJobState::ProviderUnknown,
                MediaJobAttemptState::ProviderUnknown,
                None,
                Some(match error {
                    Some(Err(error)) => json!(error),
                    _ => {
                        json!({"code":"submission_interrupted", "message":"The provider may have accepted the request. Do not resubmit automatically."})
                    }
                }),
            ),
        };
        let job = self
            .observe(
                &job,
                "submission",
                state,
                attempt,
                task_id,
                error.clone(),
                None,
            )
            .await?;
        let mut summary = self.summary(&job)?;
        summary["error"] = json!(error);
        Ok(summary)
    }

    async fn status(
        &self,
        adapter: &OpenRouterVideoAdapter,
        conversation_id: &str,
        job_id: &str,
    ) -> Result<Value, CoreError> {
        let mut job = self.0.get_job(job_id).await?;
        if job.job.conversation_id.as_deref() != Some(conversation_id)
            || job.job.provider_source != adapter.provider_source()
        {
            return Err(CoreError::InvalidInput(
                "The video job belongs to another conversation or OpenRouter connection.".into(),
            ));
        }
        if job.job.state.is_terminal() {
            return self.summary(&job);
        }
        let Some(task_id) = job.job.current_provider_task_id.clone() else {
            return self.summary(&job);
        };
        let status = adapter.get_status(&task_id).await.map_err(provider_error)?;
        if matches!(
            status.state,
            ProviderJobState::Running | ProviderJobState::Succeeded
        ) && matches!(
            job.job.state,
            MediaJobState::Queued | MediaJobState::ProviderUnknown
        ) {
            job = self
                .observe(
                    &job,
                    "running",
                    MediaJobState::Running,
                    MediaJobAttemptState::Observing,
                    None,
                    None,
                    None,
                )
                .await?;
        }
        if let Some(result) = &status.result {
            if job.job.state != MediaJobState::PostProcessing {
                job = self
                    .observe(
                        &job,
                        "succeeded",
                        MediaJobState::PostProcessing,
                        MediaJobAttemptState::Succeeded,
                        None,
                        None,
                        status.final_cost_micros,
                    )
                    .await?;
            }
            let staging =
                tempfile::tempdir().map_err(|error| CoreError::Internal(error.to_string()))?;
            let outputs = adapter
                .download_outputs(result, staging.path(), MAX_OUTPUT_BYTES)
                .await
                .map_err(provider_error)?;
            for (ordinal, output) in outputs.into_iter().enumerate() {
                let asset = self
                    .0
                    .import_asset(ImportMediaAssetRequest {
                        source_path: output.path,
                        declared_media_type: output.detected_media_type,
                        expected_sha256: Some(output.sha256),
                        expected_byte_length: Some(output.byte_length),
                        width: result.width,
                        height: result.height,
                        duration_ms: result.duration_ms,
                    })
                    .await?;
                job = self
                    .0
                    .link_asset(LinkMediaAssetRequest {
                        job_id: job.job.id.clone(),
                        expected_revision: job.job.revision,
                        idempotency_key: format!("output:{task_id}:{ordinal}"),
                        attempt_id: attempt_id(&job)?,
                        asset_id: asset.id,
                        parent_asset_id: None,
                        relation_type: MediaAssetRelationType::Output,
                        ordinal: ordinal as u32,
                        local_retention_policy: MediaAssetLocalRetentionPolicy::RetainUntilDeleted,
                        local_retention_expires_at: None,
                        metadata: json!({}),
                    })
                    .await?;
            }
            job = self.transition(&job, MediaJobState::Completed).await?;
        } else if let Some((next, attempt)) = terminal_failure(&status, job.job.state) {
            job = self
                .observe(
                    &job,
                    "failed",
                    next,
                    attempt,
                    None,
                    status.error.as_ref().map(|error| json!(error)),
                    status.final_cost_micros,
                )
                .await?;
        }
        let mut summary = self.summary(&job)?;
        summary["providerStatus"] = json!(status.raw_status);
        summary["error"] = json!(status.error);
        Ok(summary)
    }

    async fn transition(
        &self,
        job: &MediaJobSnapshot,
        state: MediaJobState,
    ) -> Result<MediaJobSnapshot, CoreError> {
        self.0
            .transition_job(TransitionMediaJobRequest {
                job_id: job.job.id.clone(),
                expected_revision: job.job.revision,
                next_state: state,
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn observe(
        &self,
        job: &MediaJobSnapshot,
        event: &str,
        state: MediaJobState,
        attempt: MediaJobAttemptState,
        task_id: Option<String>,
        error: Option<Value>,
        cost: Option<u64>,
    ) -> Result<MediaJobSnapshot, CoreError> {
        let attempt_id = attempt_id(job)?;
        self.0
            .record_provider_event(RecordMediaProviderEventRequest {
                job_id: job.job.id.clone(),
                expected_revision: job.job.revision,
                deduplication_key: format!("{attempt_id}:{event}"),
                attempt_id,
                provider_id: PROVIDER.into(),
                event_source: job.job.provider_source.clone(),
                event_kind: event.into(),
                payload: json!({}),
                provider_created_at: None,
                provider_task_id: task_id,
                attempt_state: Some(attempt),
                next_job_state: Some(state),
                error,
                retry_classification: None,
                next_eligible_at: None,
                cancellation_result: None,
                final_cost_micros: cost.and_then(|cost| i64::try_from(cost).ok()),
                watermark_present: None,
                provenance: None,
            })
            .await
    }

    fn summary(&self, job: &MediaJobSnapshot) -> Result<Value, CoreError> {
        let files = job
            .assets
            .iter()
            .filter(|asset| asset.local_state == MediaAssetLocalState::Available)
            .map(|asset| self.0.resolve_local_asset_path(asset))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "jobId": job.job.id,
            "state": job.job.state,
            "modelId": job.job.model_id,
            "finalCostMicros": job.job.final_cost_micros,
            "files": files,
            "error": job.attempts.last().and_then(|attempt| attempt.error.as_ref())
        }))
    }
}

fn terminal_failure(
    status: &ProviderJobStatus,
    current: MediaJobState,
) -> Option<(MediaJobState, MediaJobAttemptState)> {
    if current == MediaJobState::PostProcessing {
        return (status.raw_status == "expired")
            .then_some((MediaJobState::Expired, MediaJobAttemptState::Succeeded));
    }
    match status.state {
        ProviderJobState::Failed if status.raw_status == "expired" => {
            Some((MediaJobState::Expired, MediaJobAttemptState::Expired))
        }
        ProviderJobState::Failed => Some((MediaJobState::Failed, MediaJobAttemptState::Failed)),
        ProviderJobState::Cancelled => {
            Some((MediaJobState::Cancelled, MediaJobAttemptState::Cancelled))
        }
        _ => None,
    }
}

fn attempt_id(job: &MediaJobSnapshot) -> Result<String, CoreError> {
    job.job
        .current_attempt_id
        .clone()
        .ok_or_else(|| CoreError::Internal("Video job has no active attempt.".into()))
}

fn resolve_adapter(
    db: &crate::db::Database,
    config_id: Option<&str>,
) -> Result<OpenRouterVideoAdapter, CoreError> {
    let official = |config: &crate::conversation::AgentConfig| {
        config.provider == PROVIDER
            && config
                .base_url
                .as_deref()
                .unwrap_or(BASE_URL)
                .trim_end_matches('/')
                == BASE_URL
    };
    let config = if let Some(id) = config_id {
        db.get_agent_config(id)?
    } else {
        let mut configs = db.list_agent_configs()?.into_iter().filter(official);
        let config = configs
            .next()
            .ok_or_else(|| missing("OpenRouter connection"))?;
        if configs.any(|other| other.api_key != config.api_key) {
            return Err(CoreError::InvalidInput(
                "Choose an OpenRouter provider_config_id because multiple API keys are configured."
                    .into(),
            ));
        }
        config
    };
    if !official(&config) {
        return Err(CoreError::InvalidInput(
            "Video generation requires an official OpenRouter connection.".into(),
        ));
    }
    let key = config.api_key.trim();
    let scope = blake3::hash(key.as_bytes()).to_hex();
    OpenRouterVideoAdapter::new(key, scope.as_str()).map_err(provider_error)
}

fn missing(field: &str) -> CoreError {
    CoreError::InvalidInput(format!("Provide {field} for the video action."))
}

fn provider_error(error: crate::media_generation::adapters::NormalizedProviderError) -> CoreError {
    CoreError::InvalidInput(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::Database, db_executor::DatabaseExecutor};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn approved_video_job_is_submitted_once_and_recovers_output() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base =
            url::Url::parse(&format!("http://{}/api/v1", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            for (request_line, status, mime, body) in [
                (
                    "POST /api/v1/videos",
                    "202 Accepted",
                    "application/json",
                    br#"{"id":"local-test","status":"pending"}"#.as_slice(),
                ),
                (
                    "GET /api/v1/videos/local-test",
                    "200 OK",
                    "application/json",
                    br#"{"id":"local-test","status":"completed","usage":{"cost":0.125}}"#
                        .as_slice(),
                ),
                (
                    "GET /api/v1/videos/local-test/content?index=0",
                    "200 OK",
                    "video/mp4",
                    b"\0\0\0\x18ftypmp42\0\0\0\0".as_slice(),
                ),
                (
                    "POST /api/v1/videos",
                    "202 Accepted",
                    "application/json",
                    br#"{"id":"next-sample","status":"pending"}"#.as_slice(),
                ),
                (
                    "POST /api/v1/videos",
                    "202 Accepted",
                    "application/json",
                    br#"{"id":"next-turn","status":"pending"}"#.as_slice(),
                ),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    header.push(stream.read_u8().await.unwrap());
                }
                let header = String::from_utf8(header).unwrap();
                assert!(header.starts_with(&format!("{request_line} ")), "{header}");
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")?
                            .trim()
                            .parse::<usize>()
                            .ok()
                    })
                    .unwrap_or(0);
                if content_length > 0 {
                    let mut data = vec![0; content_length];
                    stream.read_exact(&mut data).await.unwrap();
                    let request: Value = serde_json::from_slice(&data).unwrap();
                    assert_eq!(request["model"], "google/veo-3.1");
                }
                stream.write_all(format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()
                ).as_bytes()).await.unwrap();
                stream.write_all(body).await.unwrap();
            }
        });
        let db = Database::open_memory().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let runtime = MediaGenerationRuntime::with_asset_store(
            DatabaseExecutor::new(db.clone(), 16).unwrap(),
            MediaGenerationAssetStore::new(directory.path()),
        );
        let tool = GenerateVideoTool(runtime);
        assert!(tool.requires_confirmation(&json!({"action":"submit"})));
        assert!(!tool.requires_confirmation(&json!({"action":"status"})));
        let adapter = OpenRouterVideoAdapter::build("local-key".into(), "scope", base).unwrap();
        let context = ToolExecutionContext::new("call_0", "{}", &db, &[])
            .with_turn_id(Some("turn-1"))
            .with_execution_id("sample-1:0");
        let input = || {
            serde_json::from_value(json!({
                "modelId": "google/veo-3.1", "operation": "text_to_video",
                "prompt": "A paper boat on a lake", "durationSeconds": 8,
                "resolution": "720p", "aspectRatio": "16:9"
            }))
            .unwrap()
        };
        let submitted = tool
            .submit(&adapter, &context, "chat-1", input())
            .await
            .unwrap();
        assert_eq!(submitted["state"], "queued");
        let repeated = tool
            .submit(&adapter, &context, "chat-1", input())
            .await
            .unwrap();
        assert_eq!(submitted["jobId"], repeated["jobId"]);
        let id = submitted["jobId"].as_str().unwrap();
        assert!(tool.status(&adapter, "chat-2", id).await.is_err());
        let completed = tool.status(&adapter, "chat-1", id).await.unwrap();
        assert_eq!(completed["state"], "completed");
        assert_eq!(completed["finalCostMicros"], 125_000);
        assert!(std::path::Path::new(completed["files"][0].as_str().unwrap()).is_file());
        assert_eq!(
            tool.status(&adapter, "chat-1", id).await.unwrap()["state"],
            "completed"
        );
        let next_sample = ToolExecutionContext::new("call_0", "{}", &db, &[])
            .with_turn_id(Some("turn-1"))
            .with_execution_id("sample-2:0");
        let regenerated = tool
            .submit(&adapter, &next_sample, "chat-1", input())
            .await
            .unwrap();
        assert_ne!(regenerated["jobId"], submitted["jobId"]);
        let next_turn = ToolExecutionContext::new("call_0", "{}", &db, &[])
            .with_turn_id(Some("turn-2"))
            .with_execution_id("sample-3:0");
        let later = tool
            .submit(&adapter, &next_turn, "chat-1", input())
            .await
            .unwrap();
        assert_ne!(later["jobId"], regenerated["jobId"]);
        assert_ne!(later["jobId"], submitted["jobId"]);
        let unscoped = ToolExecutionContext::new("call_0", "{}", &db, &[]);
        assert!(tool
            .submit(&adapter, &unscoped, "chat-1", input())
            .await
            .is_err());
        server.await.unwrap();
    }
}
