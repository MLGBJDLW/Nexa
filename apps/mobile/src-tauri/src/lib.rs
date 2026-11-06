mod connections;
mod local;
mod phone_host;
mod platform;
mod remote;
mod speech;
mod web_settings;

use local::{LocalRuntime, LocalSettings};

#[derive(Default)]
struct RuntimeSlot(tokio::sync::OnceCell<Arc<LocalRuntime>>);
impl RuntimeSlot {
    async fn get(&self, app: &AppHandle) -> Result<&Arc<LocalRuntime>, String> {
        self.0
            .get_or_try_init(|| async {
                let key = match platform::read_secret(app, "settings-key").await? {
                    Some(value) => value.as_str().ok_or("Invalid settings key")?.to_string(),
                    None => {
                        let key = nexa_core::crypto::generate_host_key();
                        platform::save_secret(app, "settings-key", Some(&json!(key))).await?;
                        key
                    }
                };
                nexa_core::crypto::install_host_key(&key).map_err(|e| e.to_string())?;
                LocalRuntime::open(app, app.path().app_data_dir().map_err(|e| e.to_string())?).await
            })
            .await
    }
}
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

#[tauri::command]
async fn mobile_state(app: AppHandle, runtime: State<'_, RuntimeSlot>) -> Result<Value, String> {
    runtime.get(&app).await?;
    let mut state = connections::public_state(&app).await?;
    state["version"] = json!(env!("CARGO_PKG_VERSION"));
    state["hosts"] = json!(remote::hosts(&app).await?);
    state["capabilities"] = platform::call(&app, "capabilities", json!({})).await?;
    Ok(state)
}

#[tauri::command]
async fn mobile_save_settings(
    app: AppHandle,
    settings: LocalSettings,
    api_key: String,
    name: Option<String>,
    create_new: Option<bool>,
) -> Result<(), String> {
    connections::save_settings(&app, settings, api_key, name, create_new.unwrap_or(false)).await
}

#[tauri::command]
async fn mobile_connection(app: AppHandle, action: String, id: String) -> Result<(), String> {
    match action.as_str() {
        "select" => connections::select(&app, &id).await,
        "remove" => connections::remove(&app, &id).await,
        _ => Err("Unsupported model connection operation.".into()),
    }
}

#[tauri::command]
async fn mobile_sync_connections(
    app: AppHandle,
    host_id: String,
) -> Result<connections::SyncSummary, String> {
    connections::synchronize(&app, &host_id).await
}

