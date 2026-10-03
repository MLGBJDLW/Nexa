//! MCP (Model Context Protocol) module — client, manager, and data models.

pub mod client;
pub mod config_file;
mod events;
pub mod identity;
mod manager;
pub mod result;
pub(crate) use manager::McpConnectorSlot;
pub use manager::{McpCatalogSnapshot, McpManager};

pub use identity::{CanonicalToolId, McpToolIdentity};

#[cfg(test)]
mod integration_tests;

use crate::db::Database;
use crate::error::CoreError;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::TcpListener;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Data models
// ---------------------------------------------------------------------------

/// Persisted MCP connector configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub id: String,
    pub name: String,
    /// Transport type: `"stdio"`, `"sse"`, or `"streamable_http"`.
    pub transport: String,
    pub command: Option<String>,
    /// JSON array string, e.g. `["--port", "8080"]`.
    pub args: Option<String>,
    pub url: Option<String>,
    /// JSON object string for environment variables.
    pub env_json: Option<String>,
    /// JSON object string for HTTP headers.
    pub headers_json: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    /// Non-`None` for built-in servers managed by the app.
    /// Built-in connectors cannot be deleted and have their process lifecycle managed.
    pub builtin_id: Option<String>,
}

/// Input for creating or updating an MCP connector configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SaveMcpServerInput {
    /// `None` = create new, `Some` = update existing.
    pub id: Option<String>,
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    pub args: Option<String>,
    pub url: Option<String>,
    pub env_json: Option<String>,
    pub headers_json: Option<String>,
    pub enabled: bool,
}

/// Tool information returned by an MCP connector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpToolInfo {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// Preserve server-defined metadata while keeping each catalog inside the transport budget.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct McpContentCatalog {
    pub capabilities: serde_json::Value,
    pub resources: Vec<serde_json::Value>,
    pub resource_templates: Vec<serde_json::Value>,
    pub prompts: Vec<serde_json::Value>,
    pub resources_complete: bool,
    pub resource_templates_complete: bool,
    pub prompts_complete: bool,
    pub content_diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpContentRequest {
    ReadResource {
        uri: String,
    },
    GetPrompt {
        name: String,
        arguments: BTreeMap<String, String>,
    },
}

impl McpContentRequest {
    pub(crate) fn validate(&self, catalog: &McpContentCatalog) -> Result<(), CoreError> {
        match self {
            Self::ReadResource { uri } => {
                if catalog.capabilities.get("resources").is_none()
                    || uri.is_empty()
                    || uri.len() > 16_384
                    || Url::parse(uri).is_err()
                {
                    return Err(CoreError::InvalidInput("Choose an absolute resource URI from this connector or expand one of its advertised URI templates.".into()));
                }
            }
            Self::GetPrompt { name, arguments } => {
                if !catalog.prompts_complete {
                    return Err(CoreError::Mcp("Prompt discovery is incomplete; refresh this connector before choosing a template.".into()));
                }
                let prompt = catalog
                    .prompts
                    .iter()
                    .find(|prompt| {
                        prompt.get("name").and_then(serde_json::Value::as_str) == Some(name)
                    })
                    .ok_or_else(|| {
                        CoreError::InvalidInput(
                            "The prompt is absent from this connector's current catalog.".into(),
                        )
                    })?;
                let definitions = prompt
                    .get("arguments")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for definition in &definitions {
                    let key = definition
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    if definition
                        .get("required")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                        && !arguments.contains_key(key)
                    {
                        return Err(CoreError::InvalidInput(format!(
                            "Prompt argument '{key}' is required."
                        )));
                    }
                }
                if arguments.len() > 128
                    || serde_json::to_vec(arguments)?.len() > 64 * 1024
                    || arguments.keys().any(|key| {
                        !definitions.iter().any(|definition| {
                            definition.get("name").and_then(serde_json::Value::as_str) == Some(key)
                        })
                    })
                {
                    return Err(CoreError::InvalidInput(
                        "Prompt arguments must match the current catalog and fit within 64 KiB."
                            .into(),
                    ));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::ReadResource { .. } => "resources/read",
            Self::GetPrompt { .. } => "prompts/get",
        }
    }
}

fn normalize_required_text(field: &str, value: &str) -> Result<String, CoreError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidInput(format!("{field} cannot be empty")));
    }
    Ok(trimmed.to_string())
}

