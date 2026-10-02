//! MCP tool adapter. A registry captures a tool definition and authority epoch;
//! its connector slot owns connection/catalog refresh and execution fencing.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::CoreError;
use crate::mcp::{McpConnectorSlot, McpToolIdentity, McpToolInfo};

use super::{Tool, ToolCategory, ToolResult};

pub struct McpTool {
    info: McpToolInfo,
    registry_name: String,
    ownership_selector: String,
    identity: McpToolIdentity,
    description: String,
    connector_name: String,
    slot: Arc<McpConnectorSlot>,
    authority_epoch: u64,
}

impl McpTool {
    pub(crate) fn new(
        info: McpToolInfo,
        slot: Arc<McpConnectorSlot>,
        identity: McpToolIdentity,
        authority_epoch: u64,
        connector_name: &str,
    ) -> Self {
        let description = match info.description.as_deref() {
            Some(text) if !text.trim().is_empty() => {
                format!("MCP connector '{connector_name}': {}", text.trim())
            }
            _ => format!("MCP connector '{connector_name}' tool '{}'", info.name),
        };
        Self {
            registry_name: identity.id.model_alias(),
            ownership_selector: crate::mcp::identity::ownership_selector(
                connector_name,
                &info.name,
            ),
            info,
            identity,
            description,
            connector_name: connector_name.into(),
            slot,
            authority_epoch,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.registry_name
    }
    fn canonical_identity(&self) -> Option<McpToolIdentity> {
        Some(self.identity.clone())
    }
    fn ownership_selector(&self) -> &str {
        &self.ownership_selector
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::Mcp]
    }
    fn parameters_schema(&self) -> Value {
        self.info.input_schema.clone()
    }
    fn is_read_only(&self, _args: &Value) -> bool {
        false
    }
    fn resource_keys(&self, _args: &Value) -> Vec<String> {
        vec![format!("mcp_connector:{}", self.identity.id.connector_id)]
    }
    fn confirmation_message(&self, _args: &Value) -> Option<String> {
        Some(format!(
            "Run '{}' on MCP connector '{}'.",
            self.info.name, self.connector_name
        ))
    }
    async fn execute(
        &self,
        context: super::ToolExecutionContext<'_>,
    ) -> Result<ToolResult, CoreError> {
        let args = serde_json::from_str(context.arguments).map_err(|error| {
            CoreError::InvalidInput(format!("Invalid MCP tool arguments: {error}"))
        })?;
        let result = self
            .slot
            .call(
                &self.identity,
                self.authority_epoch,
                &self.info,
                args,
                context.cancel_token,
            )
            .await;
        match result {
            Ok(result) => Ok(result.into_tool_result(context.call_id, &self.identity)),
            Err(error) => {
                let uncertain = matches!(error, CoreError::McpTransport(_));
                Ok(ToolResult {
                    call_id: context.call_id.into(),
                    content: format!("MCP tool error: {error}"),
                    is_error: true,
                    artifacts: Some(serde_json::json!({
                        "kind":"mcpToolError", "toolIdentity":self.identity,
                        "code":if uncertain {"mcp_transport_failed"} else {"mcp_call_unavailable"},
                        "retryable":!uncertain,
                        "sideEffect":if uncertain {"may_have_occurred"} else {"none"},
                        "message":"The failed call was not automatically replayed.",
                    })),
                })
            }
        }
    }
}
