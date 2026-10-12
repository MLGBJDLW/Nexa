//! Agent-facing appearance management through the durable registry.

use async_trait::async_trait;
use serde::Deserialize;

use crate::appearance::{AppearanceRegistry, BUILTIN_THEME_IDS};
use crate::error::CoreError;
use crate::theme_resource_plugin::ThemeResourcePlugin;

use super::{Tool, ToolCategory, ToolExecutionContext, ToolResult};

pub struct AppearanceTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppearanceArgs {
    action: String,
    #[serde(default)]
    plugin: Option<serde_json::Value>,
    #[serde(default)]
    theme_id: Option<String>,
}

#[async_trait]
impl Tool for AppearanceTool {
    fn name(&self) -> &str {
        "appearance"
    }

    fn description(&self) -> &str {
        "Manage Nexa themes: list, draft, apply, activate, rollback, or remove. Activate/remove accepts an id or unique theme name. For authoring guidance load the built-in themes skill. Themes are declarative and reversible: use semantic colors, managed assets, typography, motion, logo treatment, tagline/statusText/quote, and allowlisted component recipes. Never emit CSS selectors, url(), scripts, remote resources, or security/approval claims. Use draft before apply when authoring a new theme."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "draft", "apply", "activate", "rollback", "remove"] },
                "themeId": { "type": "string", "description": "Built-in id, installed theme id, or unique custom theme name for activate/remove. Exact ids take precedence; ambiguous names are rejected. Use list to inspect names and ids." },
                "plugin": {
                    "type": "object",
                    "description": "Theme resource for draft/apply. manifestVersion may be 1 or 2 and is normalized to 2.",
                    "properties": {
                        "manifestVersion": { "type": "integer", "enum": [1, 2] },
                        "kind": { "const": "theme-resource" },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "theme": {
                            "type": "object",
                            "properties": {
                                "baseTheme": { "type": "string", "enum": ["dark", "light", "midnight", "aurora", "bloom", "dream"] },
                                "mode": { "type": "string", "enum": ["dark", "light"] },
                                "colors": { "type": "object" },
                                "effects": { "type": "object" },
                                "typography": { "type": "object" },
                                "motion": { "type": "object" },
                                "brand": { "type": "object" },
                                "content": {
                                    "type": "object",
                                    "properties": {
                                        "tagline": { "type": "string", "maxLength": 160 },
                                        "statusText": { "type": "string", "maxLength": 80 },
                                        "quote": { "type": "string", "maxLength": 240 }
                                    }
                                },
                                "components": { "type": "object" },
                                "background": { "type": "object" }
                            },
                            "required": ["baseTheme", "mode", "colors", "background"]
                        }
                    },
                    "required": ["manifestVersion", "kind", "id", "name", "theme"]
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::Core]
    }

    fn requires_confirmation(&self, args: &serde_json::Value) -> bool {
        args.get("action")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|action| matches!(action, "apply" | "activate" | "rollback" | "remove"))
    }

    fn confirmation_message(&self, args: &serde_json::Value) -> Option<String> {
        let action = args.get("action")?.as_str()?;
        let theme = args
            .get("themeId")
            .and_then(serde_json::Value::as_str)
            .or_else(|| args.get("plugin")?.get("name")?.as_str())
            .unwrap_or("the selected appearance");
        match action {
            "apply" | "activate" => Some(format!(
                "Apply {theme} to the Nexa interface. This is a local, reversible appearance change."
            )),
            "rollback" => Some(
                "Restore the previous Nexa appearance. This is a local, reversible change."
                    .to_string(),
            ),
            "remove" => Some(format!(
                "Remove custom theme {theme} from Nexa. Managed background cleanup remains controlled by the host."
            )),
            _ => None,
        }
    }

    fn is_read_only(&self, args: &serde_json::Value) -> bool {
        args.get("action")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|action| matches!(action, "list" | "draft"))
    }

    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let args: AppearanceArgs = serde_json::from_str(context.arguments).map_err(|error| {
            CoreError::InvalidInput(format!("Invalid appearance arguments: {error}"))
        })?;
        let action = args.action.trim();
        let (registry, changed) = match action {
            "list" => (context.db.load_appearance_registry()?, false),
            "draft" => {
                let plugin = normalize_plugin(args.plugin, action)?;
                return Ok(result(
                    context.call_id,
                    format!(
                        "Appearance draft validated: {}.\n{}",
                        plugin.name,
                        serde_json::to_string(&plugin)?
                    ),
                    "appearanceDraft",
                    serde_json::json!({ "plugin": plugin }),
                ));
            }
            "apply" => {
                let plugin = normalize_plugin(args.plugin, action)?;
                crate::user_extensions::write_theme_plugin_to_configured_directory(plugin.clone())?;
                (context.db.apply_appearance_plugin(plugin)?, true)
            }
            "activate" => {
                let registry = context.db.load_appearance_registry()?;
                let theme_id = resolve_theme_id(&registry, &args, action)?;
                (context.db.activate_appearance(&theme_id)?, true)
            }
            "rollback" => (context.db.rollback_appearance()?, true),
            "remove" => {
                let registry = context.db.load_appearance_registry()?;
                let theme_id = resolve_theme_id(&registry, &args, action)?;
                // Validate removal before touching the file projection (built-ins cannot be removed).
                registry.remove(&theme_id)?;
                crate::user_extensions::remove_theme_plugin_from_configured_directory(&theme_id)?;
                (context.db.remove_appearance(&theme_id)?, true)
            }
            other => {
                return Err(CoreError::InvalidInput(format!(
                    "Unknown appearance action '{other}'"
                )))
            }
        };
        Ok(result(
            context.call_id,
            if changed {
                format!(
                    "Appearance registry updated. Active theme: {}. Revision: {}.",
                    registry.active_theme_id, registry.revision
                )
            } else {
                format!(
                    "Available Nexa themes:\n{}",
                    serde_json::to_string(&serde_json::json!({
                        "activeThemeId": registry.active_theme_id,
                        "previousThemeId": registry.previous_theme_id,
                        "builtinThemes": BUILTIN_THEME_IDS,
                        "themes": registry.plugins,
                        "revision": registry.revision,
                    }))?
                )
            },
            "appearanceRegistry",
            serde_json::json!({ "registry": registry }),
        ))
    }
}