fn normalize_optional_text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn parse_mcp_args(args: &str) -> Result<Vec<String>, CoreError> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if trimmed.starts_with('[') {
        return serde_json::from_str(trimmed).map_err(|e| {
            CoreError::InvalidInput(format!(
                "Invalid args: expected a JSON array of strings, one arg per line, or comma-separated values ({e})"
            ))
        });
    }

    let values = if trimmed.contains('\n') {
        trimmed.lines().map(str::trim).collect::<Vec<_>>()
    } else if trimmed.contains(',') {
        trimmed.split(',').map(str::trim).collect::<Vec<_>>()
    } else {
        trimmed.split_whitespace().collect::<Vec<_>>()
    };

    Ok(values
        .into_iter()
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn normalize_args_json(args: &Option<String>) -> Result<Option<String>, CoreError> {
    let Some(raw_args) = normalize_optional_text(args) else {
        return Ok(None);
    };
    let parsed = parse_mcp_args(&raw_args)?;
    if parsed.is_empty() {
        return Ok(None);
    }
    serde_json::to_string(&parsed)
        .map(Some)
        .map_err(CoreError::from)
}

fn normalize_json_string_map(
    field: &str,
    value: &Option<String>,
) -> Result<Option<String>, CoreError> {
    let Some(raw) = normalize_optional_text(value) else {
        return Ok(None);
    };

    // Keep persisted connector maps canonical. Besides making diffs readable,
    // this avoids treating a key-order-only rewrite as a runtime change.
    let parsed: BTreeMap<String, String> = serde_json::from_str(&raw).map_err(|e| {
        CoreError::InvalidInput(format!(
            "Invalid {field}: expected a JSON object of string values ({e})"
        ))
    })?;

    if parsed.is_empty() {
        return Ok(None);
    }

    if let Some(empty_key) = parsed.keys().find(|key| key.trim().is_empty()) {
        return Err(CoreError::InvalidInput(format!(
            "Invalid {field}: key '{empty_key}' cannot be empty"
        )));
    }

    serde_json::to_string(&parsed)
        .map(Some)
        .map_err(CoreError::from)
}

fn resolve_env_placeholders(value: &str) -> Result<String, CoreError> {
    let mut resolved = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find("${env:") {
        resolved.push_str(&remaining[..start]);
        let placeholder = &remaining[start + 6..];
        let Some(end) = placeholder.find('}') else {
            return Err(CoreError::InvalidInput(
                "Invalid MCP environment reference: missing closing '}'".into(),
            ));
        };
        let variable = &placeholder[..end];
        if variable.is_empty()
            || !variable
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(CoreError::InvalidInput(format!(
                "Invalid MCP environment reference '${{env:{variable}}}'"
            )));
        }
        let secret = std::env::var(variable).map_err(|_| {
            CoreError::InvalidInput(format!(
                "MCP connector requires environment variable '{variable}'"
            ))
        })?;
        resolved.push_str(&secret);
        remaining = &placeholder[end + 1..];
    }
    resolved.push_str(remaining);
    Ok(resolved)
}

pub fn resolve_mcp_config_map(
    field: &str,
    raw: &str,
) -> Result<HashMap<String, String>, CoreError> {
    let values: HashMap<String, String> = serde_json::from_str(raw)
        .map_err(|error| CoreError::InvalidInput(format!("Invalid {field}: {error}")))?;
    values
        .into_iter()
        .map(|(key, value)| resolve_env_placeholders(&value).map(|value| (key, value)))
        .collect()
}

