//! One phone-control contract for local Android and paired desktop hosts.
use super::{Tool, ToolExecutionContext, ToolResult};
use crate::activity::{
    ActivityEventKind, ActivityRuntime, ActivitySpec, ActivityState, ActivitySurface,
};
use crate::error::CoreError;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait PhoneControlHost: Send + Sync {
    async fn control(
        &self,
        call_id: &str,
        arguments: Value,
        cancel: CancellationToken,
    ) -> Result<Value, CoreError>;
}
pub struct PhoneControlTool(pub Arc<dyn PhoneControlHost>);

pub(crate) fn is_phone_mutation(args: &Value) -> bool {
    matches!(
        args["action"].as_str(),
        Some(
            "tap"
                | "click"
                | "set_text"
                | "scroll_forward"
                | "scroll_backward"
                | "back"
                | "home"
                | "open_app"
        )
    )
}

fn receipt_error(call_id: &str, code: &str, message: &str, uncertain: bool) -> ToolResult {
    super::structured_tool_error_result_with_side_effect(
        call_id,
        code,
        message.to_string(),
        json!({"tool":"phone_control","recovery":"Inspect a fresh phone observation before continuing. Never repeat an uncertain send; saved receipts do not restore screen access or authorize input."}),
        false,
        if uncertain {
            super::ToolSideEffect::MayHaveOccurred
        } else {
            super::ToolSideEffect::NotStarted
        },
        None,
    )
}

/// Use the existing durable activity journal for phone input, just as desktop
/// input does. A dropped caller cannot erase the possibility of native input.
/// Receipts contain identities/outcomes, never screen pixels or typed text.
struct PhoneActionReceipt {
    runtime: ActivityRuntime,
    id: String,
    settled: bool,
}

impl PhoneActionReceipt {
    fn start(context: &ToolExecutionContext<'_>, args: &Value) -> Result<Self, ToolResult> {
        let Some(runtime) = context
            .activity_runtime
            .filter(|runtime| runtime.is_persistent())
        else {
            return Err(receipt_error(
                context.call_id,
                "phone_receipt_unavailable",
                "Phone input requires a persistent action-receipt runtime. No input was sent.",
                false,
            ));
        };
        // Provider call IDs can repeat in another turn. The host execution ID
        // is authoritative when available, with the full turn scope otherwise.
        let identity = json!([
            context.conversation_id,
            context.turn_id,
            context.execution_id,
            context.call_id
        ]);
        let id = format!(
            "phone_control:{}",
            blake3::hash(identity.to_string().as_bytes()).to_hex()
        );
        let mut spec = ActivitySpec::new(ActivitySurface::Phone, "phone_control")
            .with_activity_id(&id)
            .with_session_id(context.call_id);
        if let Some(conversation) = context.conversation_id {
            spec = spec.with_conversation_id(conversation);
        }
        if let Some(turn) = context.turn_id {
            spec = spec.with_turn_id(turn);
        }
        if runtime.get(&id).is_some() {
            return Err(receipt_error(context.call_id, "phone_action_already_recorded",
                "This execution already has a durable phone action receipt and was not replayed. Observe the phone before deciding what remains necessary.", true));
        }
        runtime.start(spec).map_err(|_| {
            receipt_error(
                context.call_id,
                "phone_receipt_unavailable",
                "The phone action receipt could not be started. No input was sent.",
                false,
            )
        })?;
        if runtime.append(&id, ActivityEventKind::Progress, json!({
            "stage":"dispatching", "effectMayHaveOccurred":true,
            "action":args["action"], "deviceId":args["deviceId"].as_str().unwrap_or("local"),
            "packageName":args["packageName"], "consumedObservationId":args["observationId"],
            "screenContentPersistence":"removed",
        })).is_err() {
            let _ = runtime.transition(&id, ActivityState::Failed, json!({"stage":"precommit_rejected","effectMayHaveOccurred":false}));
            return Err(receipt_error(context.call_id, "phone_receipt_unavailable",
                "The phone dispatch receipt could not be persisted. No input was sent.", false));
        }
        Ok(Self {
            runtime: runtime.clone(),
            id,
            settled: false,
        })
    }

