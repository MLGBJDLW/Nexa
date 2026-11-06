use std::sync::OnceLock;

use nexa_core::{
    app_settings::AppConfig, connection_sync::SharedModelConnection,
    speech_credentials::connection_speech_endpoint,
};
use serde::{Deserialize, Serialize};

use crate::speech;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preset {
    id: String,
    provider: String,
    api_style: String,
    base_url: String,
}

fn presets(raw: &str, supported: fn(&str) -> bool) -> Vec<Preset> {
    serde_json::from_str::<Vec<Preset>>(raw)
        .expect("bundled speech presets")
        .into_iter()
        .filter(|preset| supported(&preset.api_style))
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Route {
    preset_id: &'static str,
    base_url: String,
    #[serde(skip)]
    provider: &'static str,
    #[serde(skip)]
    api_style: &'static str,
}

fn routes(source: &SharedModelConnection, presets: &'static [Preset]) -> Vec<Route> {
    presets
        .iter()
        .filter_map(|preset| {
            let base_url = connection_speech_endpoint(
                &source.provider,
                source.base_url.as_deref(),
                &preset.provider,
                &preset.base_url,
                &preset.api_style,
            )?;
            Some(Route {
                preset_id: &preset.id,
                provider: &preset.provider,
                api_style: &preset.api_style,
                base_url,
            })
        })
        .collect()
}

#[derive(Serialize)]
pub struct SpeechConnection {
    id: String,
    name: String,
    input: Vec<Route>,
    output: Vec<Route>,
}

fn resolve_bound_key(
    source: &SharedModelConnection,
    routes: &[Route],
    provider: &str,
    style: &str,
    base_url: Option<&str>,
    key: &mut String,
) {
    if routes.iter().any(|route| {
        route.provider == provider
            && route.api_style == style
            && Some(route.base_url.as_str()) == base_url
    }) {
        key.clone_from(&source.api_key);
    }
}

/// The returned catalog contains only public selection metadata. Keys are
/// materialized on the execution copy and never serialized alongside a route.
pub fn resolve(
    config: &mut AppConfig,
    sources: &[&SharedModelConnection],
    list_connections: bool,
) -> Vec<SpeechConnection> {
    static INPUT: OnceLock<Vec<Preset>> = OnceLock::new();
    static OUTPUT: OnceLock<Vec<Preset>> = OnceLock::new();
    let input_presets = INPUT.get_or_init(|| {
        presets(
            include_str!("../../../../shared/stt-provider-presets.json"),
            speech::supports_input,
        )
    });
    let output_presets = OUTPUT.get_or_init(|| {
        presets(
            include_str!("../../../../shared/tts-provider-presets.json"),
            speech::supports_output,
        )
    });
    // Retain automatic scope-based inheritance for pre-existing unbound settings.
    config.inherit_speech_credentials(sources.iter().map(|source| {
        (
            source.provider.as_str(),
            source.base_url.as_deref(),
            source.api_key.as_str(),
        )
    }));
    let input = &mut config.speech_to_text;
    let output = &mut config.text_to_speech;
    // A missing or changed bound account must fail validation, even when
    // another account shares its scope or an old config contains a copied key.
    if input.connection_id.is_some() {
        input.api_key.clear();
    }
    if output.connection_id.is_some() {
        output.api_key.clear();
    }
    let mut available = Vec::new();
    for source in sources {
        let input_selected = input.connection_id.as_deref() == Some(&source.id);
        let output_selected = output.connection_id.as_deref() == Some(&source.id);
        if source.api_key.trim().is_empty()
            || !(list_connections || input_selected || output_selected)
        {
            continue;
        }
        let input_routes = routes(source, input_presets);
        let output_routes = routes(source, output_presets);
        if input_selected {
            resolve_bound_key(
                source,
                &input_routes,
                &input.provider,
                &input.api_style,
                input.base_url.as_deref(),
                &mut input.api_key,
            );
        }
        if output_selected {
            resolve_bound_key(
                source,
                &output_routes,
                &output.provider,
                &output.api_style,
                output.base_url.as_deref(),
                &mut output.api_key,
            );
        }
        if list_connections && !(input_routes.is_empty() && output_routes.is_empty()) {
            available.push(SpeechConnection {
                id: source.id.clone(),
                name: source.name.clone(),
                input: input_routes,
                output: output_routes,
            });
        }
    }
    available
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account(id: &str, key: &str) -> SharedModelConnection {
        serde_json::from_value(json!({
            "id":id, "name":id, "provider":"alibaba_model_studio",
            "model":"qwen-chat", "apiKey":key,
            "baseUrl":"https://dashscope.aliyuncs.com/compatible-mode/v1"
        }))
        .unwrap()
    }

    #[test]
    fn synced_accounts_offer_only_supported_speech_directions() {
        for (provider, base, input, output) in [
            ("open_ai", "https://api.openai.com/v1", true, true),
            (
                "google",
                "https://generativelanguage.googleapis.com/v1beta",
                true,
                true,
            ),
            ("siliconflow", "https://api.siliconflow.cn/v1", true, true),
            ("open_ai", "https://api.groq.com/openai/v1", true, true),
            ("open_ai", "https://api.mistral.ai/v1", true, true),
            ("open_ai", "https://api.meta.ai/v1", true, false),
            ("open_ai", "https://api.minimax.io/v1", false, true),
            ("deep_seek", "https://api.deepseek.com", false, false),
        ] {
            let mut source = account("saved", "fixture");
            source.provider = provider.into();
            source.base_url = Some(base.into());
            let offered = resolve(&mut AppConfig::default(), &[&source], true);
            assert_eq!(
                offered.iter().any(|account| !account.input.is_empty()),
                input,
                "{base}"
            );
            assert_eq!(
                offered.iter().any(|account| !account.output.is_empty()),
                output,
                "{base}"
            );
        }
    }

    #[test]
    fn synced_speech_accounts_offer_routes_and_resolve_only_the_selected_account() {
        use nexa_core::connection_sync::{
            merge_connection_response, ConnectionExchange, CONNECTION_SYNC_VERSION,
        };
        let desktop = nexa_core::db::Database::open_memory().unwrap();
        for (name, key) in [
            ("account-one", "fixture-one"),
            ("account-two", "fixture-two"),
        ] {
            desktop.save_agent_config(&serde_json::from_value(json!({
                "name":name, "provider":"alibaba_model_studio", "apiKey":key, "model":"qwen-chat",
                "baseUrl":"https://dashscope.aliyuncs.com/compatible-mode/v1", "isDefault":name == "account-one"
            })).unwrap()).unwrap();
        }
        let exchange = desktop
            .exchange_model_connections(
                &ConnectionExchange {
                    version: CONNECTION_SYNC_VERSION,
                    connections: vec![],
                    after: None,
                },
                "Phone",
            )
            .unwrap();
        let mut phone = std::collections::HashMap::new();
        merge_connection_response(
            &mut phone,
            &Default::default(),
            &mut Default::default(),
            &Default::default(),
            &exchange,
        )
        .unwrap();
        let first = phone
            .values()
            .find(|record| record.name == "account-one")
            .unwrap();
        let mut second = phone
            .values()
            .find(|record| record.name == "account-two")
            .unwrap()
            .clone();
        let mut coding = account("coding-plan", "fixture-coding");
        coding.base_url = Some("https://coding.dashscope.aliyuncs.com/v1".into());
        let mut config = AppConfig::default();
        let offered = resolve(&mut config, &[first, &second, &coding], true);
        let public = serde_json::to_value(&offered).unwrap();
        assert_eq!(public.as_array().unwrap().len(), 2);
        assert!(!public.to_string().contains("fixture"));
        let input = offered[1]
            .input
            .iter()
            .find(|p| p.preset_id == "alibaba-qwen-audio")
            .unwrap();
        let output = offered[0]
            .output
            .iter()
            .find(|p| p.preset_id == "dashscope-cosyvoice")
            .unwrap();
        config.speech_to_text = serde_json::from_value(json!({
            "connectionId":second.id, "provider":input.provider, "apiStyle":input.api_style,
            "baseUrl":input.base_url, "model":"qwen-audio-3.1-asr-flash"
        }))
        .unwrap();
        config.text_to_speech = serde_json::from_value(json!({
            "connectionId":first.id, "provider":output.provider, "apiStyle":output.api_style,
            "baseUrl":output.base_url, "model":"qwen-audio-3.0-tts-flash", "voice":"Cherry"
        }))
        .unwrap();
        let mut execution = config.clone();
        resolve(&mut execution, &[first, &second], false);
        assert_eq!(execution.speech_to_text.api_key, "fixture-two");
        assert_eq!(execution.text_to_speech.api_key, "fixture-one");
        speech::validate_input(&execution.speech_to_text).unwrap();
        speech::validate_output(&execution.text_to_speech).unwrap();
        second.api_key = "fixture-rotated".into();
        resolve(&mut execution, &[&second], false);
        assert_eq!(execution.speech_to_text.api_key, "fixture-rotated");
        assert!(execution.text_to_speech.api_key.is_empty());
        execution.speech_to_text.base_url = Some("https://attacker.example/v1".into());
        resolve(&mut execution, &[&second], false);
        assert!(execution.speech_to_text.api_key.is_empty());
        execution.speech_to_text.base_url = config.speech_to_text.base_url;
        execution.speech_to_text.api_style = "openai_transcription".into();
        resolve(&mut execution, &[&second], false);
        assert!(execution.speech_to_text.api_key.is_empty());
    }
}