fn normalize_http_url(field: &str, value: &Option<String>) -> Result<Option<String>, CoreError> {
    let Some(raw) = normalize_optional_text(value) else {
        return Ok(None);
    };

    let parsed = Url::parse(&raw).map_err(|e| {
        CoreError::InvalidInput(format!("Invalid {field}: expected an http/https URL ({e})"))
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(Some(parsed.to_string())),
        other => Err(CoreError::InvalidInput(format!(
            "Invalid {field}: expected an http/https URL, got '{other}'"
        ))),
    }
}

fn normalize_save_input(input: &SaveMcpServerInput) -> Result<SaveMcpServerInput, CoreError> {
    let name = normalize_required_text("MCP connector name", &input.name)?;
    let transport = match input.transport.trim() {
        "" => {
            return Err(CoreError::InvalidInput(
                "MCP transport cannot be empty".into(),
            ))
        }
        "stdio" => "stdio".to_string(),
        "sse" => "sse".to_string(),
        "streamable_http" => "streamable_http".to_string(),
        other => {
            return Err(CoreError::InvalidInput(format!(
                "Unsupported MCP transport: {other}. Expected 'stdio', 'sse', or 'streamable_http'."
            )))
        }
    };

    match transport.as_str() {
        "stdio" => {
            let command = normalize_optional_text(&input.command);
            if command.is_none() {
                return Err(CoreError::InvalidInput(
                    "stdio transport requires a command".into(),
                ));
            }
            if normalize_optional_text(&input.url).is_some() {
                return Err(CoreError::InvalidInput(
                    "stdio transport does not use a URL".into(),
                ));
            }
            if normalize_optional_text(&input.headers_json).is_some() {
                return Err(CoreError::InvalidInput(
                    "stdio transport does not use headersJson".into(),
                ));
            }

            Ok(SaveMcpServerInput {
                id: input.id.clone(),
                name,
                transport,
                command,
                args: normalize_args_json(&input.args)?,
                url: None,
                env_json: normalize_json_string_map("envJson", &input.env_json)?,
                headers_json: None,
                enabled: input.enabled,
            })
        }
        "sse" | "streamable_http" => {
            if normalize_optional_text(&input.command).is_some() {
                return Err(CoreError::InvalidInput(format!(
                    "{transport} transport does not use a command"
                )));
            }
            if normalize_optional_text(&input.args).is_some() {
                return Err(CoreError::InvalidInput(format!(
                    "{transport} transport does not use args"
                )));
            }
            if normalize_optional_text(&input.env_json).is_some() {
                return Err(CoreError::InvalidInput(format!(
                    "{transport} transport does not use envJson"
                )));
            }

            let url = normalize_http_url("url", &input.url)?;
            if url.is_none() {
                return Err(CoreError::InvalidInput(format!(
                    "{transport} transport requires a URL"
                )));
            }

            Ok(SaveMcpServerInput {
                id: input.id.clone(),
                name,
                transport,
                command: None,
                args: None,
                url,
                env_json: None,
                headers_json: normalize_json_string_map("headersJson", &input.headers_json)?,
                enabled: input.enabled,
            })
        }
        _ => unreachable!("transport already normalized"),
    }
}

/// Find an available TCP port by binding to port 0.
fn find_free_port() -> Result<u16, CoreError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| CoreError::Mcp(format!("Failed to find free port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| CoreError::Mcp(format!("Failed to get port: {e}")))?
        .port();
    drop(listener);
    Ok(port)
}

fn runtime_config_changed(current: &McpServer, desired: &McpServer) -> bool {
    current.name != desired.name
        || current.transport != desired.transport
        || current.command != desired.command
        || current.args != desired.args
        || current.url != desired.url
        || current.env_json != desired.env_json
        || current.headers_json != desired.headers_json
        || current.builtin_id != desired.builtin_id
}

fn expand_managed_arg(arg: &str, port: u16) -> String {
    arg.replace("${PORT}", &port.to_string())
}

// ---------------------------------------------------------------------------
// Database CRUD
// ---------------------------------------------------------------------------

impl Database {
    /// List all MCP connectors, newest first.
    pub fn list_mcp_servers(&self) -> Result<Vec<McpServer>, CoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, transport, command, args, url, env_json, headers_json,
                    enabled, created_at, updated_at, builtin_id
             FROM mcp_servers
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(McpServer {
                id: row.get(0)?,
                name: row.get(1)?,
                transport: row.get(2)?,
                command: row.get(3)?,
                args: row.get(4)?,
                url: row.get(5)?,
                env_json: row.get(6)?,
                headers_json: row.get(7)?,
                enabled: row.get::<_, i32>(8)? != 0,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                builtin_id: row.get(11)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Create or update an MCP connector configuration.
    pub fn save_mcp_server(&self, input: &SaveMcpServerInput) -> Result<McpServer, CoreError> {
        let input = normalize_save_input(input)?;
        let conn = self.conn();
        let id = match &input.id {
            Some(existing_id) => {
                // Check if the existing server is built-in; if so, block transport/command/args changes.
                let existing_builtin_id: Option<String> = conn
                    .query_row(
                        "SELECT builtin_id FROM mcp_servers WHERE id = ?1",
                        rusqlite::params![existing_id],
                        |row| row.get(0),
                    )
                    .ok();

                if existing_builtin_id.is_some() {
                    // For built-in servers, only allow toggling name/enabled/url/headers — not transport/command/args.
                    conn.execute(
                        "UPDATE mcp_servers
                         SET name = ?2, url = ?3, headers_json = ?4,
                             enabled = ?5, updated_at = datetime('now')
                         WHERE id = ?1",
                        rusqlite::params![
                            existing_id,
                            &input.name,
                            &input.url,
                            &input.headers_json,
                            input.enabled as i32,
                        ],
                    )?;
                } else {
                    conn.execute(
                        "UPDATE mcp_servers
                         SET name = ?2, transport = ?3, command = ?4, args = ?5,
                             url = ?6, env_json = ?7, headers_json = ?8,
                             enabled = ?9, updated_at = datetime('now')
                         WHERE id = ?1",
                        rusqlite::params![
                            existing_id,
                            &input.name,
                            &input.transport,
                            &input.command,
                            &input.args,
                            &input.url,
                            &input.env_json,
                            &input.headers_json,
                            input.enabled as i32,
                        ],
                    )?;
                }
                existing_id.clone()
            }
            None => {
                let new_id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO mcp_servers (id, name, transport, command, args, url, env_json, headers_json, enabled)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        &new_id,
                        &input.name,
                        &input.transport,
                        &input.command,
                        &input.args,
                        &input.url,
                        &input.env_json,
                        &input.headers_json,
                        input.enabled as i32,
                    ],
                )?;
                new_id
            }
        };
        drop(conn);
        self.get_mcp_server(&id)
    }

    /// Delete an MCP connector by ID.
    pub fn delete_mcp_server(&self, id: &str) -> Result<(), CoreError> {
        let conn = self.conn();
        // Prevent deletion of built-in connectors.
        let builtin: Option<String> = conn
            .query_row(
                "SELECT builtin_id FROM mcp_servers WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .map_err(|_| CoreError::NotFound(format!("MCP connector {id}")))?;
        if builtin.is_some() {
            return Err(CoreError::InvalidInput(
                "Cannot delete built-in MCP connector".into(),
            ));
        }
        let affected = conn.execute(
            "DELETE FROM mcp_servers WHERE id = ?1",
            rusqlite::params![id],
        )?;
        if affected == 0 {
            return Err(CoreError::NotFound(format!("MCP connector {id}")));
        }
        Ok(())
    }

    /// Toggle an MCP connector's enabled state.
    pub fn toggle_mcp_server(&self, id: &str, enabled: bool) -> Result<(), CoreError> {
        let conn = self.conn();
        let affected = conn.execute(
            "UPDATE mcp_servers SET enabled = ?2, updated_at = datetime('now') WHERE id = ?1",
            rusqlite::params![id, enabled as i32],
        )?;
        if affected == 0 {
            return Err(CoreError::NotFound(format!("MCP connector {id}")));
        }
        Ok(())
    }

    /// Get only enabled MCP connectors.
    pub fn get_enabled_mcp_servers(&self) -> Result<Vec<McpServer>, CoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, transport, command, args, url, env_json, headers_json,
                    enabled, created_at, updated_at, builtin_id
             FROM mcp_servers
             WHERE enabled = 1
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(McpServer {
                id: row.get(0)?,
                name: row.get(1)?,
                transport: row.get(2)?,
                command: row.get(3)?,
                args: row.get(4)?,
                url: row.get(5)?,
                env_json: row.get(6)?,
                headers_json: row.get(7)?,
                enabled: true,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                builtin_id: row.get(11)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn get_mcp_server(&self, id: &str) -> Result<McpServer, CoreError> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, name, transport, command, args, url, env_json, headers_json,
                    enabled, created_at, updated_at, builtin_id
             FROM mcp_servers
             WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(McpServer {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    transport: row.get(2)?,
                    command: row.get(3)?,
                    args: row.get(4)?,
                    url: row.get(5)?,
                    env_json: row.get(6)?,
                    headers_json: row.get(7)?,
                    enabled: row.get::<_, i32>(8)? != 0,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                    builtin_id: row.get(11)?,
                })
            },
        )
        .map_err(|_| CoreError::NotFound(format!("MCP connector {id}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener as TokioTcpListener;

    #[test]
    fn parse_mcp_args_accepts_json_array() {
        let parsed =
            parse_mcp_args(r#"["-y","@modelcontextprotocol/server-filesystem","D:/vault"]"#)
                .unwrap();
        assert_eq!(
            parsed,
            vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-filesystem".to_string(),
                "D:/vault".to_string()
            ]
        );
    }

    #[test]
    fn environment_placeholders_resolve_without_persisting_secret_values() {
        let path = std::env::var("PATH").expect("PATH is available in the test environment");
        assert_eq!(
            resolve_env_placeholders("Bearer ${env:PATH}").unwrap(),
            format!("Bearer {path}")
        );

        let error = resolve_env_placeholders("${env:NEXA_MCP_MISSING_ENV_TEST_9F31}")
            .unwrap_err()
            .to_string();
        assert!(error.contains("NEXA_MCP_MISSING_ENV_TEST_9F31"));
        assert!(!error.contains(&path));
    }

    #[test]
    fn parse_mcp_args_accepts_legacy_text_formats() {
        assert_eq!(
            parse_mcp_args("-y, @modelcontextprotocol/server-filesystem, D:/vault").unwrap(),
            vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-filesystem".to_string(),
                "D:/vault".to_string()
            ]
        );
        assert_eq!(
            parse_mcp_args("-y\n@modelcontextprotocol/server-filesystem\nD:/vault").unwrap(),
            vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-filesystem".to_string(),
                "D:/vault".to_string()
            ]
        );
    }

    #[test]
    fn save_mcp_server_rejects_unknown_transport() {
        let db = Database::open_memory().unwrap();
        let result = db.save_mcp_server(&SaveMcpServerInput {
            id: None,
            name: "Remote".into(),
            transport: "websocket".into(),
            command: None,
            args: None,
            url: Some("http://localhost:8080/mcp".into()),
            env_json: None,
            headers_json: None,
            enabled: true,
        });
        assert!(result.is_err());
    }

    #[test]
    fn save_mcp_server_normalizes_remote_transport() {
        let db = Database::open_memory().unwrap();
        let server = db
            .save_mcp_server(&SaveMcpServerInput {
                id: None,
                name: "Remote".into(),
                transport: "streamable_http".into(),
                command: None,
                args: None,
                url: Some("https://example.com/mcp".into()),
                env_json: None,
                headers_json: Some(r#"{"Authorization":"Bearer token"}"#.into()),
                enabled: true,
            })
            .unwrap();

        assert_eq!(server.transport, "streamable_http");
        assert_eq!(server.url.as_deref(), Some("https://example.com/mcp"));
        assert_eq!(
            server.headers_json.as_deref(),
            Some(r#"{"Authorization":"Bearer token"}"#)
        );
        assert_eq!(server.command, None);
        assert_eq!(server.args, None);
        assert_eq!(server.env_json, None);
    }

    #[test]
    fn save_mcp_server_requires_url_for_remote_transport() {
        let db = Database::open_memory().unwrap();
        let result = db.save_mcp_server(&SaveMcpServerInput {
            id: None,
            name: "Remote".into(),
            transport: "sse".into(),
            command: None,
            args: None,
            url: None,
            env_json: None,
            headers_json: None,
            enabled: true,
        });

        assert!(result.is_err());
    }

    #[test]
    fn save_mcp_server_normalizes_args_and_env() {
        let db = Database::open_memory().unwrap();
        let server = db
            .save_mcp_server(&SaveMcpServerInput {
                id: None,
                name: "Filesystem".into(),
                transport: "stdio".into(),
                command: Some("npx".into()),
                args: Some("-y, @modelcontextprotocol/server-filesystem, D:/vault".into()),
                url: None,
                env_json: Some(r#"{"API_KEY":"secret"}"#.into()),
                headers_json: None,
                enabled: true,
            })
            .unwrap();

        assert_eq!(
            server.args.as_deref(),
            Some(r#"["-y","@modelcontextprotocol/server-filesystem","D:/vault"]"#)
        );
        assert_eq!(server.env_json.as_deref(), Some(r#"{"API_KEY":"secret"}"#));
    }

    #[test]
    fn mcp_registry_slug_normalizes_names() {
        assert_eq!(
            identity::registry_slug("Web Search", "server"),
            "web_search"
        );
        assert_eq!(
            identity::registry_slug("search.query", "tool"),
            "search_query"
        );
        assert_eq!(identity::registry_slug("!!!", "tool"), "tool");
    }

    #[tokio::test]
    async fn disconnecting_a_live_server_advances_registry_generation() {
        let connector = start_test_connector("disconnect", false).await;
        let manager = McpManager::new();
        manager
            .connect_server(&connector.server, Some(5))
            .await
            .unwrap();
        let before = manager.registry_generation();
        manager
            .disconnect_server(&connector.server.id)
            .await
            .unwrap();
        assert!(manager.registry_generation() > before);
        assert!(manager.catalog_snapshot(&connector.server.id).is_none());
    }

    #[tokio::test]
    async fn failed_catalog_refresh_invalidates_snapshot_without_losing_diagnostics() {
        let connector = start_test_connector("catalog-failure", false).await;
        let manager = McpManager::new();
        manager
            .connect_server(&connector.server, Some(5))
            .await
            .unwrap();
        connector.fail_listing.store(true, Ordering::SeqCst);
        let before = manager.registry_generation();
        assert!(manager.refresh_server(&connector.server.id).await.is_err());
        let snapshot = manager.catalog_snapshot(&connector.server.id).unwrap();
        assert!(!snapshot.complete);
        assert!(!snapshot.tools.is_empty());
        assert!(snapshot.diagnostics.is_some());
        assert!(manager.registry_generation() > before);
        manager.shutdown().await;
    }

    async fn read_test_http_request(
        stream: &mut tokio::net::TcpStream,
    ) -> std::io::Result<(String, serde_json::Value)> {
        let mut bytes = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 4096];
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "request closed before headers",
                ));
            }
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let method = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().next())
            .unwrap_or_default()
            .to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let mut chunk = [0_u8; 4096];
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        let payload = if content_length == 0 {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap()
        };
        Ok((method, payload))
    }

    async fn write_test_http_response(
        stream: &mut tokio::net::TcpStream,
        status: &str,
        payload: Option<&serde_json::Value>,
    ) -> std::io::Result<()> {
        let body = payload
            .map(|value| serde_json::to_vec(value).map_err(std::io::Error::other))
            .transpose()?
            .unwrap_or_default();
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(headers.as_bytes()).await?;
        stream.write_all(&body).await?;
        stream.flush().await
    }

    struct TestConnector {
        server: McpServer,
        fail_listing: Arc<AtomicBool>,
        initialize_calls: Arc<AtomicUsize>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestConnector {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn start_test_connector(name: &str, tool_is_error: bool) -> TestConnector {
        let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let fail_listing = Arc::new(AtomicBool::new(false));
        let initialize_calls = Arc::new(AtomicUsize::new(0));
        let server_fail_listing = Arc::clone(&fail_listing);
        let server_initialize_calls = Arc::clone(&initialize_calls);
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let fail_listing = Arc::clone(&server_fail_listing);
                let initialize_calls = Arc::clone(&server_initialize_calls);
                tokio::spawn(async move {
                    let (http_method, request) = read_test_http_request(&mut stream).await.unwrap();
                    if http_method == "DELETE" {
                        write_test_http_response(&mut stream, "204 No Content", None)
                            .await
                            .unwrap();
                        return;
                    }
                    let id = request.get("id").cloned().unwrap_or_default();
                    let result = match request["method"].as_str().unwrap_or_default() {
                        "initialize" => {
                            initialize_calls.fetch_add(1, Ordering::SeqCst);
                            serde_json::json!({
                                "protocolVersion": "2025-11-25",
                                "capabilities": {},
                                "serverInfo": { "name": "test", "version": "1.0.0" }
                            })
                        }
                        "notifications/initialized" => {
                            write_test_http_response(&mut stream, "202 Accepted", None)
                                .await
                                .unwrap();
                            return;
                        }
                        "tools/list" => {
                            if fail_listing.load(Ordering::SeqCst) {
                                let response = serde_json::json!({
                                    "jsonrpc": "2.0", "id": id,
                                    "error": { "code": -32603, "message": "Discovery unavailable" }
                                });
                                write_test_http_response(&mut stream, "200 OK", Some(&response))
                                    .await
                                    .unwrap();
                                return;
                            }
                            serde_json::json!({
                                "tools": [{
                                    "name": "demo",
                                    "inputSchema": { "type": "object", "properties": {} }
                                }]
                            })
                        }
                        "tools/call" => serde_json::json!({
                            "isError": tool_is_error,
                            "content": [{ "type": "text", "text": "connector result" }]
                        }),
                        other => panic!("unexpected MCP method {other}"),
                    };
                    let response =
                        serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
                    write_test_http_response(&mut stream, "200 OK", Some(&response))
                        .await
                        .unwrap();
                });
            }
        });
        TestConnector {
            server: McpServer {
                id: name.to_string(),
                name: name.to_string(),
                transport: "streamable_http".into(),
                command: None,
                args: None,
                url: Some(format!("http://{address}/mcp")),
                env_json: None,
                headers_json: None,
                enabled: true,
                created_at: String::new(),
                updated_at: String::new(),
                builtin_id: None,
            },
            fail_listing,
            initialize_calls,
            task,
        }
    }

    #[tokio::test]
    async fn discovery_failure_preserves_other_connectors_in_registry() {
        let connectors = [
            start_test_connector("alpha", false).await,
            start_test_connector("beta", false).await,
        ];
        let manager = McpManager::new();
        for connector in &connectors {
            // Discovery failure is injected as a JSON-RPC error, not a timer.
            // Leave headroom for the healthy server during parallel DB fixtures.
            manager
                .connect_server(&connector.server, Some(20))
                .await
                .unwrap();
        }
        // Fail whichever connector is visited first, independently of HashMap order.
        let failed_id = connectors[0].server.id.clone();
        let failed = connectors
            .iter()
            .find(|connector| connector.server.id == failed_id)
            .unwrap();
        let healthy = connectors
            .iter()
            .find(|connector| connector.server.id != failed_id)
            .unwrap();
        failed.fail_listing.store(true, Ordering::SeqCst);
        let generation = manager.registry_generation();
        let mut registry = ToolRegistry::new();

        assert!(manager.refresh_server(&failed.server.id).await.is_err());
        let result = manager.register_tools(&mut registry);

        assert!(
            result.is_err(),
            "incomplete discovery must not be cached as complete"
        );
        assert!(!registry.contains(&CanonicalToolId::new(&failed.server.id, "demo").model_alias()));
        let healthy_tool = registry
            .get(&CanonicalToolId::new(&healthy.server.id, "demo").model_alias())
            .expect("a failing connector must not hide another connector's tools");
        let db = Database::open_memory().unwrap();
        let output = healthy_tool
            .execute(crate::tools::ToolExecutionContext::new(
                "healthy-call",
                "{}",
                &db,
                &[],
            ))
            .await
            .unwrap();
        assert!(
            !output.is_error,
            "healthy connector failed: {}",
            output.content
        );
        assert_eq!(output.content, "connector result");
        assert!(manager.registry_generation() > generation);
        assert!(
            !manager
                .catalog_snapshot(&failed.server.id)
                .unwrap()
                .complete
        );
        assert!(
            manager
                .catalog_snapshot(&healthy.server.id)
                .unwrap()
                .complete
        );
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn tool_result_is_error_is_preserved_without_transport_recovery() {
        let connector = start_test_connector("remote", true).await;
        let manager = Arc::new(McpManager::new());
        let mut registry = ToolRegistry::new();
        let generation = {
            let guard = &manager;
            guard
                .connect_server(&connector.server, Some(2))
                .await
                .unwrap();
            guard.register_tools(&mut registry).unwrap();
            guard.registry_generation()
        };
        let db = Database::open_memory().unwrap();
        let output = registry
            .get(&CanonicalToolId::new(&connector.server.id, "demo").model_alias())
            .unwrap()
            .execute(crate::tools::ToolExecutionContext::new(
                "error-call",
                "{}",
                &db,
                &[],
            ))
            .await
            .unwrap();

        assert!(
            output.is_error,
            "MCP result.isError must remain a tool failure"
        );
        assert!(output.content.contains("connector result"));
        assert!(!output.content.contains("recovery"));
        let guard = &manager;
        assert_eq!(guard.registry_generation(), generation);
        assert!(
            guard
                .catalog_snapshot(&connector.server.id)
                .unwrap()
                .complete
        );
        assert_eq!(connector.initialize_calls.load(Ordering::SeqCst), 1);
        guard.shutdown().await;
    }

    #[tokio::test]
    async fn only_transport_failure_recovers_connection_for_the_next_call() {
        let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let tool_calls = Arc::new(AtomicUsize::new(0));
        let initialize_calls = Arc::new(AtomicUsize::new(0));
        let recovery_started = Arc::new(tokio::sync::Semaphore::new(0));
        let recovery_release = Arc::new(tokio::sync::Semaphore::new(0));
        let server_calls = Arc::clone(&tool_calls);
        let server_initializes = Arc::clone(&initialize_calls);
        let server_recovery_started = Arc::clone(&recovery_started);
        let server_recovery_release = Arc::clone(&recovery_release);
        let server_task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let tool_calls = Arc::clone(&server_calls);
                let initialize_calls = Arc::clone(&server_initializes);
                let recovery_started = Arc::clone(&server_recovery_started);
                let recovery_release = Arc::clone(&server_recovery_release);
                tokio::spawn(async move {
                    let (http_method, request) = read_test_http_request(&mut stream).await.unwrap();
                    if http_method == "DELETE" {
                        write_test_http_response(&mut stream, "204 No Content", None)
                            .await
                            .unwrap();
                        return;
                    }
                    let method = request
                        .get("method")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let id = request
                        .get("id")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    match method {
                        "initialize" => {
                            if initialize_calls.fetch_add(1, Ordering::SeqCst) > 0 {
                                recovery_started.add_permits(1);
                                let _permit = recovery_release.acquire().await.unwrap();
                            }
                            let response = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "protocolVersion": "2025-11-25",
                                    "capabilities": {},
                                    "serverInfo": { "name": "remote", "version": "1.0.0" }
                                }
                            });
                            write_test_http_response(&mut stream, "200 OK", Some(&response))
                                .await
                                .unwrap();
                        }
                        "notifications/initialized" => {
                            write_test_http_response(&mut stream, "202 Accepted", None)
                                .await
                                .unwrap();
                        }
                        "tools/list" => {
                            let response = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "tools": [{
                                        "name": "demo",
                                        "description": "Demo tool",
                                        "inputSchema": { "type": "object", "properties": {} }
                                    }]
                                }
                            });
                            write_test_http_response(&mut stream, "200 OK", Some(&response))
                                .await
                                .unwrap();
                        }
                        "tools/call" => match tool_calls.fetch_add(1, Ordering::SeqCst) {
                            0 => {
                                let response = serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": {
                                        "code": -32602,
                                        "message": "Invalid tool arguments"
                                    }
                                });
                                write_test_http_response(&mut stream, "200 OK", Some(&response))
                                    .await
                                    .unwrap();
                            }
                            1 => {
                                write_test_http_response(
                                    &mut stream,
                                    "500 Internal Server Error",
                                    Some(&serde_json::json!({ "error": "connection lost" })),
                                )
                                .await
                                .unwrap();
                            }
                            _ => {
                                let response = serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "content": [{ "type": "text", "text": "recovered" }]
                                    }
                                });
                                write_test_http_response(&mut stream, "200 OK", Some(&response))
                                    .await
                                    .unwrap();
                            }
                        },
                        other => panic!("unexpected MCP method {other}"),
                    }
                });
            }
        });

        let manager = Arc::new(McpManager::new());
        let server = McpServer {
            id: "server-1".into(),
            name: "Remote".into(),
            transport: "streamable_http".into(),
            command: None,
            args: None,
            url: Some(format!("http://{address}/mcp")),
            env_json: None,
            headers_json: None,
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
            builtin_id: None,
        };
        let mut registry = ToolRegistry::new();
        {
            let guard = &manager;
            guard.connect_server(&server, Some(5)).await.unwrap();
            guard.register_tools(&mut registry).unwrap();
        }
        let db = Database::open_memory().unwrap();
        let source_scope = Vec::new();
        let tool = registry
            .get(&CanonicalToolId::new(&server.id, "demo").model_alias())
            .unwrap();
        let application_error = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "call-1",
                "{}",
                &db,
                &source_scope,
            ))
            .await
            .unwrap();
        assert!(application_error.is_error);
        assert!(application_error.content.contains("Invalid tool arguments"));
        assert!(!application_error.content.contains("recovery"));
        assert_eq!(initialize_calls.load(Ordering::SeqCst), 1);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), recovery_started.acquire())
                .await
                .is_err()
        );

        let transport_error = tokio::time::timeout(
            Duration::from_millis(250),
            tool.execute(crate::tools::ToolExecutionContext::new(
                "call-2",
                "{}",
                &db,
                &source_scope,
            )),
        )
        .await
        .expect("the original tool failure must not wait for reconnect")
        .unwrap();
        assert!(transport_error.is_error);
        assert!(transport_error
            .content
            .contains("recovery scheduled for subsequent calls"));
        tokio::time::timeout(Duration::from_secs(1), recovery_started.acquire())
            .await
            .expect("background recovery starts")
            .unwrap()
            .forget();
        recovery_release.add_permits(1);

        let second = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "call-3",
                "{}",
                &db,
                &source_scope,
            ))
            .await
            .unwrap();
        assert!(!second.is_error);
        assert_eq!(second.content, "recovered");
        assert_eq!(tool_calls.load(Ordering::SeqCst), 3);

        server_task.abort();
    }

    #[test]
    fn expand_managed_arg_replaces_port_placeholder() {
        assert_eq!(expand_managed_arg("--port=${PORT}", 8931), "--port=8931");
        assert_eq!(expand_managed_arg("static", 8931), "static");
    }
}
