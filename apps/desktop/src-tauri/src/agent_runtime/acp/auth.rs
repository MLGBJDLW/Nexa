//! Authentication is explicit setup work owned by the external runtime. Native
//! terminal methods launch that same adapter; they never call `authenticate`.
use super::{
    catalog::{Initialization, Session},
    error,
    transport::Wire,
    Catalog, Result,
};
use nexa_core::external_agent::{
    launch::{self, LaunchPlan},
    preset, ExternalAgentLaunch,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub kind: String,
    #[serde(skip)]
    args: Vec<String>,
    #[serde(skip)]
    env: BTreeMap<String, String>,
}

pub(super) fn methods(initialized: &Value) -> Result<Vec<AuthMethod>> {
    let Some(values) = initialized.get("authMethods").and_then(Value::as_array) else {
        return Ok(vec![]);
    };
    if values.len() > 64 {
        return Err(error("ACP authentication method list is too large"));
    }
    let mut methods = Vec::new();
    for value in values {
        let kind = value["type"].as_str().unwrap_or("agent");
        if !matches!(kind, "agent" | "terminal") {
            continue;
        }
        let id = value["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
            .ok_or_else(|| error("Invalid ACP authentication method id"))?;
        let args: Vec<String> = value
            .get("args")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()
            .map_err(error)?
            .unwrap_or_default();
        let env: BTreeMap<String, String> = value
            .get("env")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()
            .map_err(error)?
            .unwrap_or_default();
        ExternalAgentLaunch {
            args: Some(args.clone()),
            env: env.clone(),
            ..Default::default()
        }
        .validate()?;
        if args.len() > 128 || env.len() > 128 {
            return Err(error("ACP authentication invocation is too large"));
        }
        if methods.iter().any(|method: &AuthMethod| method.id == id) {
            return Err(error("Duplicate ACP authentication method"));
        }
        methods.push(AuthMethod {
            id: id.into(),
            name: value["name"]
                .as_str()
                .unwrap_or(id)
                .chars()
                .filter(|c| !c.is_control())
                .take(120)
                .collect(),
            description: value["description"]
                .as_str()
                .map(|text| text.chars().filter(|c| !c.is_control()).take(400).collect()),
            kind: kind.into(),
            args,
            env,
        });
    }
    Ok(methods)
}

pub(crate) enum Authentication {
    Connected(Catalog),
    Terminal {
        plan: LaunchPlan,
        cwd: String,
        label: String,
    },
}

pub(crate) async fn authenticate(
    provider: &str,
    preferences: &ExternalAgentLaunch,
    db: &nexa_core::db::Database,
    model: Option<&str>,
    method_id: &str,
) -> Result<Authentication> {
    let preset = preset(provider).ok_or_else(|| error("Unknown external agent"))?;
    let resolved = preferences.resolve(db, None, None)?;
    let mut plan =
        launch::prepare(preset, &resolved, super::adapter_cache_root(db).as_deref()).await?;
    let mut wire = Wire::spawn(super::transport::command_from_plan(&plan, &resolved)?)?;
    let initialized = Session::initialize(&mut wire).await?;
    let method = initialized
        .auth_methods
        .iter()
        .find(|method| method.id == method_id)
        .ok_or_else(|| {
            error("This sign-in method is no longer offered; check the connection again")
        })?
        .clone();
    if method.kind == "terminal" {
        drop(wire);
        plan.args.extend(method.args);
        for (key, value) in method.env {
            if cfg!(windows) {
                plan.env.retain(|name, _| !name.eq_ignore_ascii_case(&key));
            }
            plan.env.insert(key, value);
        }
        // Share Windows command-shim validation with the protocol transport.
        let _ = super::transport::command_from_plan(&plan, &resolved)?;
        return Ok(Authentication::Terminal {
            plan,
            cwd: resolved.working_directory,
            label: preset.name.clone(),
        });
    }
    wire.request("authenticate", json!({"methodId":method.id}))
        .await?;
    let servers = super::mcp::selected(db, &resolved.mcp_server_ids)?;
    catalog(&mut wire, &resolved, model, &servers, initialized)
        .await
        .map(Authentication::Connected)
}

pub(super) async fn catalog(
    wire: &mut Wire,
    launch: &ExternalAgentLaunch,
    model: Option<&str>,
    servers: &[nexa_core::mcp::McpServer],
    initialized: Initialization,
) -> Result<Catalog> {
    let auth_methods = initialized.auth_methods.clone();
    let Some(mut session) =
        Session::create(wire, &launch.working_directory, servers, initialized).await?
    else {
        return Ok(Catalog {
            models: vec![],
            config_options: vec![],
            commands: vec![],
            auth_methods,
            requires_authentication: true,
        });
    };
    session
        .configure(
            wire,
            model,
            &launch.config_options,
            None,
            launch.config_options_model.as_deref(),
            super::catalog::ConfigurationUse::Discovery,
        )
        .await?;
    Ok(Catalog {
        models: session.models,
        config_options: session.config_options,
        commands: session.commands,
        auth_methods: vec![],
        requires_authentication: false,
    })
}
