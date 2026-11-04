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
        "Control the Android phone. status checks access; list_apps finds installed app names/package IDs. observe returns accessibility nodes; prefer node click/set_text. screenshot returns current pixels (Android 11+) and screenshot-pixel coordinates for tap when nodes are unavailable. Every action requires its fresh observationId and packageName. Sensitive/locked screens are unavailable. After acting, observe again to verify the outcome. For messaging, use the user's specified recipient and text, verify the correct conversation before sending, and never repeat an uncertain send. Missing OS access requires the user to follow the recovery steps; repeated calls cannot grant it."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["status","list_apps","observe","screenshot","tap","click","set_text","scroll_forward","scroll_backward","back","home","open_app"]},
            "observationId":{"type":"string"},"nodeId":{"type":"integer"},"packageName":{"type":"string"},"text":{"type":"string"},
            "query":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"}
        },"required":["action"]})
    }
    fn requires_confirmation(&self, args: &Value) -> bool {
        !matches!(
            args["action"].as_str(),
            Some("observe" | "status" | "list_apps")
        )
    }
    fn is_read_only(&self, args: &Value) -> bool {
        matches!(
            args["action"].as_str(),
            Some("observe" | "status" | "list_apps" | "screenshot")
        )
    }
    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        false
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
        let mut value = crate::platform::call(&self.0, "phoneControl", arguments)
            .await
            .map_err(CoreError::InvalidInput)?;
        let pixels = value
            .get_mut("image")
            .and_then(Value::as_object_mut)
            .and_then(|image| image.remove("base64"));
        let mut artifacts = json!({"kind":"phoneObservation","result":value});
        if let Some(pixels) = pixels {
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
