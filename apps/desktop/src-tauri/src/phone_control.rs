use async_trait::async_trait;
use nexa_core::agent::CancellationToken;
use nexa_core::{error::CoreError, tools::phone_control_tool::PhoneControlHost};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

pub struct PairedPhoneHost(pub AppHandle);
#[async_trait]
impl PhoneControlHost for PairedPhoneHost {
    async fn control(
        &self,
        _: &str,
        mut args: Value,
        cancel: CancellationToken,
    ) -> Result<Value, CoreError> {
        let server = match self.0.state::<crate::remote::RemoteState>().server() {
            Ok(server) => server,
            Err(error) => {
                return Ok(
                    json!({"code":"remote_access_unavailable","retryable":false,"requiresUserAction":true,"message":error,"recovery":"Enable Phone access on this computer, pair Nexa Mobile, and enable this computer's phone control in the phone settings."}),
                )
            }
        };
        if args["action"] == "list_devices"
            || (args["action"] == "status" && args["deviceId"].is_null())
        {
            return Ok(server.phone_control_devices());
        }
        let device = args["deviceId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                CoreError::InvalidInput(
                    "Select the exact deviceId from phone_control list_devices.".into(),
                )
            })?
            .to_owned();
        args.as_object_mut().unwrap().remove("deviceId");
        Ok(match server.control_phone(&device, args, cancel).await {
            Ok(mut result) => {
                result["deviceId"] = json!(device);
                result
            }
            Err(error) => {
                json!({"code":"phone_control_unavailable","retryable":false,"deviceId":device,"message":error,"recovery":"Inspect device status and obtain a new observation before continuing. Do not replay a possibly completed action."})
            }
        })
    }
}
