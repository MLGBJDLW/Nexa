use async_trait::async_trait;
use nexa_core::{
    error::CoreError,
    tools::{Tool, ToolExecutionContext, ToolResult},
};
use serde_json::{json, Value};
use tauri::AppHandle;

pub struct PhoneTool(pub AppHandle);

#[async_trait]
impl Tool for PhoneTool {
    fn name(&self) -> &str {
        "phone_control"
    }
    fn description(&self) -> &str {
        "Observe the Android accessibility UI, then perform one verified-target action with the returned observation ID. Sensitive/password fields are unavailable. Observe again to verify the outcome."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["observe","click","set_text","scroll_forward","scroll_backward","back","home","open_app"]},
            "observationId":{"type":"string"},"nodeId":{"type":"integer"},"packageName":{"type":"string"},"text":{"type":"string"}
        },"required":["action"]})
    }
    fn requires_confirmation(&self, args: &Value) -> bool {
        args["action"] != "observe"
    }
    fn confirmation_message(&self, args: &Value) -> Option<String> {
        Some(format!(
            "Phone action: {} in {}",
            args["action"].as_str().unwrap_or("unknown"),
            args["packageName"].as_str().unwrap_or("the observed app")
        ))
    }
    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let arguments = serde_json::from_str(context.arguments)?;
        let value = crate::platform::call(&self.0, "phoneControl", arguments)
            .await
            .map_err(CoreError::InvalidInput)?;
        Ok(ToolResult {
            call_id: context.call_id.into(),
            content: value.to_string(),
            is_error: false,
            artifacts: Some(json!({"kind":"phoneObservation","result":value})),
        })
    }
}
