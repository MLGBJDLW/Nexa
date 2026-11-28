use async_trait::async_trait;
use nexa_core::{error::CoreError, tools::phone_control_tool::PhoneControlHost};
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

pub struct PhoneHost {
    app: AppHandle,
    lane: std::sync::Arc<tokio::sync::Mutex<()>>,
    lease: std::sync::Mutex<Option<tokio::sync::OwnedMutexGuard<()>>>,
}

impl PhoneHost {
    pub fn new(app: AppHandle, lane: std::sync::Arc<tokio::sync::Mutex<()>>) -> Self {
        Self {
            app,
            lane,
            lease: Default::default(),
        }
    }
}
#[async_trait]
impl PhoneControlHost for PhoneHost {
    async fn control(
        &self,
        call_id: &str,
        mut arguments: Value,
        cancel: CancellationToken,
    ) -> Result<Value, CoreError> {
        if arguments["deviceId"]
            .as_str()
            .is_some_and(|id| id != "local")
        {
            return Err(CoreError::InvalidInput("This conversation operates this phone. Choose a computer conversation for paired devices.".into()));
        }
        if arguments["action"] == "list_devices" {
            return Ok(json!({"devices":[{"id":"local","name":"This phone"}]}));
        }
        if arguments["action"] != "status" {
            let mut lease = self.lease.lock().unwrap_or_else(|e| e.into_inner());
            if lease.is_none() {
                *lease = Some(self.lane.clone().try_lock_owned().map_err(|_| CoreError::InvalidInput(
                    "Another phone task is operating the screen. Continue with independent work or wait for that task to finish.".into()
                ))?);
            }
        }
        arguments.as_object_mut().unwrap().remove("deviceId");
        arguments["requestId"] = json!(call_id);
        tokio::select! {
            result = crate::platform::call(&self.app, "phoneControl", arguments) => result.map_err(CoreError::InvalidInput),
            _ = cancel.cancelled() => {
                let _ = crate::platform::call(&self.app, "cancelPhoneControl", json!({"requestId":call_id})).await;
                Ok(json!({"code":"phone_control_cancelled","retryable":false,"message":"The phone action was cancelled. Observe before deciding whether another action is needed; never repeat an uncertain send."}))
            }
        }
    }
}
