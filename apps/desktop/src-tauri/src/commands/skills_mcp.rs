use super::conversation::desktop_package_host_snapshot;
use super::*;
use nexa_core::package_host::PackageSurfaceKind;

// ── Skills Commands ─────────────────────────────────────────────────

fn materialize_user_skill_resource(state: &AppState, skill: &Skill) -> Result<(), String> {
    nexa_core::skills::materialize_user_skill_to_directory(
        &state.user_extensions.skills_dir(),
        skill,
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn find_user_skill(state: &AppState, skill_id: &str) -> Result<Option<Skill>, String> {
    state
        .db
        .list_skills()
        .map_err(|error| error.to_string())
        .map(|skills| skills.into_iter().find(|skill| skill.id == skill_id))
}

fn reconcile_user_skill_resource(
    state: &AppState,
    previous: Option<&Skill>,
    next: &Skill,
) -> Result<(), String> {
    if let Some(previous) = previous {
        nexa_core::skills::remove_obsolete_user_skill_resources_from_directory(
            &state.user_extensions.skills_dir(),
            previous,
            next,
        )
        .map_err(|error| error.to_string())?;
    }
    materialize_user_skill_resource(state, next)
}

fn materialize_user_skill_resources_except(
    state: &AppState,
    skills: &[Skill],
    preserved_skill_ids: &[String],
) -> Result<(), String> {
    nexa_core::skills::materialize_user_skills_to_directory_except(
        &state.user_extensions.skills_dir(),
        skills,
        preserved_skill_ids,
    )
    .map_err(|e| e.to_string())
}

fn remove_user_skill_resource(state: &AppState, skill: &Skill) -> Result<(), String> {
    nexa_core::skills::remove_materialized_user_skill_from_directory(
        &state.user_extensions.skills_dir(),
        skill,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_skills_cmd(state: tauri::State<'_, AppState>) -> Result<Vec<Skill>, String> {
    state.db.list_skills().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_skill_cmd(
    state: tauri::State<'_, AppState>,
    mut input: SaveSkillInput,
) -> Result<Skill, String> {
    let previous = match input.id.as_deref() {
        Some(skill_id) => find_user_skill(&state, skill_id)?,
        None => None,
    };
    // The current editor intentionally receives only resource metadata. An
    // ordinary text edit must therefore preserve the server-side bundle.
    if input.resource_bundle.is_empty() {
        if let Some(previous) = &previous {
            input.resource_bundle = previous.resource_bundle.clone();
        }
    }
    let skill = state.db.save_skill(&input).map_err(|e| e.to_string())?;
    reconcile_user_skill_resource(&state, previous.as_ref(), &skill)?;
    Ok(skill)
}

#[tauri::command]
pub fn delete_skill_cmd(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let previous = find_user_skill(&state, &id)?.ok_or_else(|| format!("Skill not found: {id}"))?;
    state.db.delete_skill(&id).map_err(|e| e.to_string())?;
    remove_user_skill_resource(&state, &previous)
}

#[tauri::command]
pub fn toggle_skill_cmd(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .db
        .toggle_skill(&id, enabled)
        .map_err(|e| e.to_string())
}

pub(crate) fn filter_desktop_builtin_skills_by_package_host(
    db: &Database,
    skills: Vec<Skill>,
) -> Result<Vec<Skill>, String> {
    let snapshot = desktop_package_host_snapshot(db)?;
    let visible_skill_ids = snapshot
        .runtime_components()
        .into_iter()
        .filter(|component| component.kind == PackageSurfaceKind::Skill)
        .map(|component| component.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    Ok(skills
        .into_iter()
        .filter(|skill| {
            visible_skill_ids.contains(skill.id.as_str())
                || skill
                    .id
                    .strip_prefix("builtin-")
                    .is_some_and(|slug| visible_skill_ids.contains(slug))
        })
        .collect())
}

#[tauri::command]
pub fn list_builtin_skills_cmd(state: tauri::State<'_, AppState>) -> Result<Vec<Skill>, String> {
    filter_desktop_builtin_skills_by_package_host(
        state.db.as_ref(),
        nexa_core::skills::load_builtin_skills(),
    )
}

#[tauri::command]
pub fn import_skill_from_md_cmd(
    state: tauri::State<'_, AppState>,
    content: String,
) -> Result<Skill, String> {
    let (fm, body) = nexa_core::skills::parse_skill_file(&content).map_err(|e| e.to_string())?;
    let input = SaveSkillInput {
        id: None,
        name: fm.name,
        description: fm.description,
        content: body,
        enabled: true,
        resource_bundle: Vec::new(),
    };
    let skill = state.db.save_skill(&input).map_err(|e| e.to_string())?;
    if skill.enabled {
        materialize_user_skill_resource(&state, &skill)?;
    }
    Ok(skill)
}

/// Parse an editor import without persisting it. This keeps file selection and
/// the explicit Save action as one database write instead of creating a hidden
/// duplicate skill first.
#[tauri::command]
pub fn parse_skill_markdown_cmd(content: String) -> Result<SaveSkillInput, String> {
    let (fm, body) = nexa_core::skills::parse_skill_file(&content).map_err(|e| e.to_string())?;
    Ok(SaveSkillInput {
        id: None,
        name: fm.name,
        description: fm.description,
        content: body,
        enabled: true,
        resource_bundle: Vec::new(),
    })
}

#[tauri::command]
pub fn inspect_skill_install_source_cmd(
    source: String,
) -> Result<Vec<DiscoveredSkillBundle>, String> {
    nexa_core::skills::inspect_skill_install_source(Path::new(&source)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn install_skills_from_source_cmd(
    state: tauri::State<'_, AppState>,
    source: String,
    replace_existing: bool,
    accept_blocked_warnings: bool,
) -> Result<Vec<Skill>, String> {
    nexa_core::skills::install_skills_from_sources(
        &state.db,
        &[std::path::PathBuf::from(source)],
        None,
        replace_existing,
        accept_blocked_warnings,
        &state.user_extensions.skills_dir(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn inspect_skill_install_sources_cmd(
    sources: Vec<String>,
) -> Result<Vec<DiscoveredSkillBundle>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let sources = sources
            .into_iter()
            .map(std::path::PathBuf::from)
            .collect::<Vec<_>>();
        nexa_core::skills::inspect_skill_install_sources(&sources)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn install_skills_from_sources_cmd(
    state: tauri::State<'_, AppState>,
    sources: Vec<String>,
    selection: Vec<nexa_core::skills::SkillInstallSelection>,
    replace_existing: bool,
    accept_blocked_warnings: bool,
) -> Result<Vec<Skill>, String> {
    let db = state.db.clone();
    let destination = state.user_extensions.skills_dir();
    tauri::async_runtime::spawn_blocking(move || {
        let sources = sources
            .into_iter()
            .map(std::path::PathBuf::from)
            .collect::<Vec<_>>();
        nexa_core::skills::install_skills_from_sources(
            &db,
            &sources,
            Some(&selection),
            replace_existing,
            accept_blocked_warnings,
            &destination,
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn discover_skills_in_directory_cmd(
    directory: String,
) -> Result<Vec<DiscoveredSkillBundle>, String> {
    nexa_core::skills::discover_skills_in_directory(Path::new(&directory))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_skills_from_directory_cmd(
    state: tauri::State<'_, AppState>,
    directory: String,
) -> Result<Vec<Skill>, String> {
    nexa_core::skills::install_skills_from_sources(
        &state.db,
        &[std::path::PathBuf::from(directory)],
        None,
        false,
        false,
        &state.user_extensions.skills_dir(),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn export_skill_to_md_cmd(
    state: tauri::State<'_, AppState>,
    skill_id: String,
) -> Result<String, String> {
    // Check built-ins first.
    if let Some(s) = nexa_core::skills::load_builtin_skills()
        .into_iter()
        .find(|s| s.id == skill_id)
    {
        return Ok(nexa_core::skills::export_skill_to_md(&s));
    }
    let skills = state.db.list_skills().map_err(|e| e.to_string())?;
    let skill = skills
        .into_iter()
        .find(|s| s.id == skill_id)
        .ok_or_else(|| format!("Skill not found: {skill_id}"))?;
    Ok(nexa_core::skills::export_skill_to_md(&skill))
}

#[tauri::command]
pub fn scan_skill_content_cmd(
    content: String,
) -> Result<Vec<nexa_core::skills::SkillWarning>, String> {
    Ok(nexa_core::skills::scan_skill_content(&content))
}

#[tauri::command]
pub fn list_skill_change_proposals_cmd(
    state: tauri::State<'_, AppState>,
    status: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<SkillChangeProposal>, String> {
    let parsed_status = status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(SkillProposalStatus::try_from)
        .transpose()
        .map_err(|e| e.to_string())?;
    state
        .db
        .list_skill_change_proposals(parsed_status, limit.unwrap_or(20).min(100) as usize)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn apply_skill_change_proposal_cmd(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<AppliedSkillChange, String> {
    let proposal = state
        .db
        .get_skill_change_proposal(&id)
        .map_err(|e| e.to_string())?;
    let previous = match proposal.skill_id.as_deref() {
        Some(skill_id) => find_user_skill(&state, skill_id)?,
        None => None,
    };
    let applied = state
        .db
        .apply_skill_change_proposal(&id)
        .map_err(|e| e.to_string())?;
    reconcile_user_skill_resource(&state, previous.as_ref(), &applied.skill)?;
    Ok(applied)
}

#[tauri::command]
pub fn reject_skill_change_proposal_cmd(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<SkillChangeProposal, String> {
    state
        .db
        .reject_skill_change_proposal(&id)
        .map_err(|e| e.to_string())
}

// ── MCP Commands ────────────────────────────────────────────────────

fn may_list_mcp_tools(server: &McpServer) -> bool {
    server.enabled
}

#[tauri::command]
pub fn get_user_extension_layout_cmd(
    state: tauri::State<'_, AppState>,
) -> nexa_core::user_extensions::UserExtensionLayoutView {
    state.user_extensions.view()
}

#[tauri::command]
pub fn reload_user_skill_files_cmd(
    state: tauri::State<'_, AppState>,
) -> Result<nexa_core::skills::RegisteredSkillFileSyncReport, String> {
    let report = nexa_core::skills::sync_registered_user_skills_from_directory(
        &state.db,
        &state.user_extensions.skills_dir(),
    )
    .map_err(|error| error.to_string())?;
    let skills = state.db.list_skills().map_err(|error| error.to_string())?;
    materialize_user_skill_resources_except(&state, &skills, &report.preserved_skill_ids)?;
    Ok(report)
}

#[tauri::command]
pub fn prepare_mcp_config_file_cmd(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let path = state.user_extensions.mcp_config_path();
    nexa_core::mcp::config_file::ensure_user_mcp_config(&path)
        .map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn reload_mcp_config_file_cmd(
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
) -> Result<nexa_core::mcp::config_file::McpConfigReloadReport, String> {
    let path = state.user_extensions.mcp_config_path();
    let report = nexa_core::mcp::config_file::reload_user_mcp_config(&state.db, &path)
        .map_err(|error| error.to_string())?;
    let manager = &mcp_state.manager;
    match sync_enabled_mcp_servers(&state.db, manager).await {
        Ok(errors) => {
            for (server_id, error) in errors {
                warn!("Failed to sync MCP connector {server_id} after config reload: {error}");
            }
        }
        Err(error) => warn!("Failed to refresh MCP connectors after config reload: {error}"),
    }
    Ok(report)
}

#[tauri::command]
pub fn list_mcp_servers_cmd(state: tauri::State<'_, AppState>) -> Result<Vec<McpServer>, String> {
    state.db.list_mcp_servers().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_mcp_server_cmd(
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
    input: SaveMcpServerInput,
) -> Result<McpServer, String> {
    let saved = state
        .db
        .save_mcp_server(&input)
        .map_err(|e| e.to_string())?;
    let manager = &mcp_state.manager;
    if saved.enabled {
        if let Err(error) = manager
            .sync_server_from_database(&state.db, &saved.id, Some(DEFAULT_MCP_CALL_TIMEOUT_SECS))
            .await
        {
            warn!(
                "Failed to sync MCP connector {} after save: {error}",
                saved.id
            );
        }
    } else {
        manager.disconnect_server(&saved.id).await.ok();
    }
    if let Err(error) = nexa_core::mcp::oauth::McpAuthService::shared(&state.db)
        .cleanup()
        .await
    {
        warn!("MCP credential cleanup is pending: {error}");
    }
    Ok(saved)
}

#[tauri::command]
pub async fn delete_mcp_server_cmd(
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
    id: String,
) -> Result<(), String> {
    if state
        .db
        .get_mcp_server(&id)
        .map_err(|e| e.to_string())?
        .builtin_id
        .is_some()
    {
        return Err("Cannot delete built-in MCP connector".into());
    }
    nexa_core::mcp::oauth::McpAuthService::shared(&state.db)
        .disconnect(&id, false)
        .await
        .map_err(|e| e.to_string())?;
    state.db.delete_mcp_server(&id).map_err(|e| e.to_string())?;
    let manager = &mcp_state.manager;
    manager.disconnect_server(&id).await.ok();
    Ok(())
}

#[tauri::command]
pub async fn toggle_mcp_server_cmd(
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .db
        .toggle_mcp_server(&id, enabled)
        .map_err(|e| e.to_string())?;

    let manager = &mcp_state.manager;
    if enabled {
        if let Err(error) = manager
            .sync_server_from_database(&state.db, &id, Some(DEFAULT_MCP_CALL_TIMEOUT_SECS))
            .await
        {
            warn!("Failed to sync MCP connector {id} after enable: {error}");
        }
    } else {
        manager.disconnect_server(&id).await.ok();
    }

    if let Err(error) = nexa_core::mcp::oauth::McpAuthService::shared(&state.db)
        .cleanup()
        .await
    {
        warn!("MCP credential cleanup is pending: {error}");
    }

    Ok(())
}

#[tauri::command]
pub async fn test_mcp_server_cmd(
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
    id: String,
) -> Result<Vec<McpToolInfo>, String> {
    let servers = state.db.list_mcp_servers().map_err(|e| e.to_string())?;
    let server = servers
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("MCP server {id} not found"))?;
    if nexa_core::mcp::oauth::McpAuthService::shared(&state.db)
        .status(&id)
        .map_err(|e| e.to_string())?
        .config
        .is_some()
    {
        mcp_state
            .manager
            .sync_server_from_database(&state.db, &id, Some(DEFAULT_MCP_CALL_TIMEOUT_SECS))
            .await
            .map_err(|e| e.to_string())?;
        return mcp_state
            .manager
            .refresh_server(&id)
            .await
            .map_err(|e| e.to_string());
    }
    nexa_core::mcp::McpManager::probe_server(&server, Some(DEFAULT_MCP_CALL_TIMEOUT_SECS))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_mcp_oauth_status_cmd(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<nexa_core::mcp::oauth::OAuthStatus, String> {
    nexa_core::mcp::oauth::McpAuthService::shared(&state.db)
        .status(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn configure_mcp_oauth_cmd(
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
    id: String,
    config: Option<nexa_core::mcp::oauth::OAuthConfig>,
) -> Result<nexa_core::mcp::oauth::OAuthStatus, String> {
    let status = nexa_core::mcp::oauth::McpAuthService::shared(&state.db)
        .configure(&id, config)
        .await
        .map_err(|e| e.to_string())?;
    let _ = mcp_state.manager.disconnect_server(&id).await;
    Ok(status)
}

#[tauri::command]
pub async fn begin_mcp_oauth_cmd(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
    id: String,
) -> Result<nexa_core::mcp::oauth::OAuthStatus, String> {
    use tauri_plugin_shell::ShellExt;
    let service = nexa_core::mcp::oauth::McpAuthService::shared(&state.db);
    let login = service.begin(&id).await.map_err(|e| e.to_string())?;
    let _ = mcp_state.manager.disconnect_server(&id).await;
    #[allow(deprecated)]
    if app.shell().open(&login.authorization_url, None).is_err() {
        let _ = service.disconnect(&id, false).await;
        return Err("The system browser could not open. Start sign-in again from settings.".into());
    }
    service.status(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn disconnect_mcp_oauth_cmd(
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
    id: String,
    revoke_remote: bool,
) -> Result<nexa_core::mcp::oauth::DisconnectReceipt, String> {
    let receipt = nexa_core::mcp::oauth::McpAuthService::shared(&state.db)
        .disconnect(&id, revoke_remote)
        .await
        .map_err(|e| e.to_string())?;
    let _ = mcp_state.manager.disconnect_server(&id).await;
    Ok(receipt)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn test_mcp_server_direct_cmd(
    name: String,
    transport: String,
    command: Option<String>,
    args: Option<String>,
    url: Option<String>,
    env_json: Option<String>,
    headers_json: Option<String>,
) -> Result<Vec<McpToolInfo>, String> {
    let server = McpServer {
        id: "__test__".to_string(),
        name,
        transport,
        command,
        args,
        url,
        env_json,
        headers_json,
        enabled: true,
        created_at: String::new(),
        updated_at: String::new(),
        builtin_id: None,
        oauth_epoch: 0,
    };
    nexa_core::mcp::McpManager::probe_server(&server, Some(DEFAULT_MCP_CALL_TIMEOUT_SECS))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_mcp_tools_cmd(
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
    server_id: String,
) -> Result<Vec<McpToolInfo>, String> {
    let servers = state.db.list_mcp_servers().map_err(|e| e.to_string())?;
    let server = servers
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| format!("MCP server {server_id} not found"))?;
    let manager = &mcp_state.manager;
    // Tool enumeration is a runtime action, not a diagnostic test. Re-read
    // durable activation before consulting the client cache so a stale UI
    // snapshot cannot resurrect a connector disabled by a JSON reload.
    if !may_list_mcp_tools(&server) {
        let _ = manager.disconnect_server(&server_id).await;
        return Err(format!("MCP server {server_id} is disabled"));
    }
    manager
        .sync_server_from_database(&state.db, &server_id, Some(DEFAULT_MCP_CALL_TIMEOUT_SECS))
        .await
        .map_err(|error| error.to_string())?;
    manager
        .refresh_server(&server_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_mcp_content_catalog_cmd(
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
    server_id: String,
) -> Result<nexa_core::mcp::McpCatalogSnapshot, String> {
    let server = state
        .db
        .get_mcp_server(&server_id)
        .map_err(|error| error.to_string())?;
    if !server.enabled {
        return Err("MCP connector is disabled".into());
    }
    mcp_state
        .manager
        .sync_server_from_database(&state.db, &server_id, Some(DEFAULT_MCP_CALL_TIMEOUT_SECS))
        .await
        .map_err(|error| error.to_string())?;
    mcp_state
        .manager
        .refresh_server(&server_id)
        .await
        .map_err(|error| error.to_string())?;
    mcp_state
        .manager
        .catalog_snapshot(&server_id)
        .ok_or_else(|| "MCP content catalog is unavailable".into())
}

#[tauri::command]
pub async fn read_mcp_content_cmd(
    state: tauri::State<'_, AppState>,
    mcp_state: tauri::State<'_, McpManagerState>,
    server_id: String,
    authority_epoch: u64,
    request: nexa_core::mcp::McpContentRequest,
) -> Result<nexa_core::tools::ToolResult, String> {
    let server = state
        .db
        .get_mcp_server(&server_id)
        .map_err(|error| error.to_string())?;
    if !server.enabled {
        return Err("MCP connector is disabled".into());
    }
    mcp_state
        .manager
        .read_content(
            &server_id,
            authority_epoch,
            request,
            None,
            &uuid::Uuid::new_v4().to_string(),
        )
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod mcp_command_tests {
    use super::*;

    fn test_server(enabled: bool, builtin: bool) -> McpServer {
        McpServer {
            id: "test-connector".into(),
            name: "Test Connector".into(),
            transport: "stdio".into(),
            command: Some("test-mcp".into()),
            args: None,
            url: None,
            env_json: None,
            headers_json: None,
            enabled,
            created_at: String::new(),
            updated_at: String::new(),
            builtin_id: builtin.then(|| "builtin-test".into()),
            oauth_epoch: 0,
        }
    }

    #[test]
    fn tool_enumeration_requires_durable_connector_activation() {
        assert!(!may_list_mcp_tools(&test_server(false, false)));
        assert!(!may_list_mcp_tools(&test_server(false, true)));
        assert!(may_list_mcp_tools(&test_server(true, false)));
    }
}
