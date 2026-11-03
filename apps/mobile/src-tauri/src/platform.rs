use serde_json::{json, Value};
use tauri::AppHandle;
#[cfg(target_os = "android")]
use tauri::Manager;

#[cfg(target_os = "android")]
pub struct NativeServices(pub tauri::plugin::PluginHandle<tauri::Wry>);

pub async fn call(app: &AppHandle, command: &str, args: Value) -> Result<Value, String> {
    #[cfg(target_os = "android")]
    {
        app.state::<NativeServices>()
            .0
            .run_mobile_plugin_async(command, args)
            .await
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, command, args);
        Err("This native service requires Android.".into())
    }
}

pub async fn read_secret(app: &AppHandle, key: &str) -> Result<Option<Value>, String> {
    let result = call(app, "secureRead", json!({"key":key})).await?;
    result["value"]
        .as_str()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| e.to_string())
}

pub async fn save_secret(app: &AppHandle, key: &str, value: Option<&Value>) -> Result<(), String> {
    call(
        app,
        "secureWrite",
        json!({"key":key,"value":value.map(Value::to_string)}),
    )
    .await?;
    Ok(())
}

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("phone")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            _app.manage(NativeServices(
                _api.register_android_plugin("com.nexa.mobile", "PhonePlugin")?,
            ));
            Ok(())
        })
        .build()
}
