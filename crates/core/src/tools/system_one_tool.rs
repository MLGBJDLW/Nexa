//! Optional typed decision inference through the host's configured provider.

use super::{Tool, ToolCategory, ToolDef, ToolOutput, ToolResult};
use crate::error::CoreError;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::OnceLock;

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/evaluate_decisions.json");

pub struct EvaluateDecisionsTool;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionArgs {
    state: String,
    questions: Vec<ToolQuestion>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolQuestion {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    instructions: String,
    options: Option<Vec<ChoiceOption>>,
    levels: Option<Vec<String>>,
    yes: Option<String>,
    no: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChoiceOption {
    label: String,
    description: Option<String>,
}

impl DecisionArgs {
    fn request(&self) -> Result<crate::system_one::DecisionRequest, CoreError> {
        use crate::system_one::Question;
        let mut questions = std::collections::BTreeMap::new();
        for question in &self.questions {
            let instructions = json!(question.instructions);
            let no_binary_criteria = question.yes.is_none() && question.no.is_none();
            let value = match question.kind.as_str() {
                "noul" if question.options.is_none() && question.levels.is_none() => Question::Noul {
                    instructions, criteria: (!no_binary_criteria).then(|| [
                        ("true", question.yes.as_ref()), ("false", question.no.as_ref())
                    ].into_iter().filter_map(|(key,value)| value.map(|value|(key.into(),json!(value)))).collect()),
                },
                "choice" if question.levels.is_none() && no_binary_criteria => Question::Choice {
                    instructions, criteria: question.options.as_deref().unwrap_or_default().iter().enumerate()
                        .map(|(index, option)| (format!("option_{index}"), json!({"label":option.label,"description":option.description}))).collect(),
                },
                "score" if question.options.is_none() && no_binary_criteria => Question::Score {
                    instructions, criteria: question.levels.as_deref().unwrap_or_default().iter().map(|level|json!(level)).collect(),
                },
                _ => return Err(CoreError::InvalidInput("Use options only with choice, levels only with score, and yes/no only with noul.".into())),
            };
            if question.id.trim().is_empty()
                || question.instructions.trim().is_empty()
                || question.options.as_ref().is_some_and(|options| {
                    options.iter().any(|option| option.label.trim().is_empty())
                })
                || questions.insert(question.id.clone(), value).is_some()
            {
                return Err(CoreError::InvalidInput("Decision questions need distinct nonempty IDs, instructions and option labels.".into()));
            }
        }
        let request = crate::system_one::DecisionRequest {
            state: json!(self.state),
            questions,
            model: None,
        };
        request.validate()?;
        Ok(request)
    }
}

#[async_trait]
impl Tool for EvaluateDecisionsTool {
    fn name(&self) -> &str {
        "evaluate_decisions"
    }
    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }
    fn parameters_schema(&self) -> serde_json::Value {
        ToolDef::from_json(&DEF, DEF_JSON).parameters.clone()
    }
    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::DocumentAnalysis, ToolCategory::Web]
    }
    async fn execute(
        &self,
        context: super::ToolExecutionContext<'_>,
    ) -> Result<ToolResult, CoreError> {
        let args: DecisionArgs = serde_json::from_str(context.arguments)?;
        let request = args.request()?;
        let cancellation = context.cancel_token.cloned().unwrap_or_default();
        let result = crate::system_one::evaluate(context.db, request, &cancellation).await?;
        let mut results = Vec::new();
        for (index, question) in args.questions.iter().enumerate() {
            let answer = &result.answers[&question.id];
            let mut value = serde_json::to_value(answer)?;
            value["questionIndex"] = json!(index);
            value["id"] = json!(question.id);
            if let crate::system_one::Answer::Choice {
                choice,
                probabilities,
                ..
            } = answer
            {
                value["choiceIndex"] = json!(choice
                    .strip_prefix("option_")
                    .and_then(|index| index.parse::<usize>().ok()));
                value["options"] = json!(question.options.as_deref().unwrap_or_default().iter().enumerate().map(|(index, option)|
                    json!({"optionIndex":index,"label":option.label,"probability":probabilities.get(&format!("option_{index}"))})).collect::<Vec<_>>());
            }
            results.push(value);
        }
        let data = json!({"kind":"systemOneEvaluation","authority":"advisory","model":result.model,"usage":result.usage,"results":results});
        let content = serde_json::to_string(&data)?;
        Ok(ToolResult::from_output(
            context.call_id,
            false,
            ToolOutput {
                llm_content: content,
                display_content: format!(
                    "Evaluated {} questions with {}.",
                    result.answers.len(),
                    result.model
                ),
                data: Some(data),
                artifacts: Some(
                    json!({"kind":"systemOneEvaluation","questionCount":result.answers.len(),"executedActions":false}),
                ),
                attachments: Vec::new(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_tool_has_portable_closed_shapes_and_stable_option_positions() {
        let input = json!({"state":"authorized evidence","questions":[
            {"id":"route","type":"choice","instructions":"Choose the relevant contact","options":[{"label":"alice@example.com"},{"label":"bob@example.com"}]},
            {"id":"score","type":"score","instructions":"Support level","levels":["low","high"]},
            {"id":"check","type":"noul","instructions":"The claim is supported","yes":"Evidence agrees"}
        ]});
        let registry = crate::tools::default_tool_registry();
        registry
            .prepare_execution("evaluate_decisions", "valid", &input.to_string())
            .unwrap();
        let args: DecisionArgs = serde_json::from_value(input.clone()).unwrap();
        let request = args.request().unwrap();
        let wire = serde_json::to_value(request).unwrap();
        assert_eq!(
            wire["questions"]["route"]["criteria"]["option_0"]["label"],
            "alice@example.com"
        );
        assert!(
            wire.get("model").is_none(),
            "the tool cannot override a configured account's model"
        );
        for field in ["api_key", "provider", "base_url", "model"] {
            let mut invalid = input.clone();
            invalid[field] = json!("untrusted");
            assert!(
                registry
                    .prepare_execution("evaluate_decisions", "invalid", &invalid.to_string())
                    .is_err(),
                "{field}"
            );
        }
        let mut mixed = input.clone();
        mixed["questions"][2]["options"] = json!([{"label":"incompatible"}]);
        assert!(serde_json::from_value::<DecisionArgs>(mixed)
            .unwrap()
            .request()
            .is_err());
        let mut duplicate = input.clone();
        duplicate["questions"][1]["id"] = json!("route");
        assert!(serde_json::from_value::<DecisionArgs>(duplicate)
            .unwrap()
            .request()
            .is_err());

        fn validate_schema(value: &serde_json::Value) {
            if value.get("type").and_then(serde_json::Value::as_str) == Some("array") {
                assert!(value.get("items").is_some());
            }
            if let Some(properties) = value
                .get("properties")
                .and_then(serde_json::Value::as_object)
            {
                assert_eq!(value["additionalProperties"], false);
                for schema in properties.values() {
                    validate_schema(schema);
                }
            }
            if let Some(items) = value.get("items") {
                validate_schema(items);
            }
        }
        validate_schema(&EvaluateDecisionsTool.parameters_schema());
    }
}
