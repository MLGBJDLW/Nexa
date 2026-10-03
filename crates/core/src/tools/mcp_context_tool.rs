use super::{Tool, ToolCategory, ToolDef, ToolExecutionContext, ToolOutput, ToolResult};
use crate::{
    error::CoreError,
    mcp::{McpContentRequest, McpManager},
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::OnceLock};

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/mcp_context.json");

pub struct McpContextTool {
    manager: McpManager,
    bindings: BTreeMap<String, u64>,
    identities: BTreeMap<String, crate::mcp::McpToolIdentity>,
}
impl McpContextTool {
    pub fn new(manager: McpManager) -> Self {
        let bindings: BTreeMap<_, _> = manager
            .content_catalogs()
            .into_iter()
            .map(|(_, catalog)| (catalog.connector_id, catalog.authority_epoch))
            .collect();
        let identities = bindings
            .iter()
            .filter_map(|(id, epoch)| {
                manager
                    .content_identity(id, *epoch)
                    .map(|identity| (id.clone(), identity))
            })
            .collect();
        Self {
            manager,
            bindings,
            identities,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    action: String,
    server_id: Option<String>,
    uri: Option<String>,
    uri_template: Option<String>,
    name: Option<String>,
    arguments: Option<BTreeMap<String, String>>,
    offset: Option<usize>,
    limit: Option<usize>,
}

impl Args {
    fn validate(&self) -> Result<(), CoreError> {
        let no_target = self.uri.is_none()
            && self.uri_template.is_none()
            && self.name.is_none()
            && self.arguments.is_none();
        let no_page = self.offset.is_none() && self.limit.is_none();
        let valid =
            match self.action.as_str() {
                "list_servers" => self.server_id.is_none() && no_target,
                "list_resources" | "list_resource_templates" | "list_prompts" => no_target,
                "read_resource" => {
                    no_page
                        && self.uri_template.is_none()
                        && self.name.is_none()
                        && self.arguments.is_none()
                        && self.uri.as_ref().is_some_and(|uri| {
                            uri.len() <= 16_384 && reqwest::Url::parse(uri).is_ok()
                        })
                }
                "read_resource_template" => {
                    no_page
                        && self.uri.is_none()
                        && self.name.is_none()
                        && self.uri_template.as_ref().is_some_and(|template| {
                            !template.is_empty() && template.len() <= 16_384
                        })
                }
                "get_prompt" => {
                    no_page
                        && self.uri.is_none()
                        && self.uri_template.is_none()
                        && self
                            .name
                            .as_ref()
                            .is_some_and(|name| !name.is_empty() && name.len() <= 4096)
                }
                _ => false,
            } && self.limit.is_none_or(|limit| (1..=128).contains(&limit))
                && (self.action == "list_servers"
                    || self.server_id.as_ref().is_some_and(|id| !id.is_empty()));
        if !valid {
            return Err(CoreError::InvalidInput("MCP content action requires its exact target fields; reads do not accept pagination and catalog lists do not accept URI/name/arguments.".into()));
        }
        Ok(())
    }
}

pub(crate) fn validate_arguments(arguments: &str) -> Result<(), CoreError> {
    serde_json::from_str::<Args>(arguments)?.validate()
}

#[async_trait]
impl Tool for McpContextTool {
    fn name(&self) -> &str {
        "mcp_context"
    }
    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }
    fn parameters_schema(&self) -> Value {
        ToolDef::from_json(&DEF, DEF_JSON).parameters.clone()
    }
    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::Mcp]
    }
    fn canonical_identity_for_arguments(
        &self,
        args: &Value,
    ) -> Option<crate::mcp::McpToolIdentity> {
        let mut identity = self
            .identities
            .get(args.get("server_id")?.as_str()?)?
            .clone();
        identity.id.tool_name = match args.get("action")?.as_str()? {
            "read_resource" | "read_resource_template" => "resources/read",
            "get_prompt" => "prompts/get",
            _ => return None,
        }
        .into();
        Some(identity)
    }
    fn is_read_only(&self, _: &Value) -> bool {
        true
    }
    fn confirmation_message(&self, args: &Value) -> Option<String> {
        Some(format!(
            "Read MCP content from connector '{}': {}",
            args.get("server_id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            args.get("uri")
                .or_else(|| args.get("uri_template"))
                .or_else(|| args.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
        ))
    }
    fn resource_keys(&self, args: &Value) -> Vec<String> {
        args.get("server_id")
            .and_then(Value::as_str)
            .map(|id| vec![format!("mcp_connector:{id}")])
            .unwrap_or_default()
    }
    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let args: Args = serde_json::from_str(context.arguments)?;
        args.validate()?;
        let catalogs = self
            .manager
            .content_catalogs()
            .into_iter()
            .filter(|(_, catalog)| {
                self.bindings.get(&catalog.connector_id) == Some(&catalog.authority_epoch)
            })
            .collect::<Vec<_>>();
        if args.action == "list_servers" {
            if args.server_id.is_some()
                || args.uri.is_some()
                || args.name.is_some()
                || args.arguments.is_some()
            {
                return Err(CoreError::InvalidInput(
                    "list_servers does not accept a content target".into(),
                ));
            }
            let offset = args.offset.unwrap_or(0);
            let limit = args.limit.unwrap_or(50);
            if !(1..=128).contains(&limit) || offset > catalogs.len() {
                return Err(CoreError::InvalidInput("Invalid MCP server page".into()));
            }
            let next = offset.saturating_add(limit).min(catalogs.len());
            let servers = catalogs[offset..next].iter().map(|(name, catalog)| json!({"id":catalog.connector_id,"name":name,"complete":catalog.complete,"resourceCount":catalog.content.resources.len(),"templateCount":catalog.content.resource_templates.len(),"promptCount":catalog.content.prompts.len(),"diagnostics":catalog.diagnostics})).collect::<Vec<_>>();
            let result = json!({"servers":servers,"total":catalogs.len(),"nextOffset":(next<catalogs.len()).then_some(next)});
            return Ok(ToolResult::from_output(
                context.call_id,
                false,
                ToolOutput::text(serde_json::to_string_pretty(&result)?),
            ));
        }
        let id = args
            .server_id
            .as_deref()
            .ok_or_else(|| CoreError::InvalidInput("server_id is required".into()))?;
        let (_, catalog) = catalogs.iter().find(|(_, catalog)| catalog.connector_id == id).ok_or_else(|| CoreError::Mcp("Connector changed or is not available to this turn. Refresh the connector and start a new turn.".into()))?;
        let request = match args.action.as_str() {
            "read_resource"
                if args.name.is_none()
                    && args.arguments.is_none()
                    && args.offset.is_none()
                    && args.limit.is_none() =>
            {
                Some(McpContentRequest::ReadResource {
                    uri: args
                        .uri
                        .ok_or_else(|| CoreError::InvalidInput("uri is required".into()))?,
                })
            }
            "get_prompt" if args.uri.is_none() && args.offset.is_none() && args.limit.is_none() => {
                Some(McpContentRequest::GetPrompt {
                    name: args
                        .name
                        .ok_or_else(|| CoreError::InvalidInput("name is required".into()))?,
                    arguments: args.arguments.unwrap_or_default(),
                })
            }
            "read_resource_template" => Some(McpContentRequest::ReadResourceTemplate {
                uri_template: args
                    .uri_template
                    .ok_or_else(|| CoreError::InvalidInput("uri_template is required".into()))?,
                arguments: args.arguments.unwrap_or_default(),
            }),
            "list_resources" | "list_resource_templates" | "list_prompts"
                if args.uri.is_none() && args.name.is_none() && args.arguments.is_none() =>
            {
                None
            }
            _ => {
                return Err(CoreError::InvalidInput(
                    "MCP content action has incompatible target or pagination arguments".into(),
                ))
            }
        };
        if let Some(request) = request {
            return self
                .manager
                .read_content(
                    id,
                    catalog.authority_epoch,
                    request,
                    context.cancel_token,
                    context.call_id,
                )
                .await;
        }
        let entries = match args.action.as_str() {
            "list_resources" => &catalog.content.resources,
            "list_resource_templates" => &catalog.content.resource_templates,
            _ => &catalog.content.prompts,
        };
        let offset = args.offset.unwrap_or(0);
        let limit = args.limit.unwrap_or(50);
        if !(1..=128).contains(&limit) || offset > entries.len() {
            return Err(CoreError::InvalidInput("Invalid MCP catalog page".into()));
        }
        let next = offset.saturating_add(limit).min(entries.len());
        let part_complete = match args.action.as_str() {
            "list_resources" => catalog.content.resources_complete,
            "list_resource_templates" => catalog.content.resource_templates_complete,
            _ => catalog.content.prompts_complete,
        };
        let result = json!({"serverId":id,"complete":catalog.complete && part_complete,"diagnostics":catalog.content.content_diagnostics,"total":entries.len(),"items":&entries[offset..next],"nextOffset":(next<entries.len()).then_some(next)});
        Ok(ToolResult::from_output(
            context.call_id,
            false,
            ToolOutput::text(serde_json::to_string_pretty(&result)?),
        ))
    }
}