#[tauri::command]
async fn mobile_local(
    app: AppHandle,
    runtime: State<'_, RuntimeSlot>,
    action: String,
    input: Value,
) -> Result<Value, String> {
    let state = runtime.get(&app).await?;
    let string = |key: &str| {
        input[key]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("Missing {key}."))
    };
    match action.as_str() {
        "connections" => {
            let library = connections::public_state(&app).await?;
            let mut records = library["connections"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for record in &mut records {
                record["isDefault"] = json!(record["id"] == library["activeConnectionId"]);
            }
            Ok(json!(records))
        }
        "models" => {
            let id = string("connectionId")?;
            let saved = connections::get(&app, Some(&id)).await?;
            serde_json::to_value(
                nexa_core::model_choices::cloud_model_choices(
                    id,
                    &saved.provider,
                    &saved.model,
                    connections::provider_config(&saved)?,
                )
                .await
                .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())
        }
        "list" => state
            .db
            .remote_conversations(input["before"].as_str())
            .map_err(|e| e.to_string()),
        "archived" => state
            .db
            .remote_archived_conversations(input["before"].as_str())
            .map_err(|e| e.to_string()),
        "restore" => {
            let id = string("conversationId")?;
            state
                .db
                .require_remote_conversation(&id)
                .map_err(|e| e.to_string())?;
            state
                .db
                .unarchive_conversation(&id)
                .map(|_| Value::Null)
                .map_err(|e| e.to_string())
        }
        "messages" => state
            .db
            .remote_messages(&string("conversationId")?, input["beforeOrder"].as_i64())
            .map_err(|e| e.to_string()),
        "thinking" => state
            .db
            .remote_message_thinking(
                &string("conversationId")?,
                &string("messageId")?,
                input["offset"]
                    .as_u64()
                    .unwrap_or(0)
                    .try_into()
                    .map_err(|_| "Invalid message offset.")?,
            )
            .map_err(|e| e.to_string()),
        "message" => state
            .db
            .remote_message_text(
                &string("conversationId")?,
                &string("messageId")?,
                input["offset"]
                    .as_u64()
                    .unwrap_or(0)
                    .try_into()
                    .map_err(|_| "Invalid message offset.")?,
            )
            .map_err(|e| e.to_string()),
        "currentRun" => {
            let page = state
                .db
                .remote_run_page(&string("conversationId")?, None, 0, None)
                .map_err(|e| e.to_string())?;
            let run = &page["run"];
            let checkpoint = if run["status"] == "paused" {
                state
                    .db
                    .latest_task_resume_checkpoint(run["id"].as_str().ok_or("Missing paused run.")?)
                    .map_err(|e| e.to_string())?
            } else {
                None
            };
            Ok(
                json!({"run":run,"checkpoint":checkpoint.map(|checkpoint|json!({"id":checkpoint.id}))}),
            )
        }
        "approvals" => state.approvals(&string("conversationId")?).await,
        "archive" | "delete" | "deleteArchived" => {
            let ids = [string("conversationId")?];
            state
                .db
                .require_remote_conversation(&ids[0])
                .map_err(|e| e.to_string())?;
            let result = if action == "archive" {
                state.db.archive_conversations_batch(&ids)
            } else if action == "deleteArchived" {
                state.db.delete_archived_conversations_batch(&ids)
            } else {
                state.db.delete_conversations_batch(&ids)
            };
            result.map(|count| json!(count)).map_err(|e| e.to_string())
        }
        "create" => {
            let mut saved = connections::get(&app, input["connectionId"].as_str()).await?;
            if !input["modelSelection"].is_null() {
                let selection: nexa_core::conversation::TurnModelSelection =
                    serde_json::from_value(input["modelSelection"].clone())
                        .map_err(|e| e.to_string())?;
                selection
                    .apply_to_shared(&mut saved)
                    .map_err(|e| e.to_string())?;
            }
            state.create(&connections::settings(&saved))
        }
        "start" => {
            state
                .start(
                    app,
                    input["conversationId"].as_str().map(str::to_owned),
                    string("message")?,
                    string("requestId")?,
                    input["checkpointId"].as_str().map(str::to_owned),
                    serde_json::from_value(
                        input.get("options").cloned().unwrap_or_else(|| json!({})),
                    )
                    .map_err(|e| e.to_string())?,
                )
                .await
        }
        "stop" => {
            state.stop().await;
            Ok(Value::Null)
        }
        "events" => state
            .db
            .remote_run_page(
                &string("conversationId")?,
                input["runId"].as_str(),
                input["afterSequence"].as_u64().unwrap_or(0),
                input["durableHighWater"].as_u64(),
            )
            .map_err(|e| e.to_string()),
        "approve" => {
            state
                .respond(
                    &string("requestId")?,
                    input["allow"]
                        .as_bool()
                        .ok_or("Select Allow once or Deny.")?,
                )
                .await?;
            Ok(Value::Null)
        }
        _ => Err("Unsupported phone task operation.".into()),
    }
}

fn validate_provider_url(input: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(input).map_err(|_| "Enter a valid provider URL.")?;
    let loopback = url
        .host_str()
        .is_some_and(|host| host == "localhost" || host == "127.0.0.1" || host == "[::1]");
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err("Use HTTPS for cloud providers. HTTP is allowed only for a model server on this phone's loopback address.".into());
    }
    Ok(())
}

#[tauri::command]
async fn mobile_permission(app: AppHandle, permission: String) -> Result<Value, String> {
    if !matches!(
        permission.as_str(),
        "notifications" | "accessibility" | "app_info" | "battery"
    ) {
        return Err("Unsupported permission.".into());
    }
    platform::call(&app, "requestPermission", json!({"permission":permission})).await
}

#[tauri::command]
async fn mobile_overlay(app: AppHandle, enabled: bool) -> Result<Value, String> {
    platform::call(&app, "assistantOverlay", json!({"enabled":enabled})).await
}

#[tauri::command]
async fn mobile_phone_control(
    app: AppHandle,
    host_id: String,
    enabled: bool,
) -> Result<(), String> {
    remote::configure_phone_control(&app, &host_id, enabled).await
}

