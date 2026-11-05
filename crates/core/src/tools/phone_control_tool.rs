//! One phone-control contract for local Android and paired desktop hosts.
use super::{Tool, ToolExecutionContext, ToolResult};
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
        let mut value = self
            .0
            .control(
                context.call_id,
                arguments,
                context.cancel_token.cloned().unwrap_or_default(),
            )
            .await?;
        let pixels = value
            .get_mut("image")
            .and_then(Value::as_object_mut)
            .and_then(|image| image.remove("base64"));
        let mut artifacts = json!({"kind":"phoneObservation","result":value});
        if let Some(pixels) = pixels.and_then(|value| value.as_str().map(str::to_owned)) {
            if pixels.len() > 6 * 1024 * 1024 {
                return Err(CoreError::InvalidInput("Phone screenshot exceeds the transport envelope. Observe accessible nodes instead.".into()));
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
