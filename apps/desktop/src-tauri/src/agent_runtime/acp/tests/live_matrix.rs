//! Opt-in acceptance against real adapters. Uses empty homes and never sends a
//! prompt or requests authentication; reports each protocol stage separately.
use super::*;
use futures::{stream, StreamExt};
use std::{collections::BTreeMap, path::Path};

fn isolated_environment(root: &Path, cache: &Path) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for name in ["PATH", "SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
        if let Ok(value) = std::env::var(name) {
            env.insert(name.into(), value);
        }
    }
    for name in [
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "CODEX_HOME",
        "GEMINI_CLI_HOME",
        "GEMINI_HOME",
        "CLAUDE_CONFIG_DIR",
        "QWEN_CODE_HOME",
        "PI_CODING_AGENT_DIR",
        "HERMES_HOME",
    ] {
        let directory = root.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        env.insert(name.into(), directory.to_string_lossy().into_owned());
    }
    env.insert(
        "NPM_CONFIG_CACHE".into(),
        cache.to_string_lossy().into_owned(),
    );
    env.insert("NPM_CONFIG_UPDATE_NOTIFIER".into(), "false".into());
    env.insert("NPM_CONFIG_AUDIT".into(), "false".into());
    env.insert("NPM_CONFIG_FUND".into(), "false".into());
    env.insert("PYTHONUTF8".into(), "1".into());
    env.insert("PYTHONIOENCODING".into(), "utf-8".into());
    for name in [
        "UV_CACHE_DIR",
        "UV_PYTHON_INSTALL_DIR",
        "UV_TOOL_DIR",
        "UV_TOOL_BIN_DIR",
    ] {
        env.insert(
            name.into(),
            cache
                .parent()
                .unwrap()
                .join(name)
                .to_string_lossy()
                .into_owned(),
        );
    }
    env
}

async fn probe(provider: String, root: std::path::PathBuf) -> Value {
    let preset = nexa_core::external_agent::preset(&provider).expect("bundled preset");
    let home = root.join(&provider).join(uuid::Uuid::new_v4().to_string());
    let cwd = home.join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    let launch = ExternalAgentLaunch {
        working_directory: cwd.to_string_lossy().into_owned(),
        env: isolated_environment(&home, &root.join("npm-cache")),
        ..Default::default()
    };
    let mut report = json!({
        "provider": provider, "version": preset.registry_version,
        "stage": "prepare", "status": "pending", "isolatedHome": true,
        "authenticationRequested": false, "inferenceRequested": false,
    });
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(900), async {
        let plan =
            super::super::transport::prepare_plan(preset, &launch, Some(&root.join("adapters")))
                .await?;
        report["prepared"] = json!(plan.prepared);
        report["executable"] = json!(plan.executable.file_name().unwrap().to_string_lossy());
        report["args"] = json!(plan.args);
        report["stage"] = json!("initialize");
        let mut command = super::super::transport::command_from_plan(&plan, &launch)?;
        command.env_clear().envs(&plan.env);
        let mut wire = Wire::spawn(command)?;
        let initialized = Session::initialize(&mut wire).await?;
        report["initializeMs"] = json!(started.elapsed().as_millis());
        report["authenticationMethods"] = json!(initialized
            .auth_methods
            .iter()
            .map(|method| json!({"id":method.id,"kind":method.kind}))
            .collect::<Vec<_>>());
        report["stage"] = json!("session/new");
        let catalog =
            super::super::auth::catalog(&mut wire, &launch, None, &[], initialized).await?;
        if let Some(cause) = catalog.setup_error {
            report["status"] = json!("session_error_with_recovery");
            report["error"] = json!(cause);
        } else if catalog.requires_authentication {
            report["status"] = json!("authentication_required");
        } else {
            report["status"] = json!("session_created");
            report["models"] = json!(catalog.models);
            report["configOptions"] = json!(catalog.config_options);
        }
        Ok::<_, nexa_core::error::CoreError>(())
    })
    .await;
    let failure = match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error.to_string()),
        Err(_) => Some("Acceptance operation exceeded its 900-second test deadline".into()),
    };
    if let Some(error) = failure {
        report["status"] = json!("failed");
        report["error"] = json!(nexa_core::external_agent::launch::redact_diagnostic(
            &error,
            &[]
        ));
    }
    report["elapsedMs"] = json!(started.elapsed().as_millis());
    report
}

#[tokio::test]
#[ignore = "downloads real adapters; requires NEXA_TEST_ACP_MATRIX_ROOT and NEXA_TEST_ACP_MATRIX_REPORT"]
async fn real_adapters_report_initialize_authentication_and_session_stages() {
    let root = std::path::PathBuf::from(std::env::var("NEXA_TEST_ACP_MATRIX_ROOT").unwrap());
    let output = std::path::PathBuf::from(std::env::var("NEXA_TEST_ACP_MATRIX_REPORT").unwrap());
    assert!(root.is_absolute() && output.is_absolute());
    std::fs::create_dir_all(&root).unwrap();
    let selection =
        std::env::var("NEXA_TEST_ACP_MATRIX_PROVIDERS").unwrap_or_else(|_| "all".into());
    let providers = nexa_core::external_agent::presets()
        .iter()
        .filter(|preset| selection == "all" || selection.split(',').any(|id| id == preset.provider))
        .map(|preset| preset.provider.clone())
        .collect::<Vec<_>>();
    assert!(!providers.is_empty());
    let mut pending = stream::iter(
        providers
            .into_iter()
            .map(|provider| probe(provider, root.clone())),
    )
    .buffer_unordered(3);
    let mut reports = Vec::new();
    while let Some(report) = pending.next().await {
        eprintln!("ACP_MATRIX {}", report);
        reports.push(report);
        let saved = json!({"platform": std::env::consts::OS, "results": reports});
        std::fs::write(&output, serde_json::to_vec_pretty(&saved).unwrap()).unwrap();
    }
}
