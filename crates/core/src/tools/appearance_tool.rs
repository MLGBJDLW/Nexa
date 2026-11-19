//! Agent-facing appearance management through the durable registry.

use async_trait::async_trait;
use serde::Deserialize;

use crate::appearance::{AppearanceRegistry, BUILTIN_THEME_IDS};
use crate::error::CoreError;
use crate::theme_resource_plugin::ThemeResourcePlugin;
use unicode_normalization::UnicodeNormalization;

use super::{Tool, ToolCategory, ToolExecutionContext, ToolResult};

pub struct AppearanceTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppearanceArgs {
    action: String,
    #[serde(default)]
    plugin: Option<serde_json::Value>,
    #[serde(default)]
    background_path: Option<String>,
    #[serde(default, alias = "name")]
    theme_id: Option<String>,
}

#[async_trait]
impl Tool for AppearanceTool {
    fn name(&self) -> &str {
        "appearance"
    }

    fn description(&self) -> &str {
        "Manage Nexa themes: list, draft, apply, activate, rollback, or remove. Use a plain name to select a theme. New plugins need only name and theme; the host supplies format defaults and a stable id. For authoring guidance load the built-in themes skill. Themes are declarative and reversible: use semantic colors, managed assets, typography, motion, logo treatment, content, and allowlisted component recipes. Never emit CSS selectors, url(), scripts, remote resources, or security/approval claims. Draft validates without applying."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "draft", "apply", "activate", "rollback", "remove"] },
                "name": { "type": "string", "description": "Plain theme name for activate/remove. Ambiguous names require an explicit themeId." },
                "themeId": { "type": "string", "description": "Built-in id, installed theme id, or unique custom theme name for activate/remove. Exact ids take precedence; ambiguous names are rejected. Use list to inspect names and ids." },
                "backgroundPath": { "type": "string", "description": "For apply only. Import this local image, animated GIF, MP4, or WebM using the conversation's file access, and bind it to plugin.theme.background. Omit kind and assetId when supplying a path. The installed declaration contains only its managed asset id." },
                "plugin": {
                    "type": "object",
                    "description": "Theme resource for draft/apply. Only name and theme are required; format metadata and id are supplied automatically. Existing explicit ids are preserved.",
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
                            "required": []
                        }
                    },
                    "required": ["name", "theme"]
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
            .or_else(|| args.get("name"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| args.get("plugin")?.get("name")?.as_str())
            .unwrap_or("the selected appearance");
        match action {
            "apply" if args.get("backgroundPath").and_then(|path| path.as_str()).is_some() => Some(format!(
                "Use {} as the background of {theme} on Nexa and its paired clients.",
                args["backgroundPath"].as_str()?
            )),
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
        if action != "apply" && args.background_path.is_some() {
            return Err(CoreError::InvalidInput(
                "backgroundPath is only available with appearance apply".into(),
            ));
        }
        let (registry, changed) = match action {
            "list" => (context.db.load_appearance_registry()?, false),
            "draft" => {
                let plugin = normalize_plugin(args.plugin, action, context.db)?;
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
                let plugin = plugin_to_apply(args.plugin, args.background_path, &context).await?;
                let written = crate::user_extensions::write_theme_plugin_to_configured_directory(
                    plugin.clone(),
                )?;
                (
                    if written {
                        context.db.apply_appearance_file_plugin(plugin)?
                    } else {
                        context.db.apply_appearance_plugin(plugin)?
                    },
                    true,
                )
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
                let source = registry.file_source_for_theme(&theme_id).map(str::to_owned);
                // Validate removal before touching the file projection (built-ins cannot be removed).
                registry.remove(&theme_id)?;
                crate::user_extensions::remove_theme_plugin_from_configured_directory_with_source(
                    &theme_id,
                    source.as_deref(),
                )?;
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

async fn plugin_to_apply(
    declaration: Option<serde_json::Value>,
    background_path: Option<String>,
    context: &ToolExecutionContext<'_>,
) -> Result<ThemeResourcePlugin, CoreError> {
    let Some(path) = background_path else {
        return normalize_plugin(declaration, "apply", context.db);
    };
    let source = super::resolve_agent_file_path(context, std::path::Path::new(&path))?;
    let data_root = context
        .db
        .db_path()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| {
            CoreError::InvalidInput(
                "Background import needs a file-backed appearance registry".into(),
            )
        })?
        .to_path_buf();
    let mut declaration = declaration.ok_or_else(|| {
        CoreError::InvalidInput("Supply a theme plugin with backgroundPath".into())
    })?;
    let theme = declaration
        .get_mut("theme")
        .and_then(|theme| theme.as_object_mut())
        .ok_or_else(|| {
            CoreError::InvalidInput("Supply a theme object with backgroundPath".into())
        })?;
    let background = theme
        .entry("background")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| CoreError::InvalidInput("Theme background must be an object".into()))?;
    // Normalize the whole declaration before writing a managed asset.
    background.remove("value");
    background.insert("kind".into(), "none".into());
    background.remove("assetId");
    let mut plugin = normalize_plugin(Some(declaration), "apply", context.db)?;
    let asset = tokio::task::spawn_blocking(move || {
        crate::managed_assets::ManagedLocalAssetStore::new(&data_root, &data_root)
            .import_theme_background(&source)
    })
    .await
    .map_err(|error| CoreError::Internal(error.to_string()))??;
    plugin.theme.background.kind = "image".into();
    if asset.media_type.starts_with("video/") {
        plugin.theme.background.kind = "video".into();
        if plugin.theme.background.fit.as_deref() == Some("tile") {
            plugin.theme.background.fit = Some("cover".into());
        }
    }
    plugin.theme.background.asset_id = Some(asset.asset_id);
    Ok(plugin)
}

fn normalize_plugin(
    value: Option<serde_json::Value>,
    action: &str,
    db: &crate::db::Database,
) -> Result<ThemeResourcePlugin, CoreError> {
    let value = value.ok_or_else(|| {
        CoreError::InvalidInput(format!(
            "plugin is required for appearance action '{action}'"
        ))
    })?;
    let mut plugin = serde_json::from_value::<ThemeResourcePlugin>(value)
        .map_err(|error| CoreError::InvalidInput(format!("Invalid appearance plugin: {error}")))?;
    plugin.name = plugin.name.trim().nfc().collect();
    if plugin.id.trim().is_empty() {
        let registry = db.load_appearance_registry()?;
        let matching = registry
            .plugins
            .iter()
            .filter(|existing| existing.name.eq_ignore_ascii_case(plugin.name.trim()))
            .collect::<Vec<_>>();
        match matching.as_slice() {
            [existing] => plugin.id = existing.id.clone(),
            [] => {}
            _ => {
                return Err(CoreError::InvalidInput(
                    "This theme name is ambiguous; specify the existing id".into(),
                ))
            }
        }
    }
    plugin.normalize()
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
            CoreError::InvalidInput(format!("name is required for appearance action '{action}'"))
        })?;
    let normalized: String = selector.nfc().collect();
    let selector = normalized.as_str();
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
            background_path: None,
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
    async fn applies_a_workspace_video_as_a_portable_managed_background() {
        let root = tempfile::tempdir().unwrap();
        let workspace_path = root.path().join("project");
        std::fs::create_dir(&workspace_path).unwrap();
        let clip = b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isom";
        std::fs::write(workspace_path.join("loop.mp4"), clip).unwrap();
        let db = Database::new(root.path().join("nexa.db")).unwrap();
        let workspace =
            crate::workspace::Workspace::validate(&[workspace_path.to_string_lossy().into_owned()])
                .unwrap();
        let mut context = ToolExecutionContext::new("apply", "", &db, &[]);
        context.workspace = Some(&workspace);
        let plugin = plugin_to_apply(
            Some(serde_json::json!({"name":"Rain","theme":{"background":{"dim":0.35}}})),
            Some("loop.mp4".into()),
            &context,
        )
        .await
        .unwrap();
        assert_eq!(plugin.theme.background.kind, "video");
        assert_eq!(plugin.theme.background.dim, Some(0.35));
        let asset = crate::managed_assets::ManagedLocalAssetStore::new(root.path(), root.path())
            .resolve_theme_background(plugin.theme.background.asset_id.as_deref().unwrap())
            .unwrap();
        assert_eq!(asset.media_type, "video/mp4");
        assert_eq!(std::fs::read(asset.path).unwrap(), clip);
        assert!(!serde_json::to_string(&plugin).unwrap().contains("loop.mp4"));
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