fn normalize_plugin(
    value: Option<serde_json::Value>,
    action: &str,
) -> Result<ThemeResourcePlugin, CoreError> {
    let value = value.ok_or_else(|| {
        CoreError::InvalidInput(format!(
            "plugin is required for appearance action '{action}'"
        ))
    })?;
    serde_json::from_value::<ThemeResourcePlugin>(value)
        .map_err(|error| CoreError::InvalidInput(format!("Invalid appearance plugin: {error}")))?
        .normalize()
}

fn resolve_theme_id(
    registry: &AppearanceRegistry,
    args: &AppearanceArgs,
    action: &str,
) -> Result<String, CoreError> {
    let selector = args
        .theme_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            CoreError::InvalidInput(format!(
                "themeId is required for appearance action '{action}'"
            ))
        })?;
    if BUILTIN_THEME_IDS.contains(&selector)
        || registry.plugins.iter().any(|plugin| plugin.id == selector)
    {
        return Ok(selector.to_string());
    }
    let matches: Vec<_> = registry
        .plugins
        .iter()
        .filter(|plugin| plugin.name.eq_ignore_ascii_case(selector))
        .collect();
    match matches.as_slice() {
        [plugin] => Ok(plugin.id.clone()),
        [] => Err(CoreError::NotFound(format!(
            "Theme '{selector}'. Use appearance list to find an installed theme."
        ))),
        _ => Err(CoreError::InvalidInput(format!(
            "Theme name '{selector}' is ambiguous; use an exact id: {}",
            matches
                .iter()
                .map(|plugin| plugin.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn result(call_id: &str, content: String, kind: &str, payload: serde_json::Value) -> ToolResult {
    ToolResult {
        call_id: call_id.to_string(),
        content,
        is_error: false,
        artifacts: Some(serde_json::json!({ "kind": kind, "payload": payload })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn plugin() -> serde_json::Value {
        serde_json::json!({
            "manifestVersion": 2,
            "kind": "theme-resource",
            "id": "agent-autumn",
            "name": "Agent Autumn",
            "theme": {
                "baseTheme": "dark",
                "mode": "dark",
                "colors": { "accent": "#d66a3e" },
                "effects": { "densityScale": 0.95 },
                "typography": {},
                "motion": { "cursorStyle": "fluid" },
                "brand": { "logoVariant": "accent" },
                "content": { "tagline": "Warm focus for long sessions" },
                "components": {},
                "background": { "kind": "none" }
            }
        })
    }

    #[tokio::test]
    async fn lists_themes_in_model_visible_content_and_activates_by_name() {
        let db = Database::open_memory().unwrap();
        let theme: ThemeResourcePlugin = serde_json::from_value(plugin()).unwrap();
        db.hydrate_appearance_registry(vec![theme], "dark".into())
            .unwrap();
        let tool = AppearanceTool;
        let listing = tool
            .execute(ToolExecutionContext::new(
                "list",
                r#"{"action":"list"}"#,
                &db,
                &[],
            ))
            .await
            .unwrap();
        assert!(listing.content.contains("Agent Autumn"));
        assert!(listing.content.contains("agent-autumn"));
        assert!(listing.content.contains("dream"));
        tool.execute(ToolExecutionContext::new(
            "activate",
            r#"{"action":"activate","themeId":"Agent Autumn"}"#,
            &db,
            &[],
        ))
        .await
        .unwrap();
        assert_eq!(
            db.load_appearance_registry().unwrap().active_theme_id,
            "agent-autumn"
        );
        tool.execute(ToolExecutionContext::new(
            "rollback",
            r#"{"action":"rollback"}"#,
            &db,
            &[],
        ))
        .await
        .unwrap();
        assert_eq!(
            db.load_appearance_registry().unwrap().active_theme_id,
            "dark"
        );
    }

    #[test]
    fn exact_ids_win_and_duplicate_names_are_rejected() {
        let first: ThemeResourcePlugin = serde_json::from_value(plugin()).unwrap();
        let mut second = first.clone();
        second.id = "autumn-two".into();
        let registry = AppearanceRegistry::default()
            .hydrate(vec![first, second], "dark".into())
            .unwrap();
        let mut args = AppearanceArgs {
            action: "activate".into(),
            plugin: None,
            theme_id: Some("Agent Autumn".into()),
        };
        assert!(resolve_theme_id(&registry, &args, "activate")
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
        args.theme_id = Some("agent-autumn".into());
        assert_eq!(
            resolve_theme_id(&registry, &args, "activate").unwrap(),
            "agent-autumn"
        );
        args.theme_id = Some("missing".into());
        assert!(resolve_theme_id(&registry, &args, "activate").is_err());
    }

    #[tokio::test]
    async fn drafts_then_applies_a_validated_theme() {
        let db = Database::open_memory().unwrap();
        db.hydrate_appearance_registry(Vec::new(), "dark".into())
            .unwrap();
        let tool = AppearanceTool;
        let draft_args = serde_json::json!({ "action": "draft", "plugin": plugin() });
        let draft = tool
            .execute(ToolExecutionContext::new(
                "draft",
                &draft_args.to_string(),
                &db,
                &[],
            ))
            .await
            .unwrap();
        let normalized = draft.artifacts.unwrap();
        assert_eq!(normalized["kind"], "appearanceDraft");
        let visible_plugin: serde_json::Value =
            serde_json::from_str(draft.content.split_once('\n').unwrap().1).unwrap();
        assert_eq!(visible_plugin, normalized["payload"]["plugin"]);

        let apply_args = serde_json::json!({ "action": "apply", "plugin": plugin() });
        assert!(tool.requires_confirmation(&apply_args));
        tool.execute(ToolExecutionContext::new(
            "apply",
            &apply_args.to_string(),
            &db,
            &[],
        ))
        .await
        .unwrap();
        assert_eq!(
            db.load_appearance_registry().unwrap().active_theme_id,
            "agent-autumn"
        );
    }
}
