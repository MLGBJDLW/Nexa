//! Connector-owned tool identity. Model aliases and display names are projections.

use serde::{Deserialize, Serialize};

use super::McpServer;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalToolId {
    pub connector_id: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct McpToolIdentity {
    #[serde(flatten)]
    pub id: CanonicalToolId,
    pub trust_config_digest: String,
}

impl CanonicalToolId {
    pub fn new(connector_id: impl Into<String>, tool_name: impl Into<String>) -> Self {
        Self {
            connector_id: connector_id.into(),
            tool_name: tool_name.into(),
        }
    }

    /// Provider-safe, at most 63 ASCII bytes, independent of display name,
    /// discovery order, transport configuration and other installed tools.
    pub fn model_alias(&self) -> String {
        let material = serde_json::to_vec(&(&self.connector_id, &self.tool_name))
            .expect("string tuple is serializable");
        let digest = blake3::hash(&material).to_hex();
        let label = registry_slug(&self.tool_name, "tool")
            .chars()
            .take(24)
            .collect::<String>();
        format!("mcp__{label}__{}", &digest[..32])
    }
}

impl McpToolIdentity {
    pub fn new(server: &McpServer, tool_name: impl Into<String>) -> Self {
        Self {
            id: CanonicalToolId::new(&server.id, tool_name),
            trust_config_digest: connector_trust_digest(server),
        }
    }

    pub fn permission_target(&self) -> String {
        serde_json::to_string(self).expect("MCP identity is serializable")
    }
}

/// Only execution/credential configuration affects a grant. A display rename
/// does not move the grant; changing an endpoint or launch contract does.
/// Values are hashed, never included verbatim in a permission or tool receipt.
pub fn connector_trust_digest(server: &McpServer) -> String {
    let canonical_map = |raw: &Option<String>| {
        raw.as_deref()
            .and_then(|value| {
                serde_json::from_str::<std::collections::BTreeMap<String, String>>(value).ok()
            })
            .map(|value| serde_json::to_value(value).expect("string map is serializable"))
            .unwrap_or_else(|| serde_json::json!(raw))
    };
    let material = serde_json::json!({
        "transport": server.transport,
        "command": server.command,
        "args": server.args,
        "url": server.url,
        "env": canonical_map(&server.env_json),
        "headers": canonical_map(&server.headers_json),
        "builtinId": server.builtin_id,
    });
    blake3::hash(material.to_string().as_bytes())
        .to_hex()
        .to_string()
}

/// Compatibility selector for existing package declarations. This is never
/// registered, called or used as a permission key.
pub(crate) fn ownership_selector(server_name: &str, tool_name: &str) -> String {
    format!(
        "mcp__{}__{}",
        registry_slug(server_name, "server"),
        registry_slug(tool_name, "tool")
    )
}

pub(crate) fn registry_slug(value: &str, fallback: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | '0'..='9' => ch,
            'A'..='Z' => ch.to_ascii_lowercase(),
            _ => '_',
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_preserve_distinct_exact_names_and_file_connector_ids() {
        let mut aliases = std::collections::HashSet::new();
        for connector in ["user-json:a", "user-json:b", "user-json:c"] {
            for name in ["read.file", "read_file", "READ_FILE", "很长的工具名字"] {
                let id = CanonicalToolId::new(connector, name);
                let alias = id.model_alias();
                assert!(alias.len() <= 63);
                assert!(alias
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'));
                assert!(aliases.insert(alias.clone()));
                assert_eq!(id.model_alias(), alias);
            }
        }
    }
}
