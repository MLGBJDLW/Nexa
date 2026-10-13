use super::{config, error, transport::Wire, Result};
use serde::Serialize;
use serde_json::{json, Value};

pub(super) const DEFAULT_MODEL: &str = "@nexa/agent-default";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigurationUse {
    Discovery,
    Inference,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Model {
    pub id: String,
    pub name: String,
    pub reasoning_efforts: Vec<String>,
}

#[derive(Clone)]
pub(super) struct Initialization {
    pub auth_methods: Vec<super::auth::AuthMethod>,
    images: bool,
    mcp_capabilities: Value,
}

pub(super) struct Session {
    pub id: String,
    pub cwd: String,
    pub terminals: std::sync::Arc<super::terminals::Terminals>,
    pub models: Vec<Model>,
    pub config_options: Vec<config::ConfigOption>,
    pub commands: Vec<String>,
    model_option: Option<String>,
    current_model: Option<String>,
    pub images: bool,
}

impl Session {
    pub(super) fn parse(value: &Value, images: bool) -> Result<Self> {
        let id = value["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 1024)
            .ok_or_else(|| error("ACP session/new returned no session id"))?
            .to_string();
        let config_options = config::parse(value)?;
        let mut models = Vec::new();
        let (model_option, current_model) =
            if let Some((id, current, choices)) = config::models(&config_options) {
                models = choices;
                (Some(id), Some(current))
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
            cwd: String::new(),
            terminals: std::sync::Arc::new(super::terminals::Terminals::default()),
            models,
            config_options,
            commands: vec![],
            model_option,
            current_model,
            images,
        })
    }

    pub(super) async fn connect(wire: &mut Wire, cwd: &str) -> Result<Self> {
        Self::connect_with_mcp(wire, cwd, &[], None).await
    }

    pub(super) async fn connect_with_mcp(
        wire: &mut Wire,
        cwd: &str,
        servers: &[nexa_core::mcp::McpServer],
        authentication_method: Option<&str>,
    ) -> Result<Self> {
        let initialized = Self::initialize_with_services(wire, true).await?;
        Self::create_authenticated(wire, cwd, servers, initialized, authentication_method).await?
            .ok_or_else(|| error("The external agent requires authentication. Open its Settings profile and choose a sign-in method."))
    }

    pub(super) async fn initialize(wire: &mut Wire) -> Result<Initialization> {
        Self::initialize_with_services(wire, false).await
    }

    async fn initialize_with_services(
        wire: &mut Wire,
        native_services: bool,
    ) -> Result<Initialization> {
        let initialized = wire.request("initialize", json!({
            "protocolVersion":1,
            "clientInfo":{"name":"nexa","version":env!("CARGO_PKG_VERSION")},
            "clientCapabilities":{"fs":{"readTextFile":native_services,"writeTextFile":native_services},"terminal":native_services,"auth":{"terminal":true}}
        })).await?;
        if initialized["protocolVersion"] != 1 {
            return Err(error(
                "This agent did not negotiate ACP version 1. Disable experimental ACP v2 in its launch environment or use a compatible ACP adapter.",
            ));
        }
        let images = initialized
            .pointer("/agentCapabilities/promptCapabilities/image")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(Initialization {
            auth_methods: super::auth::methods(&initialized)?,
            images,
            mcp_capabilities: initialized["agentCapabilities"]["mcpCapabilities"].clone(),
        })
    }

    pub(super) async fn create_authenticated(
        wire: &mut Wire,
        cwd: &str,
        servers: &[nexa_core::mcp::McpServer],
        initialized: Initialization,
        authentication_method: Option<&str>,
    ) -> Result<Option<Self>> {
        let session = Self::create(wire, cwd, servers, initialized.clone()).await?;
        if session.is_some() {
            return Ok(session);
        }
        let Some(method) = authentication_method.and_then(|id| {
            initialized
                .auth_methods
                .iter()
                .find(|method| method.id == id && method.kind == "agent")
        }) else {
            return Ok(None);
        };
        // Only the user's saved choice can authorize authentication on a new
        // connection. Never auto-select a method or run terminal login here.
        wire.request("authenticate", json!({"methodId":method.id}))
            .await?;
        Self::create(wire, cwd, servers, initialized).await
    }

    pub(super) async fn create(
        wire: &mut Wire,
        cwd: &str,
        servers: &[nexa_core::mcp::McpServer],
        initialized: Initialization,
    ) -> Result<Option<Self>> {
        let servers = super::mcp::project(servers, &initialized.mcp_capabilities)?;
        let response = wire
            .request_response("session/new", json!({"cwd":cwd,"mcpServers":servers}))
            .await?;
        if response["error"]["code"] == -32000 {
            return Ok(None);
        }
        let value = super::transport::rpc_result(response, "session/new")?;
        let mut session = Self::parse(&value, initialized.images)?;
        session.cwd = cwd.to_string();
        session.sync(wire)?;
        Ok(Some(session))
    }

    pub(super) async fn select_model(&mut self, wire: &mut Wire, model: &str) -> Result<()> {
        self.sync(wire)?;
        if !self.models.iter().any(|item| item.id == model) {
            return Err(error("The selected external-agent model is no longer available. Refresh the model list and select an available model."));
        }
        if model == DEFAULT_MODEL || self.current_model.as_deref() == Some(model) {
            return Ok(());
        }
        if let Some(option) = self.model_option.clone() {
            self.set_option(wire, &option, model).await?;
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

    pub(super) async fn set_option(
        &mut self,
        wire: &mut Wire,
        id: &str,
        value: &str,
    ) -> Result<()> {
        let option = self
            .config_options
            .iter()
            .find(|option| option.id == id)
            .ok_or_else(|| {
                error(format!(
                    "Native option '{id}' is unavailable; check the connection again"
                ))
            })?;
        if !option.options.iter().any(|choice| choice.value == value) {
            return Err(error(format!(
                "Native option '{id}' does not support the saved value; refresh its options"
            )));
        }
        if option.current_value == value {
            return Ok(());
        }
        let is_model = config::is_model(option);
        if id == config::LEGACY_MODE {
            wire.request(
                "session/set_mode",
                json!({"sessionId":self.id,"modeId":value}),
            )
            .await?;
            if let Some(option) = self
                .config_options
                .iter_mut()
                .find(|option| option.id == id)
            {
                option.current_value = value.into();
            }
        } else {
            let response = wire
                .request(
                    "session/set_config_option",
                    json!({"sessionId":self.id,"configId":id,"value":value}),
                )
                .await?;
            self.replace_options(&response)?;
            // Non-model values may normalize (Qwen's reasoning "default"
            // becomes the model's effective level). The successful response
            // and complete replacement own that state; model identity is exact.
            if is_model
                && !self
                    .config_options
                    .iter()
                    .any(|option| option.id == id && option.current_value == value)
            {
                return Err(error(
                    "The external agent did not confirm the selected configuration",
                ));
            }
        }
        self.sync(wire)?;
        Ok(())
    }

    fn replace_options(&mut self, value: &Value) -> Result<()> {
        let options = config::parse(value)?;
        if let Some((id, current, mut models)) = config::models(&options) {
            models.sort_by_key(|model| model.id != current);
            self.model_option = Some(id);
            self.current_model = Some(current);
            self.models = models;
        } else if self.model_option.take().is_some() {
            self.current_model = None;
            self.models = vec![Model {
                id: DEFAULT_MODEL.into(),
                name: "Agent default".into(),
                reasoning_efforts: vec![],
            }];
        }
        self.config_options = options;
        Ok(())
    }

    pub(super) fn update(&mut self, update: &Value) -> Result<bool> {
        match update["sessionUpdate"].as_str() {
            Some("config_option_update") => self.replace_options(update)?,
            Some("current_mode_update") => {
                if let Some(mode) = update["currentModeId"].as_str() {
                    if let Some(option) = self
                        .config_options
                        .iter_mut()
                        .find(|option| option.id == config::LEGACY_MODE)
                    {
                        option.current_value = mode.into();
                    }
                }
            }
            Some("available_commands_update") => {
                self.commands = update["availableCommands"]
                    .as_array()
                    .ok_or_else(|| error("Invalid ACP command catalog"))?
                    .iter()
                    .take(256)
                    .filter_map(|command| {
                        command["name"]
                            .as_str()
                            .filter(|name| name.len() <= 128)
                            .map(str::to_owned)
                    })
                    .collect();
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(super) fn sync(&mut self, wire: &mut Wire) -> Result<()> {
        let mut retained = std::collections::VecDeque::new();
        for message in wire.take_pending()? {
            if message["method"] == "session/update"
                && message["params"]["sessionId"] == self.id
                && self.update(&message["params"]["update"])?
            {
                continue;
            }
            retained.push_back(message);
        }
        wire.requeue(retained);
        Ok(())
    }

    pub(super) async fn configure(
        &mut self,
        wire: &mut Wire,
        model: Option<&str>,
        preferences: &std::collections::BTreeMap<String, String>,
        effort: Option<&str>,
        saved_model: Option<&str>,
        purpose: ConfigurationUse,
    ) -> Result<()> {
        self.sync(wire)?;
        let mut remaining = preferences.clone();
        // Provider and mode can replace the entire model catalog (e.g. Goose).
        // Resolve them in advertised order before validating the chat model.
        while let Some(id) = self
            .config_options
            .iter()
            .find(|option| {
                remaining.contains_key(&option.id)
                    && !config::is_model(option)
                    && !config::is_thought(option)
                    && option.category.as_deref() != Some("model_config")
            })
            .map(|option| option.id.clone())
        {
            let value = remaining.remove(&id).expect("selected preference");
            if purpose == ConfigurationUse::Discovery
                && !self.config_options.iter().any(|option| {
                    option.id == id && option.options.iter().any(|choice| choice.value == value)
                })
            {
                continue;
            }
            self.set_option(wire, &id, &value).await?;
        }
        // Discovery must still return alternatives after a saved model retires.
        // Inference keeps its strict identity check and never switches silently.
        let model = model.filter(|selected| {
            purpose == ConfigurationUse::Inference
                || self.models.iter().any(|model| model.id == *selected)
        });
        let reuse_model_options = saved_model.is_none_or(|saved| Some(saved) == model);
        if let Some(model) = model {
            self.select_model(wire, model).await?;
        }
        for (id, value) in remaining {
            if !reuse_model_options {
                continue;
            }
            // Chat reasoning exclusively owns thought options, including
            // Default. Ignore legacy hidden preferences even for opaque IDs.
            // Respect an explicitly different category on a same-named option.
            let thought_preference = self
                .config_options
                .iter()
                .find(|option| option.id == id)
                .map(config::is_thought)
                .unwrap_or_else(|| {
                    matches!(id.as_str(), "reasoning_effort" | "effort" | "thought_level")
                });
            if self.model_option.as_deref() == Some(id.as_str()) || thought_preference {
                continue;
            }
            if purpose == ConfigurationUse::Discovery
                && !self.config_options.iter().any(|option| {
                    option.id == id && option.options.iter().any(|choice| choice.value == value)
                })
            {
                continue;
            }
            self.set_option(wire, &id, &value).await?;
        }
        if let Some(effort) = effort {
            let option = self.config_options.iter().find(|option| config::is_thought(option)).map(|option| option.id.clone())
                .ok_or_else(|| error("The external agent does not advertise reasoning configuration for this model"))?;
            self.set_option(wire, &option, effort).await?;
        }
        if model.is_some_and(|model| {
            model != DEFAULT_MODEL && self.current_model.as_deref() != Some(model)
        }) {
            return Err(error("Native configuration changed the selected model; refresh its options before inference"));
        }
        Ok(())
    }
}
