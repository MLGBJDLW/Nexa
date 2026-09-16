use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::CoreError;

use super::{Tool, ToolCategory, ToolExecutionContext, ToolResult};

const DEFAULT_WAIT_UP_TO_MS: u64 = 2_500;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObserveArgs {
    activity_id: String,
    #[serde(default)]
    after_seq: u64,
    #[serde(default = "default_wait_up_to_ms")]
    wait_up_to_ms: u64,
    #[serde(default)]
    wait_for: WaitFor,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WaitFor {
    #[default]
    Output,
    Completion,
}

fn default_wait_up_to_ms() -> u64 {
    DEFAULT_WAIT_UP_TO_MS
}

pub struct ActivityObserveTool;

#[async_trait]
impl Tool for ActivityObserveTool {
    fn name(&self) -> &str {
        "activity_observe"
    }

    fn description(&self) -> &str {
        "Observe a process using the exact activityId and last cursor returned by run_shell or another runtime tool. For compilation, use waitFor=completion and waitUpToMs=30000 to remain attached through intermediate output; progress continues to reach the UI. Output mode returns on new output within 2.5 seconds. Do not relaunch a still-running build or invent process identifiers."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "activityId": {
                    "type": "string",
                    "description": "Activity identifier returned by a runtime-backed tool."
                },
                "afterSeq": {
                    "type": "integer",
                    "minimum": 0,
                    "default": 0,
                    "description": "Return only events whose sequence is greater than this cursor."
                },
                "waitUpToMs": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 60000,
                    "default": DEFAULT_WAIT_UP_TO_MS,
                    "description": "Completion mode waits up to 60 seconds; output mode caps at 2500ms. Cancellation remains immediate."
                },
                "waitFor": {
                    "type": "string", "enum": ["output", "completion"], "default": "output",
                    "description": "Use completion for builds and tests; output for interactive services."
                }
            },
            "required": ["activityId"],
            "additionalProperties": false
        })
    }

    fn categories(&self) -> &'static [ToolCategory] {
        &[
            ToolCategory::Core,
            ToolCategory::Process,
            ToolCategory::Terminal,
            ToolCategory::BrowserInteract,
            ToolCategory::DesktopInteract,
        ]
    }

    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let ToolExecutionContext {
            call_id,
            arguments,
            conversation_id,
            activity_runtime,
            ..
        } = context;
        let args: ObserveArgs = serde_json::from_str(arguments).map_err(|error| {
            CoreError::InvalidInput(format!("Invalid activity_observe arguments: {error}"))
        })?;
        let activity_id = args.activity_id.trim();
        if activity_id.is_empty() {
            return Err(CoreError::InvalidInput(
                "activityId cannot be empty".to_string(),
            ));
        }
        let runtime = activity_runtime.ok_or_else(|| {
            CoreError::Internal("Activity Runtime is unavailable for this tool call".to_string())
        })?;
        let record = runtime.get(activity_id).ok_or_else(|| {
            CoreError::InvalidInput(format!("Activity '{activity_id}' was not found"))
        })?;
        if record.conversation_id.as_deref() != conversation_id {
            return Err(CoreError::InvalidInput(
                "Activity belongs to a different conversation".to_string(),
            ));
        }
        let started = std::time::Instant::now();
        let budget = Duration::from_millis(args.wait_up_to_ms);
        let observation = match args.wait_for {
            WaitFor::Output => runtime.observe(activity_id, args.after_seq, budget).await?,
            WaitFor::Completion => {
                runtime
                    .wait_for_completion(activity_id, args.after_seq, budget)
                    .await?
            }
        };
        let content = serde_json::to_string_pretty(&observation)?;
        Ok(ToolResult {
            call_id: call_id.to_string(),
            content,
            is_error: false,
            artifacts: Some(serde_json::json!({
                "kind": "activityObservation",
                "activity": observation,
                "waitedMs": started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                "nextAction": if observation.record.state.is_terminal() { serde_json::Value::Null } else {
                    serde_json::json!({"tool":"activity_observe", "arguments": {
                        "activityId":activity_id, "afterSeq":observation.cursor,
                        "waitFor":"completion", "waitUpToMs":30000
                    }})
                },
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{ActivityEventKind, ActivityRuntime, ActivitySpec, ActivitySurface};
    use crate::db::Database;

    #[tokio::test]
    async fn completion_wait_stays_attached_through_output_and_returns_exit_receipt() {
        use crate::activity::ActivityState;
        let db = Database::open_memory().unwrap();
        let runtime = ActivityRuntime::new();
        let record = runtime
            .start(ActivitySpec::new(ActivitySurface::Process, "run_shell"))
            .unwrap();
        let worker = runtime.clone();
        let id = record.activity_id.clone();
        let finish = tokio::spawn(async move {
            worker
                .append(
                    &id,
                    ActivityEventKind::StdoutChunk,
                    serde_json::json!({"data":"Compiling 1/2"}),
                )
                .unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            worker
                .transition(
                    &id,
                    ActivityState::Completed,
                    serde_json::json!({"exitCode":0}),
                )
                .unwrap();
        });
        let arguments = serde_json::json!({"activityId":record.activity_id, "afterSeq":1, "waitFor":"completion", "waitUpToMs":1000}).to_string();
        let result = ActivityObserveTool
            .execute(
                ToolExecutionContext::new("build-wait", &arguments, &db, &[])
                    .with_activity_runtime(&runtime),
            )
            .await
            .unwrap();
        finish.await.unwrap();
        let artifacts = result.artifacts.unwrap();
        assert_eq!(artifacts["activity"]["record"]["state"], "completed");
        assert_eq!(artifacts["activity"]["timedOut"], false);
        assert!(artifacts["nextAction"].is_null());
        assert!(artifacts["activity"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["payload"]["data"] == "Compiling 1/2"));
    }

    #[tokio::test]
    async fn cancelling_a_completion_wait_does_not_kill_the_owned_process() {
        let db = Database::open_memory().unwrap();
        let runtime = ActivityRuntime::new();
        let record = runtime
            .start(ActivitySpec::new(ActivitySurface::Process, "run_shell"))
            .unwrap();
        let args = serde_json::json!({"activityId":record.activity_id,"waitFor":"completion","waitUpToMs":60000}).to_string();
        let waiting = ActivityObserveTool.execute(
            ToolExecutionContext::new("wait", &args, &db, &[]).with_activity_runtime(&runtime),
        );
        assert!(tokio::time::timeout(Duration::from_millis(30), waiting)
            .await
            .is_err());
        assert!(!runtime
            .get(&record.activity_id)
            .unwrap()
            .state
            .is_terminal());
        let snapshot = runtime
            .observe(&record.activity_id, 0, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(snapshot.record.activity_id, record.activity_id);
    }

    #[tokio::test]
    async fn observe_tool_returns_only_events_after_cursor() {
        let db = Database::open_memory().unwrap();
        let runtime = ActivityRuntime::new();
        let activity = runtime
            .start(ActivitySpec::new(ActivitySurface::Process, "run_shell"))
            .unwrap();
        runtime
            .append(
                &activity.activity_id,
                ActivityEventKind::StdoutChunk,
                serde_json::json!({ "data": "hello" }),
            )
            .unwrap();
        let arguments = serde_json::json!({
            "activityId": activity.activity_id,
            "afterSeq": 1,
            "waitUpToMs": 0,
        })
        .to_string();

        let result = ActivityObserveTool
            .execute(
                ToolExecutionContext::new("observe-1", &arguments, &db, &[])
                    .with_activity_runtime(&runtime),
            )
            .await
            .unwrap();
        let observation = &result.artifacts.unwrap()["activity"];
        assert_eq!(observation["events"].as_array().unwrap().len(), 1);
        assert_eq!(observation["events"][0]["seq"], 2);
    }

    #[tokio::test]
    async fn observe_tool_rejects_another_conversations_activity() {
        let db = Database::open_memory().unwrap();
        let runtime = ActivityRuntime::new();
        let activity = runtime
            .start(
                ActivitySpec::new(ActivitySurface::Process, "run_shell")
                    .with_conversation_id("conversation-1"),
            )
            .unwrap();
        let arguments = serde_json::json!({
            "activityId": activity.activity_id,
            "waitUpToMs": 0,
        })
        .to_string();

        let error = ActivityObserveTool
            .execute(
                ToolExecutionContext::new("observe-2", &arguments, &db, &[])
                    .with_conversation_id(Some("conversation-2"))
                    .with_activity_runtime(&runtime),
            )
            .await
            .expect_err("cross-conversation activity output must stay private");
        assert!(error.to_string().contains("different conversation"));
    }
}
