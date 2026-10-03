//! Explicit connector forwarding follows ACP transport capabilities. The
//! external runtime owns these clients; native reports are never re-executed.
use super::{error, Result};
use nexa_core::{db::Database, mcp::McpServer};
use serde_json::{json, Value};

pub(super) fn selected(db: &Database, ids: &[String]) -> Result<Vec<McpServer>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let servers = db.list_mcp_servers()?;
    let mut selected = Vec::new();
    for id in ids {
        let server = servers.iter().find(|server| server.id == *id && server.enabled && server.builtin_id.is_none())
            .ok_or_else(|| error("A selected MCP connector is disabled, unavailable, or managed internally by Nexa. Review this profile's connectors."))?;
        if nexa_core::mcp::oauth::McpAuthService::shared(db)
            .status(id)?
            .config
            .is_some()
        {
            return Err(error("This connector uses Nexa-managed OAuth. Use it with a Nexa API agent, or configure OAuth in the external agent's own MCP settings; Nexa does not export account tokens to native ACP processes."));
        }
        if !selected.iter().any(|item: &McpServer| item.id == *id) {
            selected.push(server.clone());
        }
    }
    Ok(selected)
}
pub(super) fn project(servers: &[McpServer], capabilities: &Value) -> Result<Vec<Value>> {
    servers.iter().map(|server| {
        let named_values = |raw: Option<&str>| -> Result<Vec<Value>> {
            nexa_core::mcp::resolve_mcp_config_map("external-agent MCP settings", raw.unwrap_or("{}"))
                .map(|map| map.into_iter().map(|(name, value)| json!({"name":name,"value":value})).collect())
        };
        match server.transport.as_str() {
            "stdio" => Ok(json!({
                "name":format!("nexa-{}",server.id),
                "command":server.command.as_deref().ok_or_else(|| error("MCP command is missing"))?,
                "args":nexa_core::mcp::parse_mcp_args(server.args.as_deref().unwrap_or("[]"))?,
                "env":named_values(server.env_json.as_deref())?
            })),
            "streamable_http" | "sse" => {
                let kind = if server.transport == "sse" { "sse" } else { "http" };
                if capabilities[kind] != true { return Err(error(format!("The external agent does not support the selected MCP {kind} transport"))); }
                Ok(json!({"type":kind,"name":format!("nexa-{}",server.id),"url":server.url.as_deref().ok_or_else(|| error("MCP URL is missing"))?,"headers":named_values(server.headers_json.as_deref())?}))
            }
            _ => Err(error("Unsupported external-agent MCP transport")),
        }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connectors_require_explicit_selection_and_advertised_remote_transport() {
        let db = Database::open_memory().unwrap();
        let input = nexa_core::mcp::SaveMcpServerInput {
            id: None,
            name: "Selected connector".into(),
            transport: "streamable_http".into(),
            command: None,
            args: None,
            url: Some("https://example.invalid/mcp".into()),
            env_json: None,
            headers_json: Some(r#"{"X-Example":"literal value"}"#.into()),
            enabled: true,
        };
        let server = db.save_mcp_server(&input).unwrap();
        assert!(selected(&db, &[]).unwrap().is_empty());
        assert!(selected(&db, &["missing".into()]).is_err());
        let servers = selected(&db, &[server.id.clone(), server.id.clone()]).unwrap();
        assert_eq!(servers.len(), 1);
        assert!(project(&servers, &json!({})).is_err());
        let projected = project(&servers, &json!({"http":true})).unwrap();
        assert_eq!(projected[0]["type"], "http");
        assert_eq!(
            projected[0]["headers"][0],
            json!({"name":"X-Example","value":"literal value"})
        );
        let disabled = nexa_core::mcp::SaveMcpServerInput {
            id: Some(server.id.clone()),
            enabled: false,
            ..input
        };
        db.save_mcp_server(&disabled).unwrap();
        assert!(selected(&db, &[server.id]).is_err());
    }
}
