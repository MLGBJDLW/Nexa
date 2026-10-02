use std::path::PathBuf;

use nexa_agent_eval::{run_suite, EvalConfig, EvalMode};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut mode = EvalMode::Scripted;
    let mut config = EvalConfig::default();
    let mut output = PathBuf::from("docs/local/task-eval.json");
    let mut filter = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--live" => mode = EvalMode::Live,
            "--config" => {
                let path = args.next().ok_or("--config needs a JSON path")?;
                config = serde_json::from_slice(&std::fs::read(path)?)?;
            }
            "--output" => output = args.next().ok_or("--output needs a path")?.into(),
            "--filter" => filter = Some(args.next().ok_or("--filter needs a task ID substring")?),
            "--help" | "-h" => {
                println!("nexa-agent-eval [--live --config CONFIG.json] [--filter TASK] [--output REPORT.json]\nScripted mode exercises the real executor without a model account. Live mode uses NEXA_EVAL_API_KEY and the configured route. Reports distinguish runtime fixtures from live model task scores.");
                return Ok(());
            }
            _ => return Err(format!("Unknown option: {argument}").into()),
        }
    }
    if mode == EvalMode::Live && (config.model == "scripted-eval" || config.base_url.is_none()) {
        return Err("Live evaluation requires an explicit model and baseUrl in --config".into());
    }
    let api_key = if mode == EvalMode::Live {
        std::env::var("NEXA_EVAL_API_KEY").ok()
    } else {
        None
    };
    if mode == EvalMode::Live
        && api_key.is_none()
        && !matches!(
            config.provider,
            nexa_core::llm::ProviderType::Ollama | nexa_core::llm::ProviderType::LmStudio
        )
    {
        return Err("Set NEXA_EVAL_API_KEY for the explicitly configured live route".into());
    }
    let report = run_suite(mode, &config, api_key.as_deref(), filter.as_deref()).await?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}/{} {} cases passed; report: {}",
        report.passed,
        report.total,
        report.score_kind,
        output.display()
    );
    if report.passed != report.total {
        std::process::exit(1);
    }
    Ok(())
}