#[tauri::command]
async fn mobile_web_settings(
    app: AppHandle,
    runtime: State<'_, RuntimeSlot>,
    action: String,
    web_search: Option<nexa_core::app_settings::WebSearchConfig>,
    connection_id: Option<String>,
) -> Result<Value, String> {
    let db = runtime.get(&app).await?.db.clone();
    let connection = if action == "use_deepseek" {
        let id = connection_id
            .as_deref()
            .ok_or("Choose a DeepSeek connection.")?;
        Some(connections::get(&app, Some(id)).await?)
    } else {
        None
    };
    tokio::task::spawn_blocking(move || {
        let mut saved = db.load_app_config().map_err(|e| e.to_string())?;
        let mut config = web_search.unwrap_or_else(|| saved.web_search.clone());
        if let Some(connection) = connection {
            web_settings::use_deepseek_connection(&mut config, &connection)?;
        }
        for provider in &mut config.custom_providers {
            if provider.api_key == "__NEXA_SAVED_KEY__" {
                provider.api_key = saved
                    .web_search
                    .custom_providers
                    .iter()
                    .find(|old| old.id == provider.id && old.preset == provider.preset)
                    .map(|old| old.api_key.clone())
                    .unwrap_or_default();
            }
            if let Some(url) = provider
                .base_url
                .as_deref()
                .filter(|url| !url.trim().is_empty())
            {
                validate_provider_url(url)?;
            }
        }
        match action.as_str() {
            "status" => Ok(json!(
                nexa_core::tools::web_search_tool::provider_status_snapshot(&config)
            )),
            "read" | "save" | "use_deepseek" => {
                if action != "read" {
                    saved.web_search = config.clone();
                    db.save_app_config(&saved).map_err(|e| e.to_string())?;
                }
                for provider in &mut config.custom_providers {
                    if !provider.api_key.is_empty() {
                        provider.api_key = "__NEXA_SAVED_KEY__".into();
                    }
                }
                let public = nexa_core::app_settings::AppConfig {
                    web_search: config,
                    ..Default::default()
                };
                Ok(json!(public))
            }
            _ => Err("Unsupported web settings operation".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn mobile_storage(
    app: AppHandle,
    runtime: State<'_, RuntimeSlot>,
    action: String,
    scope: Option<String>,
    cursor: Option<nexa_core::conversation::ConversationStorageCursor>,
    ids: Option<Vec<String>>,
) -> Result<Value, String> {
    let db = runtime.get(&app).await?.db.clone();
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?;
    let changed_ids = ids.clone().unwrap_or_default();
    let operation = action.clone();
    let result = tokio::task::spawn_blocking(move || match action.as_str() {
        "usage" => {
            let mut usage = db.storage_usage().map_err(|error| error.to_string())?;
            nexa_core::storage_usage::add_files(
                &mut usage,
                &[(directory, "files"), (cache, "runtime")],
                db.db_path(),
            );
            Ok(json!(usage))
        }
        "page" => db
            .conversation_storage_page(scope.as_deref().unwrap_or("active"), cursor.as_ref())
            .map(|page| json!(page))
            .map_err(|error| error.to_string()),
        "archive" => db
            .archive_conversations_batch(&ids.unwrap_or_default())
            .map(|count| json!(count))
            .map_err(|error| error.to_string()),
        "remove" => db
            .delete_conversations_batch(&ids.unwrap_or_default())
            .map(|count| json!(count))
            .map_err(|error| error.to_string()),
        _ => Err("Unsupported storage operation".into()),
    })
    .await
    .map_err(|error| error.to_string())??;
    if matches!(operation.as_str(), "archive" | "remove") {
        let _ = app.emit(
            "mobile:conversations",
            json!({"ids":changed_ids,"action":operation}),
        );
    }
    Ok(result)
}

#[tauri::command]
// Keep the existing flat IPC arguments compatible with native chat clients.
#[allow(clippy::too_many_arguments)]
async fn mobile_speech(
    app: AppHandle,
    runtime: State<'_, RuntimeSlot>,
    speech_runtime: State<'_, speech::SpeechRuntime>,
    action: String,
    text: Option<String>,
    language: Option<String>,
    mode: Option<String>,
    output_mode: Option<String>,
    speech_to_text: Option<nexa_core::app_settings::SpeechToTextConfig>,
    text_to_speech: Option<nexa_core::app_settings::TextToSpeechConfig>,
) -> Result<Value, String> {
    if !matches!(
        action.as_str(),
        "listen" | "finish" | "cancel" | "speak" | "stop" | "status" | "configure" | "settings"
    ) {
        return Err("Unsupported speech action.".into());
    }
    if text.as_ref().is_some_and(|text| text.len() > 512_000) {
        return Err("Load and read a smaller message section.".into());
    }
    let mut args = json!({"action":action,"text":text.unwrap_or_default()});
    if let Some(language) = language {
        args["language"] = json!(language);
    }
    if let Some(mode) = mode {
        args["mode"] = json!(mode);
    }
    if let Some(mode) = output_mode {
        args["outputMode"] = json!(mode);
    }
    if action == "cancel" {
        speech_runtime.cancel(false).await;
    }
    if action == "stop" {
        speech_runtime.cancel(true).await;
    }
    if matches!(action.as_str(), "cancel" | "stop" | "finish" | "settings") {
        return platform::call(&app, "speech", args).await;
    }
    let db = runtime.get(&app).await?.db.clone();
    let mut config = db.load_app_config().map_err(|_| "speech_configuration")?;
    let mut native = platform::call(&app, "speech", json!({"action":"status"})).await?;
    if action == "configure" {
        if speech_runtime.busy().await {
            return Err("speech_busy".into());
        }
        if let Some(mut input) = speech_to_text {
            if input.api_key == speech::SAVED_KEY {
                if (
                    input.provider.as_str(),
                    input.api_style.as_str(),
                    &input.base_url,
                ) != (
                    config.speech_to_text.provider.as_str(),
                    config.speech_to_text.api_style.as_str(),
                    &config.speech_to_text.base_url,
                ) {
                    return Err("speech_configuration".into());
                }
                input.api_key = config.speech_to_text.api_key.clone();
            }
            config.speech_to_text = input;
        }
        if let Some(mut output) = text_to_speech {
            if output.api_key == speech::SAVED_KEY {
                if (
                    output.provider.as_str(),
                    output.api_style.as_str(),
                    &output.base_url,
                ) != (
                    config.text_to_speech.provider.as_str(),
                    config.text_to_speech.api_style.as_str(),
                    &config.text_to_speech.base_url,
                ) {
                    return Err("speech_configuration".into());
                }
                output.api_key = config.text_to_speech.api_key.clone();
            }
            config.text_to_speech = output;
        }
        let input_mode = args["mode"].as_str().unwrap_or("system");
        let output_mode = args["outputMode"].as_str().unwrap_or("system");
        if !matches!(input_mode, "system" | "on_device" | "cloud")
            || !matches!(output_mode, "system" | "cloud")
        {
            return Err("speech_configuration".into());
        }
        let mut resolved = config.clone();
        connections::resolve_speech_credentials(&app, &mut resolved)
            .await
            .map_err(|_| "speech_configuration")?;
        if input_mode == "cloud" {
            speech::validate_input(&resolved.speech_to_text)?;
        }
        if output_mode == "cloud" {
            speech::validate_output(&resolved.text_to_speech)?;
        }
        db.save_app_config(&config)
            .map_err(|_| "speech_configuration")?;
        config = resolved;
        native = platform::call(&app, "speech", args.clone()).await?;
    } else {
        connections::resolve_speech_credentials(&app, &mut config)
            .await
            .map_err(|_| "speech_configuration")?;
    }
    if matches!(action.as_str(), "status" | "configure") {
        if !config.speech_to_text.api_key.is_empty() {
            config.speech_to_text.api_key = speech::SAVED_KEY.into();
        }
        if !config.text_to_speech.api_key.is_empty() {
            config.text_to_speech.api_key = speech::SAVED_KEY.into();
        }
        native["speechToText"] = json!(config.speech_to_text);
        native["textToSpeech"] = json!(config.text_to_speech);
        return Ok(native);
    }
    if action == "listen" {
        speech_runtime.cancel(true).await;
        let input = if native["mode"] == "cloud" {
            speech::validate_input(&config.speech_to_text)?;
            speech::apply_input_language(
                &mut config.speech_to_text,
                native["language"].as_str(),
                args["language"].as_str(),
            );
            Some(config.speech_to_text)
        } else {
            None
        };
        return speech_runtime.listen(&app, args, input).await;
    }
    let output = if native["outputMode"] == "cloud" {
        speech::validate_output(&config.text_to_speech)?;
        Some(config.text_to_speech)
    } else {
        None
    };
    speech_runtime.cancel(false).await;
    speech_runtime.speak(&app, args, output).await
}

#[tauri::command]
async fn mobile_scan_pairing(app: AppHandle) -> Result<Value, String> {
    platform::call(&app, "scanPairing", json!({})).await
}

#[tauri::command]
async fn mobile_pair(
    app: AppHandle,
    url: String,
    name: String,
    request_id: String,
) -> Result<Value, String> {
    remote::pair(&app, url, name, request_id).await
}

#[tauri::command]
async fn mobile_files(
    app: AppHandle,
    runtime: State<'_, RuntimeSlot>,
    action: String,
    relative_path: Option<String>,
) -> Result<Value, String> {
    let state = runtime.get(&app).await?;
    let root = state
        .workspace_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    match action.as_str() {
        "list" => {
            let mut paths = Vec::new();
            let mut directories = vec![root.clone()];
            while let Some(directory) = directories.pop() {
                for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let kind = entry.file_type().map_err(|e| e.to_string())?;
                    if kind.is_symlink() {
                        continue;
                    }
                    if kind.is_dir() {
                        directories.push(entry.path());
                    }
                    if kind.is_file() {
                        paths.push(
                            entry
                                .path()
                                .strip_prefix(&root)
                                .map_err(|e| e.to_string())?
                                .to_string_lossy()
                                .into_owned(),
                        );
                    }
                }
            }
            paths.sort();
            Ok(json!(paths))
        }
        "share" => {
            let requested = relative_path.ok_or("Choose a phone file.")?;
            let source = root
                .join(requested)
                .canonicalize()
                .map_err(|e| e.to_string())?;
            if !source.starts_with(&root) || !source.is_file() {
                return Err("Choose a file in the phone workspace.".into());
            }
            let share = app
                .path()
                .app_cache_dir()
                .map_err(|e| e.to_string())?
                .join("shared");
            tokio::fs::create_dir_all(&share)
                .await
                .map_err(|e| e.to_string())?;
            let destination = share.join(source.file_name().ok_or("Invalid file name.")?);
            tokio::fs::copy(&source, &destination)
                .await
                .map_err(|e| e.to_string())?;
            platform::call(&app, "shareFile", json!({"path":destination})).await
        }
        _ => Err("Unsupported file operation.".into()),
    }
}
#[tauri::command]
async fn mobile_remote(
    app: AppHandle,
    state: State<'_, Arc<remote::RemoteHosts>>,
    host_id: String,
    action: String,
    connection_id: Option<String>,
    method: Option<String>,
    params: Option<Value>,
) -> Result<Value, String> {
    match action.as_str() {
        "connect" => {
            state
                .connect(
                    app,
                    host_id,
                    connection_id.ok_or("Missing connection owner.")?,
                )
                .await?;
            Ok(Value::Null)
        }
        "close" => {
            state
                .close(
                    &host_id,
                    Some(
                        connection_id
                            .as_deref()
                            .ok_or("Missing connection owner.")?,
                    ),
                )
                .await;
            Ok(Value::Null)
        }
        "forget" => {
            state.close(&host_id, None).await;
            remote::forget(&app, &host_id).await?;
            connections::forget_host(&app, &host_id).await?;
            Ok(Value::Null)
        }
        "rpc" => {
            let method = method.ok_or("A method is required.")?;
            if method.starts_with("sync.") {
                return Err("Credential synchronization is available only through the native connection manager.".into());
            }
            remote::rpc(&app, &host_id, method, params).await
        }
        _ => Err("Unsupported remote operation.".into()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(RuntimeSlot::default())
        .manage(speech::SpeechRuntime::default())
        .manage(Arc::new(remote::RemoteHosts::default()))
        .plugin(platform::plugin())
        .invoke_handler(tauri::generate_handler![
            mobile_state,
            mobile_save_settings,
            mobile_connection,
            mobile_sync_connections,
            mobile_local,
            mobile_permission,
            mobile_scan_pairing,
            mobile_speech,
            mobile_overlay,
            mobile_phone_control,
            mobile_storage,
            mobile_web_settings,
            mobile_pair,
            mobile_remote,
            mobile_files
        ])
        .run(tauri::generate_context!())
        .expect("Nexa Mobile host failed");
}
