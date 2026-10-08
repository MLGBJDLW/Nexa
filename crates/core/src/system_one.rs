//! Explicit structured decisions, separate from text-generating agent models.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::CoreError;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemOneConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
}

impl std::fmt::Debug for SystemOneConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemOneConfig")
            .field("enabled", &self.enabled)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("credential_configured", &!self.api_key.is_empty())
            .finish()
    }
}

fn default_provider() -> String {
    "typesafe".into()
}
fn default_model() -> String {
    "jev-latest".into()
}

impl Default for SystemOneConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_provider(),
            api_key: String::new(),
            model: default_model(),
            base_url: None,
        }
    }
}

impl SystemOneConfig {
    pub fn is_configured(&self) -> bool {
        self.enabled
            && !self.api_key.trim().is_empty()
            && !self.model.trim().is_empty()
            && endpoint(self).is_ok()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemOnePreset {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub models: Vec<SystemOneModel>,
}

#[derive(Debug, Deserialize)]
pub struct SystemOneModel {
    pub id: String,
    pub name: String,
}

pub fn presets() -> &'static [SystemOnePreset] {
    static PRESETS: OnceLock<Vec<SystemOnePreset>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../shared/system-one-provider-presets.json"
        ))
        .expect("valid System One presets")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Question {
    Noul {
        instructions: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<BTreeMap<String, Value>>,
    },
    Choice {
        instructions: Value,
        criteria: BTreeMap<String, Value>,
    },
    Score {
        instructions: Value,
        criteria: Vec<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRequest {
    pub state: Value,
    pub questions: BTreeMap<String, Question>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

fn description(value: &Value) -> bool {
    value.is_string() || value.is_object() || value.is_array()
}

impl DecisionRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        if !description(&self.state) || self.questions.is_empty() {
            return Err(CoreError::InvalidInput(
                "System One needs text/structured state and at least one question.".into(),
            ));
        }
        if self
            .model
            .as_ref()
            .is_some_and(|model| model.trim().is_empty())
        {
            return Err(CoreError::InvalidInput(
                "System One model cannot be empty.".into(),
            ));
        }
        for question in self.questions.values() {
            let (instructions, valid_criteria) = match question {
                Question::Noul {
                    instructions,
                    criteria,
                } => (
                    instructions,
                    criteria.as_ref().is_none_or(|criteria| {
                        criteria.iter().all(|(key, value)| {
                            matches!(key.as_str(), "true" | "false") && description(value)
                        })
                    }),
                ),
                Question::Choice {
                    instructions,
                    criteria,
                } => (
                    instructions,
                    !criteria.is_empty()
                        && criteria.len() <= 255
                        && criteria
                            .values()
                            .all(|value| value.is_null() || description(value)),
                ),
                Question::Score {
                    instructions,
                    criteria,
                } => (
                    instructions,
                    (2..=10).contains(&criteria.len()) && criteria.iter().all(description),
                ),
            };
            if !description(instructions) || !valid_criteria {
                return Err(CoreError::InvalidInput("Invalid System One question: use Noul with optional true/false criteria, Choice with 1-255 options, or Score with 2-10 ordered levels; descriptions accept text, objects or arrays.".into()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
        legend: BTreeMap<String, Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionUsage {
    pub input_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionResponse {
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: DecisionUsage,
}

fn probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn distribution(
    probabilities: &BTreeMap<String, f64>,
    expected: impl Iterator<Item = String>,
) -> bool {
    let keys: std::collections::BTreeSet<_> = expected.collect();
    probabilities
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        == keys
        && probabilities.values().copied().all(probability)
        && (probabilities.values().sum::<f64>() - 1.0).abs() <= 0.001
}

impl DecisionResponse {
    fn validate(&self, request: &DecisionRequest) -> Result<(), CoreError> {
        let valid = !self.model.trim().is_empty()
            && self
                .usage
                .cost
                .is_none_or(|cost| cost.is_finite() && cost >= 0.0)
            && self.answers.len() == request.questions.len()
            && request.questions.iter().all(|(id, question)| {
                match (question, self.answers.get(id)) {
                    (Question::Noul { .. }, Some(Answer::Noul { noul })) => probability(*noul),
                    (
                        Question::Choice { criteria, .. },
                        Some(Answer::Choice {
                            choice,
                            probabilities,
                            confidence,
                        }),
                    ) => {
                        probability(*confidence)
                            && distribution(probabilities, criteria.keys().cloned())
                            && probabilities.get(choice).is_some_and(|selected| {
                                probabilities.values().all(|p| p <= &(selected + 0.000001))
                            })
                    }
                    (
                        Question::Score { criteria, .. },
                        Some(Answer::Score {
                            score,
                            probabilities,
                            confidence,
                            legend,
                        }),
                    ) => {
                        probability(*confidence)
                            && score.is_finite()
                            && (0.0..=(criteria.len() - 1) as f64).contains(score)
                            && distribution(
                                probabilities,
                                (0..criteria.len()).map(|index| index.to_string()),
                            )
                            && legend.keys().eq(probabilities.keys())
                            && (score
                                - probabilities
                                    .iter()
                                    .map(|(index, p)| index.parse::<f64>().unwrap_or(f64::NAN) * p)
                                    .sum::<f64>())
                            .abs()
                                <= 0.001
                    }
                    _ => false,
                }
            });
        if valid {
            Ok(())
        } else {
            Err(CoreError::InvalidInput("System One returned answers that do not match the requested types, options or probability ranges.".into()))
        }
    }
}

fn endpoint(config: &SystemOneConfig) -> Result<reqwest::Url, CoreError> {
    let preset = presets()
        .iter()
        .find(|preset| preset.id == config.provider)
        .ok_or_else(|| CoreError::InvalidInput("Unknown System One provider.".into()))?;
    let base = config
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .unwrap_or(&preset.base_url);
    let mut url = reqwest::Url::parse(base).map_err(|_| {
        CoreError::InvalidInput(
            "Enter the provider's HTTPS System One base URL in Settings.".into(),
        )
    })?;
    let expected_host = reqwest::Url::parse(&preset.base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned));
    let host_matches = match config.provider.as_str() {
        "alibaba-model-studio-cn" => url
            .host_str()
            .is_some_and(|host| host.ends_with(".cn-beijing.maas.aliyuncs.com")),
        "alibaba-model-studio-intl" => url
            .host_str()
            .is_some_and(|host| host.ends_with(".ap-southeast-1.maas.aliyuncs.com")),
        _ => url.host_str() == expected_host.as_deref(),
    };
    let path = url.path().trim_end_matches('/');
    let expected_path = if config.provider.starts_with("alibaba-model-studio-") {
        "/compatible-mode/v1"
    } else if config.provider == "openrouter" {
        "/api/v1"
    } else {
        "/v1"
    };
    if url.scheme() != "https"
        || !host_matches
        || path != expected_path
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err(CoreError::InvalidInput("System One base URL must match the selected provider and region, without credentials, query or fragment.".into()));
    }
    url.set_path(&format!("{expected_path}/systemone"));
    Ok(url)
}

fn prepare_request(
    config: &SystemOneConfig,
    request: &DecisionRequest,
    policy: &crate::privacy::ChatPrivacyPolicy,
) -> Result<DecisionRequest, CoreError> {
    request.validate()?;
    let mut state = request.state.clone();
    policy.redact_data(&mut state);
    let mut questions = BTreeMap::new();
    for (index, question) in request.questions.values().enumerate() {
        let redact = |value: &Value| {
            let mut projected = value.clone();
            policy.redact_data(&mut projected);
            projected
        };
        // Correlation keys belong to the host. Move user-provided choice keys
        // into redacted descriptions, then generate new transport identities.
        let projected = match question {
            Question::Noul {
                instructions,
                criteria,
            } => Question::Noul {
                instructions: redact(instructions),
                criteria: criteria.as_ref().map(|items| {
                    items
                        .iter()
                        .map(|(key, value)| (key.clone(), redact(value)))
                        .collect()
                }),
            },
            Question::Choice {
                instructions,
                criteria,
            } => Question::Choice {
                instructions: redact(instructions),
                criteria: criteria
                    .iter()
                    .enumerate()
                    .map(|(option_index, (label, value))| {
                        (
                            format!("c{option_index}"),
                            redact(&json!({"label":label,"description":value})),
                        )
                    })
                    .collect(),
            },
            Question::Score {
                instructions,
                criteria,
            } => Question::Score {
                instructions: redact(instructions),
                criteria: criteria.iter().map(redact).collect(),
            },
        };
        questions.insert(format!("q{index}"), projected);
    }
    let projected = DecisionRequest {
        state,
        questions,
        model: Some(
            request
                .model
                .as_deref()
                .unwrap_or(&config.model)
                .trim()
                .to_owned(),
        ),
    };
    projected.validate()?;
    Ok(projected)
}

pub async fn evaluate(
    db: &crate::db::Database,
    request: DecisionRequest,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<DecisionResponse, CoreError> {
    let config = db.load_app_config()?.system_one;
    if !config.enabled || config.api_key.trim().is_empty() {
        return Err(CoreError::InvalidInput("Enable a System One provider and add its API key under Settings > Providers > Structured decisions.".into()));
    }
    let url = endpoint(&config)?;
    let lease = db.privacy_lease(cancellation)?;
    let projected = prepare_request(&config, &request, &lease.policy)?;
    let client = reqwest::Client::builder()
        .user_agent(crate::USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| CoreError::Internal("Could not initialize System One transport.".into()))?;
    let mut response = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(CoreError::Cancelled("System One evaluation cancelled.".into())),
        _ = lease.cancelled() => return Err(CoreError::Cancelled("Privacy changed during System One evaluation.".into())),
        response = lease.scope(send(&client, url, config.api_key.trim(), &projected)) => response?,
    };
    lease.ensure_current()?;
    restore_answer_identity(&request, &mut response);
    response.validate(&request)?;
    Ok(response)
}

fn restore_answer_identity(request: &DecisionRequest, response: &mut DecisionResponse) {
    response.answers = request
        .questions
        .iter()
        .enumerate()
        .map(|(index, (id, question))| {
            let mut answer = response
                .answers
                .remove(&format!("q{index}"))
                .expect("validated question identity");
            match (question, &mut answer) {
                (
                    Question::Choice { criteria, .. },
                    Answer::Choice {
                        choice,
                        probabilities,
                        ..
                    },
                ) => {
                    let identities: BTreeMap<_, _> = criteria
                        .keys()
                        .enumerate()
                        .map(|(index, key)| (format!("c{index}"), key.clone()))
                        .collect();
                    *choice = identities[choice].clone();
                    *probabilities = std::mem::take(probabilities)
                        .into_iter()
                        .map(|(key, probability)| (identities[&key].clone(), probability))
                        .collect();
                }
                (Question::Score { criteria, .. }, Answer::Score { legend, .. }) => {
                    *legend = criteria
                        .iter()
                        .enumerate()
                        .map(|(index, value)| (index.to_string(), value.clone()))
                        .collect();
                }
                _ => {}
            }
            (id.clone(), answer)
        })
        .collect();
}

async fn send(
    client: &reqwest::Client,
    url: reqwest::Url,
    api_key: &str,
    request: &DecisionRequest,
) -> Result<DecisionResponse, CoreError> {
    crate::privacy::runtime::ensure_invocation_current()?;
    let mut response = client
        .post(url)
        .bearer_auth(api_key)
        .json(request)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                CoreError::TransientLlm(
                    "System One request timed out; no automatic retry was sent.".into(),
                )
            } else {
                CoreError::TransientLlm(
                    "System One connection failed; no automatic retry was sent.".into(),
                )
            }
        })?;
    let status = response.status();
    if matches!(status.as_u16(), 429 | 529) {
        let retry_after_secs = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                value.parse().ok().or_else(|| {
                    chrono::DateTime::parse_from_rfc2822(value)
                        .ok()
                        .map(|date| {
                            (date.with_timezone(&chrono::Utc) - chrono::Utc::now())
                                .num_seconds()
                                .max(0) as u64
                        })
                })
            })
            .unwrap_or(5);
        return Err(CoreError::RateLimited { retry_after_secs });
    }
    if !status.is_success() {
        // Error bodies may echo credentials or source material. Status is enough
        // to select recovery without recording a second copy of sensitive data.
        return Err(CoreError::ProviderUnavailable { status: status.as_u16(), message: match status.as_u16() {
            401 | 403 => "System One access rejected; check the selected provider, API key and account access.",
            404 => "System One model or endpoint is unavailable for this account.",
            400 | 422 => "System One rejected the state or question schema; check the configured model's constraints.",
            300..=399 => "System One redirects are not followed with API credentials.",
            _ => "System One provider request failed.",
        }.into() });
    }
    const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| CoreError::TransientLlm("System One response was interrupted.".into()))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(CoreError::InvalidInput(
                "System One response exceeds the transport envelope; split the question batch."
                    .into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    let result: DecisionResponse = serde_json::from_slice(&body).map_err(|_| {
        CoreError::InvalidInput("System One returned an invalid structured response.".into())
    })?;
    result.validate(request)?;
    crate::privacy::runtime::ensure_invocation_current()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};

    fn mixed_request() -> DecisionRequest {
        serde_json::from_value(json!({"state":{"body":"A supported document"},"questions":{
            "fact":{"type":"noul","instructions":"The evidence supports the claim"},
            "route":{"type":"choice","instructions":{"question":"Select the best label"},"criteria":{"keep":null,"review":"Needs review"}},
            "score":{"type":"score","instructions":"Support level","criteria":["unsupported","partial","supported"]}
        }})).unwrap()
    }

    fn mixed_response() -> Value {
        json!({"model":"jev-1.13.0","answers":{
            "fact":{"type":"noul","noul":0.9},
            "route":{"type":"choice","choice":"keep","probabilities":{"keep":0.8,"review":0.2},"confidence":0.6},
            "score":{"type":"score","score":1.25,"probabilities":{"0":0.0,"1":0.75,"2":0.25},"legend":{"0":"unsupported","1":"partial","2":"supported"},"confidence":0.7}
        },"usage":{"input_tokens":125,"output_tokens":18}})
    }

    fn server(
        status: &str,
        headers: &str,
        response: Value,
    ) -> (reqwest::Url, std::thread::JoinHandle<(String, Value)>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let headers = headers.to_owned();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 8192];
            let (header, body) = loop {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_string();
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|length| length.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break (
                            header,
                            serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap(),
                        );
                    }
                }
            };
            let body_text = response.to_string();
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body_text}", body_text.len()).unwrap();
            (header, body)
        });
        (
            reqwest::Url::parse(&format!("http://{address}/v1/systemone")).unwrap(),
            thread,
        )
    }

    #[test]
    fn system_one_matches_types_probabilities_and_complete_question_identity() {
        let request = mixed_request();
        request.validate().unwrap();
        let raw = mixed_response();
        serde_json::from_value::<DecisionResponse>(raw.clone())
            .unwrap()
            .validate(&request)
            .unwrap();
        for (pointer, value) in [
            ("/answers/fact/noul", json!(1.5)),
            ("/answers/route/choice", json!("missing")),
            ("/answers/route/confidence", json!(-0.1)),
            ("/answers/route/probabilities/keep", json!(0.1)),
            ("/answers/score/score", json!(1.0)),
            ("/answers/score/type", json!("noul")),
        ] {
            let mut invalid = raw.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(
                serde_json::from_value::<DecisionResponse>(invalid)
                    .map_err(CoreError::from)
                    .and_then(|response| response.validate(&request))
                    .is_err(),
                "{pointer}"
            );
        }
        let mut extra = raw.clone();
        extra["answers"]["extra"] = json!({"type":"noul","noul":0.5});
        assert!(serde_json::from_value::<DecisionResponse>(extra)
            .unwrap()
            .validate(&request)
            .is_err());
        let mut missing = raw;
        missing["answers"].as_object_mut().unwrap().remove("fact");
        assert!(serde_json::from_value::<DecisionResponse>(missing)
            .unwrap()
            .validate(&request)
            .is_err());
    }

    #[test]
    fn system_one_preserves_fractional_scores_and_unknown_output_usage() {
        let mut raw = mixed_response();
        raw["usage"]
            .as_object_mut()
            .unwrap()
            .remove("output_tokens");
        let response: DecisionResponse = serde_json::from_value(raw).unwrap();
        response.validate(&mixed_request()).unwrap();
        assert!(response.usage.output_tokens.is_none());
        assert!(serde_json::to_value(response).unwrap()["usage"]
            .get("output_tokens")
            .is_none());
    }

    #[test]
    fn system_one_endpoint_and_model_catalog_are_provider_and_region_scoped() {
        let mut config = SystemOneConfig::default();
        assert_eq!(
            endpoint(&config).unwrap().as_str(),
            "https://api.typesafe.ai/v1/systemone"
        );
        for base in [
            "http://api.typesafe.ai/v1",
            "https://api.typesafe.ai.evil.test/v1",
            "https://user:secret@api.typesafe.ai/v1",
            "https://api.typesafe.ai/v1?key=secret",
            "https://api.typesafe.ai/v1/other",
        ] {
            config.base_url = Some(base.into());
            assert!(endpoint(&config).is_err(), "{base}");
        }
        config.provider = "alibaba-model-studio-cn".into();
        config.base_url =
            Some("https://workspace.cn-beijing.maas.aliyuncs.com/compatible-mode/v1".into());
        assert!(endpoint(&config).is_ok());
        config.provider = "alibaba-model-studio-intl".into();
        assert!(endpoint(&config).is_err());
        config.provider = "openrouter".into();
        config.base_url = None;
        assert_eq!(
            endpoint(&config).unwrap().as_str(),
            "https://openrouter.ai/api/v1/systemone"
        );
        for (id, model) in [
            ("typesafe", "jev-1.13.0"),
            ("openrouter", "typesafe/jev-1.13"),
            ("siliconflow-cn", "Kev-4b"),
            ("siliconflow-international", "Kev-4B"),
        ] {
            assert!(presets()
                .iter()
                .find(|p| p.id == id)
                .unwrap()
                .models
                .iter()
                .any(|m| m.id == model));
        }
    }

    #[test]
    fn system_one_projects_source_data_and_uses_opaque_wire_question_ids() {
        let db = crate::db::Database::open_memory().unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let lease = db.privacy_lease(&token).unwrap();
        let request: DecisionRequest = serde_json::from_value(json!({"state":{"person@example.com":"mail me"},"questions":{
            "person@example.com":{"type":"choice","instructions":"Review person@example.com","criteria":{"person@example.com":null,"review":"safe"}}
        }})).unwrap();
        let projected =
            prepare_request(&SystemOneConfig::default(), &request, &lease.policy).unwrap();
        assert_eq!(
            projected
                .questions
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["q0"]
        );
        let serialized = serde_json::to_string(&projected).unwrap();
        assert!(!serialized.contains("person@example.com"));
        assert!(serialized.contains("[EMAIL]"));
        assert!(serialized.contains("\"type\":\"choice\""));
    }

    #[test]
    fn system_one_custom_redaction_cannot_change_choice_positions_or_binary_keys() {
        let db = crate::db::Database::open_memory().unwrap();
        let mut policy = db.load_privacy_config().unwrap();
        policy.redact_patterns.push(crate::privacy::RedactRule {
            name: "option identifiers".into(),
            pattern: "option_[0-9]+|true|false".into(),
            replacement: "[PRIVATE]".into(),
        });
        db.save_privacy_config(&policy).unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let lease = db.privacy_lease(&token).unwrap();
        let request: DecisionRequest = serde_json::from_value(json!({"state":"example","questions":{
            "a":{"type":"choice","instructions":"Choose","criteria":{"option_0":"one","option_1":"two"}},
            "b":{"type":"noul","instructions":"Check","criteria":{"true":"yes","false":"no"}}
        }})).unwrap();
        let projected =
            prepare_request(&SystemOneConfig::default(), &request, &lease.policy).unwrap();
        let body = serde_json::to_value(&projected).unwrap();
        assert!(body["questions"]["q0"]["criteria"].get("c0").is_some());
        assert!(body["questions"]["q1"]["criteria"].get("true").is_some());
        assert!(!body.to_string().contains("option_0"));
        let mut response: DecisionResponse = serde_json::from_value(json!({"model":"typesafe/jev-1.13","usage":{"input_tokens":12,"cost":0.01},"answers":{
            "q0":{"type":"choice","choice":"c1","probabilities":{"c0":0.1,"c1":0.9},"confidence":0.8},
            "q1":{"type":"noul","noul":0.7}
        }})).unwrap();
        response.validate(&projected).unwrap();
        restore_answer_identity(&request, &mut response);
        response.validate(&request).unwrap();
        let result = serde_json::to_value(response).unwrap();
        assert_eq!(result["answers"]["a"]["choice"], "option_1");
        assert_eq!(result["answers"]["a"]["probabilities"]["option_1"], 0.9);
        assert_eq!(result["usage"]["cost"], 0.01);
    }

    #[tokio::test]
    async fn system_one_wire_sends_typed_body_and_validates_the_real_response() {
        let mut request = mixed_request();
        request.model = Some("jev-1.13.0".into());
        let (url, thread) = server("200 OK", "", mixed_response());
        let result = send(&reqwest::Client::new(), url, "fixture-key", &request)
            .await
            .unwrap();
        assert_eq!(result.model, "jev-1.13.0");
        let (headers, body) = thread.join().unwrap();
        assert!(headers.starts_with("POST /v1/systemone "));
        assert!(headers
            .to_ascii_lowercase()
            .contains("authorization: bearer fixture-key"));
        assert!(body.get("messages").is_none());
        assert_eq!(body["model"], "jev-1.13.0");
        assert_eq!(body["questions"]["score"]["type"], "score");
    }

    #[tokio::test]
    async fn system_one_rate_limits_and_errors_never_echo_private_provider_bodies() {
        let request = mixed_request();
        let (url, thread) = server(
            "529 Overloaded",
            "Retry-After: 12\r\n",
            json!({"secret":"fixture-key"}),
        );
        assert!(matches!(
            send(&reqwest::Client::new(), url, "fixture-key", &request).await,
            Err(CoreError::RateLimited {
                retry_after_secs: 12
            })
        ));
        thread.join().unwrap();
        let (url, thread) = server("401 Unauthorized", "", json!({"secret":"fixture-key"}));
        let error = send(&reqwest::Client::new(), url, "fixture-key", &request)
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("fixture-key"));
        thread.join().unwrap();
    }

    #[tokio::test]
    async fn system_one_is_disabled_by_default_and_revocation_stops_physical_dispatch() {
        let db = crate::db::Database::open_memory().unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        assert!(evaluate(&db, mixed_request(), &token)
            .await
            .unwrap_err()
            .to_string()
            .contains("Enable"));
        let lease = db.privacy_lease(&token).unwrap();
        let mut policy = db.load_privacy_config().unwrap();
        policy.redact_patterns.push(crate::privacy::RedactRule {
            name: "new boundary".into(),
            pattern: "private".into(),
            replacement: "[PRIVATE]".into(),
        });
        db.save_privacy_config(&policy).unwrap();
        let result = lease
            .scope(send(
                &reqwest::Client::new(),
                reqwest::Url::parse("http://127.0.0.1:1/v1/systemone").unwrap(),
                "fixture-key",
                &mixed_request(),
            ))
            .await;
        assert!(matches!(result, Err(CoreError::Cancelled(_))));
    }
}
