use super::{error, transport::Wire, Result};
use serde::Serialize;
use serde_json::{json, Value};

pub(super) const DEFAULT_MODEL: &str = "@nexa/agent-default";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Model {
    pub id: String,
    pub name: String,
    pub reasoning_efforts: Vec<String>,
}

pub(super) struct Session {
    pub id: String,
    pub models: Vec<Model>,
    model_option: Option<String>,
    current_model: Option<String>,
    pub images: bool,
}

fn options(values: &Value, models: &mut Vec<Model>) -> Result<()> {
    for option in values
        .as_array()
        .ok_or_else(|| error("Invalid ACP model options"))?
    {
        if let Some(group) = option.get("options") {
            options(group, models)?;
            continue;
        }
        let id = option["value"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 1024)
            .ok_or_else(|| error("Invalid ACP model id"))?;
        if models.len() >= 1500 {
            return Err(error("ACP model catalog too large"));
        }
        models.push(Model {
            id: id.into(),
            name: option["name"]
                .as_str()
                .unwrap_or(id)
                .chars()
                .take(200)
                .collect(),
            reasoning_efforts: vec![],
        });
    }
    Ok(())
}

impl Session {
    pub(super) fn parse(value: &Value, images: bool) -> Result<Self> {
        let id = value["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 1024)
            .ok_or_else(|| error("ACP session/new returned no session id"))?
            .to_string();
        let model_config = value["configOptions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|option| option["category"] == "model" && option["type"] == "select");
        let mut models = Vec::new();
        let (model_option, current_model) = if let Some(option) = model_config {
            options(&option["options"], &mut models)?;
            (
                Some(
                    option["id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| error("ACP model config has no id"))?
                        .into(),
                ),
                option["currentValue"].as_str().map(str::to_owned),
            )
        } else {
            for model in value
                .pointer("/models/availableModels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let id = model["modelId"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 1024)
                    .ok_or_else(|| error("Invalid ACP model id"))?;
                if models.len() >= 1500 {
                    return Err(error("ACP model catalog too large"));
                }
                models.push(Model {
                    id: id.into(),
                    name: model["name"]
                        .as_str()
                        .unwrap_or(id)
                        .chars()
                        .take(200)
                        .collect(),
                    reasoning_efforts: vec![],
                });
            }
            (
                None,
                value
                    .pointer("/models/currentModelId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
        };
        if models.is_empty() {
            // No invented cloud catalog: allow only the runtime's own default.
            models.push(Model {
                id: DEFAULT_MODEL.into(),
                name: "Agent default".into(),
                reasoning_efforts: vec![],
            });
        }
        let mut seen = std::collections::HashSet::new();
        models.retain(|model| seen.insert(model.id.clone()));
        if let Some(current) = current_model.as_ref() {
            models.sort_by_key(|model| model.id != *current);
        }
        Ok(Self {
            id,
            models,
            model_option,
            current_model,
            images,
        })
    }

    pub(super) async fn connect(wire: &mut Wire, cwd: &str) -> Result<Self> {
        let initialized = wire.request("initialize", json!({
            "protocolVersion":1,
            "clientInfo":{"name":"nexa","version":env!("CARGO_PKG_VERSION")},
            "clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}
        })).await?;
        if initialized["protocolVersion"] != 1 {
            return Err(error(
                "The external agent does not support ACP protocol version 1",
            ));
        }
        let images = initialized
            .pointer("/agentCapabilities/promptCapabilities/image")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let value = wire
            .request("session/new", json!({"cwd":cwd,"mcpServers":[]}))
            .await?;
        Self::parse(&value, images)
    }

    pub(super) async fn select_model(&mut self, wire: &mut Wire, model: &str) -> Result<()> {
        if !self.models.iter().any(|item| item.id == model) {
            return Err(error("The selected external-agent model is no longer available. Refresh the model list and select an available model."));
        }
        if model == DEFAULT_MODEL || self.current_model.as_deref() == Some(model) {
            return Ok(());
        }
        if let Some(option) = &self.model_option {
            let response = wire
                .request(
                    "session/set_config_option",
                    json!({"sessionId":self.id,"configId":option,"value":model}),
                )
                .await?;
            if !response["configOptions"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|item| item["id"] == *option && item["currentValue"] == model)
            {
                return Err(error(
                    "The external agent did not confirm the selected model",
                ));
            }
        } else {
            wire.request(
                "session/set_model",
                json!({"sessionId":self.id,"modelId":model}),
            )
            .await?;
        }
        self.current_model = Some(model.to_string());
        Ok(())
    }
}
