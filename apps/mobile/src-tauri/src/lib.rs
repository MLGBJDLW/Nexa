mod local;
mod phone_tool;
mod platform;
mod remote;

use local::{LocalRuntime, LocalSettings};

#[derive(Default)]
struct RuntimeSlot(tokio::sync::OnceCell<Arc<LocalRuntime>>);
impl RuntimeSlot {
    async fn get(&self, app: &AppHandle) -> Result<&Arc<LocalRuntime>, String> {
        self.0
            .get_or_try_init(|| async {
                LocalRuntime::open(app, app.path().app_data_dir().map_err(|e| e.to_string())?).await
            })
            .await
    }
}
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
async fn mobile_state(app: AppHandle, runtime: State<'_, RuntimeSlot>) -> Result<Value, String> {
    runtime.get(&app).await?;
    let saved = platform::read_secret(&app, "local-provider").await?;
    Ok(
        json!({"version":env!("CARGO_PKG_VERSION"),"settings":saved.as_ref().map(|v|&v["settings"]),"hosts":remote::hosts(&app).await?,"capabilities":platform::call(&app,"capabilities",json!({})).await?}),
    )
}

#[tauri::command]
async fn mobile_save_settings(
    app: AppHandle,
    settings: LocalSettings,
    api_key: String,
) -> Result<(), String> {
    if settings.model.trim().is_empty() {
        return Err("Enter a model ID.".into());
    }
    nexa_core::provider_registry::provider_type_from_key(&settings.provider_type)
        .ok_or("Unsupported provider type.")?;
    if let Some(url) = &settings.base_url {
        validate_provider_url(url)?;
    }
    let old = platform::read_secret(&app, "local-provider").await?;
    let same_account = old.as_ref().is_some_and(|value| {
        value["settings"]["providerType"] == settings.provider_type
            && value["settings"]["baseUrl"] == json!(settings.base_url)
    });
    let key = if api_key.is_empty() && same_account {
        old.as_ref()
            .and_then(|v| v["apiKey"].as_str())
            .unwrap_or_default()
            .to_string()
    } else {
        api_key
    };
    if key.is_empty() {
        return Err("Enter the phone's own API key.".into());
    }
    platform::save_secret(
        &app,
        "local-provider",
        Some(&json!({"settings":settings,"apiKey":key})),
    )
    .await
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
        "list" => state
            .db
            .remote_conversations(input["before"].as_str())
            .map_err(|e| e.to_string()),
        "messages" => state
            .db
            .remote_messages(&string("conversationId")?, input["beforeOrder"].as_i64())
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
            Ok(json!({"run":run,"checkpoint":checkpoint}))
        }
        "approvals" => state.approvals(&string("conversationId")?).await,
        "create" => {
            let saved = platform::read_secret(&app, "local-provider")
                .await?
                .ok_or("Configure a local provider first.")?;
            state.create(
                &serde_json::from_value(saved["settings"].clone()).map_err(|e| e.to_string())?,
            )
        }
        "start" => {
            state
                .start(
                    app,
                    input["conversationId"].as_str().map(str::to_owned),
                    string("message")?,
                    string("requestId")?,
                    input["checkpointId"].as_str().map(str::to_owned),
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
    if !matches!(permission.as_str(), "notifications" | "accessibility") {
        return Err("Unsupported permission.".into());
    }
    platform::call(&app, "requestPermission", json!({"permission":permission})).await
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
    method: Option<String>,
    params: Option<Value>,
) -> Result<Value, String> {
    match action.as_str() {
        "connect" => {
            state.connect(app, host_id).await?;
            Ok(Value::Null)
        }
        "close" => {
            state.close(&host_id).await;
            Ok(Value::Null)
        }
        "forget" => {
            state.close(&host_id).await;
            remote::forget(&app, &host_id).await?;
            Ok(Value::Null)
        }
        "rpc" => {
            remote::rpc(
                &app,
                &host_id,
                method.ok_or("A method is required.")?,
                params,
            )
            .await
        }
        _ => Err("Unsupported remote operation.".into()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(RuntimeSlot::default())
        .manage(Arc::new(remote::RemoteHosts::default()))
        .plugin(platform::plugin())
        .invoke_handler(tauri::generate_handler![
            mobile_state,
            mobile_save_settings,
            mobile_local,
            mobile_permission,
            mobile_pair,
            mobile_remote,
            mobile_files
        ])
        .run(tauri::generate_context!())
        .expect("Nexa Mobile host failed");
}