    fn finish(&mut self, result: &Value) -> Result<(), CoreError> {
        self.runtime.transition(&self.id,
            if result["code"].is_string() { ActivityState::Failed } else { ActivityState::Completed },
            json!({
                "stage":"returned", "code":result["code"],
                "actionAccepted":result["actionAccepted"], "actionPerformed":result["actionPerformed"],
                "effectMayHaveOccurred":result["effectMayHaveOccurred"],
                "observationId":result["observationId"], "observationPending":result["observationPending"],
                "screenContentPersistence":"removed",
            }))?;
        self.settled = true;
        Ok(())
    }
}

impl Drop for PhoneActionReceipt {
    fn drop(&mut self) {
        if !self.settled {
            let _ = self.runtime.transition(&self.id, ActivityState::Failed,
                json!({"stage":"interrupted","effectMayHaveOccurred":true,"recovery":"Observe the phone; never replay uncertain input."}));
        }
    }
}

#[async_trait]
impl Tool for PhoneControlTool {
    fn name(&self) -> &str {
        "phone_control"
    }
    fn description(&self) -> &str {
        "Control the local Android phone or an explicitly enabled paired phone. list_devices/status checks available devices and access; keep deviceId for a paired target. list_apps finds installed apps. observe returns nodes; prefer node click/set_text. set_text replaces Unicode text and verifies readback; omit nodeId only for the focused editor (Android 13+ native fallback). screenshot returns pixels (Android 11+) for coordinate tap. Targeted input and back need observationId and packageName. Recovery home needs neither; open_app needs only the target packageName. Use these to recover from ui_unavailable, then observe again. Actions return a new post-action observation: inspect it and use its new IDs. Coordinates are revalidated; stale_observation sends no input. Use returned fresh evidence if present; if the UI is still unavailable, recover with home/open_app or ask the user to continue manually. Sensitive/locked screens are unavailable. Verify the user's recipient and text before sending; never repeat an uncertain send. Missing access requires user recovery."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list_devices","status","list_apps","observe","screenshot","tap","click","set_text","scroll_forward","scroll_backward","back","home","open_app"]},
            "deviceId":{"type":"string","description":"Exact device ID from list_devices. Omit on the phone itself."},
            "observationId":{"type":"string"},"nodeId":{"type":"integer"},"packageName":{"type":"string"},"text":{"type":"string"},
            "query":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"}
        },"required":["action"]})
    }
    fn requires_confirmation(&self, args: &Value) -> bool {
        !matches!(
            args["action"].as_str(),
            Some("observe" | "status" | "list_apps" | "list_devices")
        )
    }
    fn is_read_only(&self, args: &Value) -> bool {
        matches!(
            args["action"].as_str(),
            Some("observe" | "status" | "list_apps" | "list_devices" | "screenshot")
        )
    }
    fn is_concurrency_safe(&self, _: &Value) -> bool {
        false
    }
    fn confirmation_message(&self, args: &Value) -> Option<String> {
        Some(format!(
            "Phone action: {} on {} in {}",
            args["action"].as_str().unwrap_or("unknown"),
            args["deviceId"].as_str().unwrap_or("this phone"),
            args["packageName"].as_str().unwrap_or("the observed app")
        ))
    }
    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let arguments: Value = serde_json::from_str(context.arguments)?;
        if !arguments.is_object()
            || !matches!(
                arguments["action"].as_str(),
                Some(
                    "list_devices"
                        | "status"
                        | "list_apps"
                        | "observe"
                        | "screenshot"
                        | "tap"
                        | "click"
                        | "set_text"
                        | "scroll_forward"
                        | "scroll_backward"
                        | "back"
                        | "home"
                        | "open_app"
                )
            )
        {
            return Err(CoreError::InvalidInput(
                "Provide an object with a supported phone action.".into(),
            ));
        }
        let mutation = is_phone_mutation(&arguments);
        let cancel = context.cancel_token.cloned().unwrap_or_default();
        if cancel.is_cancelled() {
            return Ok(receipt_error(
                context.call_id,
                "phone_control_cancelled",
                "Phone control was cancelled before dispatch. No input was sent.",
                false,
            ));
        }
        let mut receipt = if mutation {
            match PhoneActionReceipt::start(&context, &arguments) {
                Ok(receipt) => Some(receipt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };
        let result = if cancel.is_cancelled() {
            Ok(
                json!({"code":"phone_control_cancelled","retryable":false,"actionPerformed":false,"message":"Phone control was cancelled before dispatch. No input was sent."}),
            )
        } else {
            self.0.control(context.call_id, arguments, cancel).await
        };
        let mut value = match result {
            Ok(value) => value,
            Err(_) if mutation => {
                json!({"code":"phone_action_uncertain","retryable":false,"message":"The phone host did not return a verified action outcome. Observe before continuing; never repeat an uncertain send."})
            }
            Err(error) => return Err(error),
        };
        if !value.is_object() {
            value = json!({"code":"phone_response_invalid","retryable":false,"message":"The phone host returned an invalid response. Observe before continuing."});
        }
        if let Some(receipt) = receipt.as_mut() {
            // Only an explicit native no-input receipt proves safe failure.
            // Transport errors/cancellation may arrive after input was sent.
            let uncertain =
                value["actionPerformed"] != false || value["actionOutcomeUnknown"] == true;
            value["effectMayHaveOccurred"] = json!(uncertain);
            if value["code"].is_string() && uncertain {
                value["retryable"] = json!(false);
                value["recovery"] =
                    json!("Observe the phone before continuing; never repeat an uncertain send.");
            }
            value["actionReceiptId"] = json!(receipt.id);
            if receipt.finish(&value).is_err() {
                return Ok(receipt_error(context.call_id, "phone_receipt_unavailable",
                    "The phone outcome could not be persisted after dispatch. Observe before continuing; do not repeat the action.", true));
            }
        }
        let pixels = value
            .get_mut("image")
            .and_then(Value::as_object_mut)
            .and_then(|image| image.remove("base64"));
        let mut artifacts = json!({"kind":"phoneObservation","result":value,
            "code":value["code"],"sideEffect":if mutation && value["effectMayHaveOccurred"] == true {"may_have_occurred"} else {"not_started"}});
        if let Some(pixels) = pixels.and_then(|value| value.as_str().map(str::to_owned)) {
            if pixels.len() > 6 * 1024 * 1024 {
                return Ok(receipt_error(context.call_id, "phone_screenshot_too_large",
                    "Phone screenshot exceeds the transport envelope. Observe accessible nodes instead; do not repeat an input action to obtain pixels.", mutation));
            }
            artifacts["toolOutput"] = json!({"attachments":[{"name":"Android screen","mimeType":"image/jpeg","data":{"base64":pixels}}]});
        }
        Ok(ToolResult {
            call_id: context.call_id.into(),
            content: value.to_string(),
            is_error: value["code"].is_string(),
            artifacts: Some(artifacts),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct Host {
        calls: AtomicUsize,
        response: Value,
        fail: bool,
        pending: bool,
        db: Database,
    }

    #[async_trait]
    impl PhoneControlHost for Host {
        async fn control(
            &self,
            _: &str,
            arguments: Value,
            _: CancellationToken,
        ) -> Result<Value, CoreError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if is_phone_mutation(&arguments) {
                let started: u32 = self.db.conn().query_row(
                    "SELECT COUNT(*) FROM activity_events WHERE event_json LIKE '%dispatching%'", [], |row| row.get(0),
                ).unwrap();
                assert!(
                    started > 0,
                    "input cannot precede its durable dispatch receipt"
                );
            }
            if self.pending {
                std::future::pending::<()>().await;
            }
            if self.fail {
                return Err(CoreError::Internal("private-native-error".into()));
            }
            Ok(self.response.clone())
        }
    }

    fn host(db: &Database, response: Value, fail: bool, pending: bool) -> Arc<Host> {
        Arc::new(Host {
            calls: AtomicUsize::new(0),
            response,
            fail,
            pending,
            db: db.clone(),
        })
    }

    #[tokio::test]
    async fn phone_input_requires_durability_but_observation_does_not() {
        let db = Database::open_memory().unwrap();
        let host = host(&db, json!({"observationId":"fresh"}), false, false);
        let tool = PhoneControlTool(host.clone());
        let denied = tool
            .execute(ToolExecutionContext::new(
                "action",
                r#"{"action":"home"}"#,
                &db,
                &[],
            ))
            .await
            .unwrap();
        assert_eq!(denied.artifacts.unwrap()["sideEffect"], "not_started");
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
        let observed = tool
            .execute(ToolExecutionContext::new(
                "observe",
                r#"{"action":"observe"}"#,
                &db,
                &[],
            ))
            .await
            .unwrap();
        assert!(!observed.is_error);
        assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn phone_receipts_survive_restart_without_replaying_input_or_saving_screen_content() {
        let db = Database::open_memory().unwrap();
        let host = host(
            &db,
            json!({"actionAccepted":true,"observationId":"after","nodes":[{"text":"private-screen"}],"image":{"base64":"private-pixels"}}),
            false,
            false,
        );
        let tool = PhoneControlTool(host.clone());
        let args = r#"{"action":"set_text","text":"private-input","observationId":"before","packageName":"test.app"}"#;
        let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
        let result = tool
            .execute(
                ToolExecutionContext::new("call", args, &db, &[])
                    .with_conversation_id(Some("chat"))
                    .with_turn_id(Some("turn"))
                    .with_execution_id("execution")
                    .with_activity_runtime(&runtime),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        let id = result.artifacts.unwrap()["result"]["actionReceiptId"]
            .as_str()
            .unwrap()
            .to_string();
        drop(runtime);
        let recovered = ActivityRuntime::with_database(db.clone()).unwrap();
        let receipt = recovered.observe(&id, 0, Duration::ZERO).await.unwrap();
        assert_eq!(receipt.record.state, ActivityState::Completed);
        let serialized = serde_json::to_string(&receipt).unwrap();
        for secret in ["private-input", "private-screen", "private-pixels"] {
            assert!(!serialized.contains(secret));
        }
        let replay = tool
            .execute(
                ToolExecutionContext::new("call", args, &db, &[])
                    .with_conversation_id(Some("chat"))
                    .with_turn_id(Some("turn"))
                    .with_execution_id("execution")
                    .with_activity_runtime(&recovered),
            )
            .await
            .unwrap();
        assert_eq!(
            replay.artifacts.unwrap()["code"],
            "phone_action_already_recorded"
        );
        assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn phone_transport_failure_and_dropped_call_leave_uncertain_receipts() {
        for pending in [false, true] {
            let db = Database::open_memory().unwrap();
            let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
            let host = host(&db, json!({}), true, pending);
            let tool = PhoneControlTool(host.clone());
            let context = ToolExecutionContext::new("call", r#"{"action":"home"}"#, &db, &[])
                .with_activity_runtime(&runtime);
            if pending {
                assert!(
                    tokio::time::timeout(Duration::from_millis(20), tool.execute(context))
                        .await
                        .is_err()
                );
            } else {
                let result = tool.execute(context).await.unwrap();
                assert!(result.is_error);
                assert!(!result.content.contains("private-native-error"));
                assert_eq!(result.artifacts.unwrap()["sideEffect"], "may_have_occurred");
            }
            let id = runtime.list()[0].activity_id.clone();
            drop(runtime);
            let recovered = ActivityRuntime::with_database(db.clone()).unwrap();
            let receipt = recovered.observe(&id, 0, Duration::ZERO).await.unwrap();
            assert_eq!(receipt.record.state, ActivityState::Failed);
            assert_eq!(
                receipt.events.last().unwrap().payload["detail"]["effectMayHaveOccurred"],
                true
            );
            assert_eq!(host.calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn native_no_input_errors_remain_safe_and_explicit() {
        let db = Database::open_memory().unwrap();
        let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
        let tool = PhoneControlTool(host(
            &db,
            json!({"code":"stale_observation","retryable":true,"actionPerformed":false}),
            false,
            false,
        ));
        let result = tool
            .execute(
                ToolExecutionContext::new("call", r#"{"action":"click"}"#, &db, &[])
                    .with_activity_runtime(&runtime),
            )
            .await
            .unwrap();
        let artifacts = result.artifacts.unwrap();
        assert_eq!(artifacts["sideEffect"], "not_started");
        assert_eq!(artifacts["result"]["retryable"], true);
    }
}
