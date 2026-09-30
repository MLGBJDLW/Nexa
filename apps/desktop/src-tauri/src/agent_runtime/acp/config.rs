//! ACP config categories guide the UI; opaque IDs and advertised values own
//! correctness. Full replacements avoid retaining options from an old model.
use super::{catalog::Model, error, Result};
use serde::Serialize;
use serde_json::Value;

pub(super) const LEGACY_MODE: &str = "@nexa/mode";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigOption {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub current_value: String,
    pub options: Vec<Choice>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Choice {
    pub value: String,
    pub name: String,
}

pub(super) fn choices(values: &Value, depth: usize) -> Result<Vec<Choice>> {
    if depth > 8 {
        return Err(error("ACP option groups are too deeply nested"));
    }
    let mut result = Vec::new();
    for value in values
        .as_array()
        .ok_or_else(|| error("Invalid ACP select options"))?
    {
        if let Some(group) = value.get("options") {
            result.extend(choices(group, depth + 1)?);
        } else {
            let id = value["value"]
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 1024)
                .ok_or_else(|| error("Invalid ACP option value"))?;
            result.push(Choice {
                value: id.into(),
                name: value["name"]
                    .as_str()
                    .unwrap_or(id)
                    .chars()
                    .take(200)
                    .collect(),
            });
        }
        if result.len() > 1500 {
            return Err(error("ACP option catalog too large"));
        }
    }
    Ok(result)
}
pub(super) fn parse(value: &Value) -> Result<Vec<ConfigOption>> {
    let mut result = Vec::new();
    if let Some(options) = value.get("configOptions") {
        for option in options
            .as_array()
            .filter(|values| values.len() <= 64)
            .ok_or_else(|| error("Invalid ACP config options"))?
        {
            // Boolean config is not advertised by this v1 client. Unknown future
            // types stay unavailable instead of receiving guessed values.
            if option["type"] != "select" {
                continue;
            }
            let id = option["id"]
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 1024)
                .ok_or_else(|| error("ACP config option has no id"))?;
            if result.iter().any(|item: &ConfigOption| item.id == id) {
                return Err(error("Duplicate ACP config option id"));
            }
            result.push(ConfigOption {
                id: id.into(),
                name: option["name"]
                    .as_str()
                    .unwrap_or(id)
                    .chars()
                    .take(200)
                    .collect(),
                category: option["category"].as_str().map(str::to_owned),
                current_value: option["currentValue"]
                    .as_str()
                    .ok_or_else(|| error("ACP select option has no current value"))?
                    .into(),
                options: choices(&option["options"], 0)?,
            });
        }
    } else if let Some(modes) = value.get("modes") {
        let options = modes["availableModes"]
            .as_array()
            .ok_or_else(|| error("Invalid ACP modes"))?
            .iter()
            .take(64)
            .map(|mode| {
                let id = mode["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| error("Invalid ACP mode id"))?;
                Ok(Choice {
                    value: id.into(),
                    name: mode["name"]
                        .as_str()
                        .unwrap_or(id)
                        .chars()
                        .take(200)
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        result.push(ConfigOption {
            id: LEGACY_MODE.into(),
            name: "Mode".into(),
            category: Some("mode".into()),
            current_value: modes["currentModeId"].as_str().unwrap_or_default().into(),
            options,
        });
    }
    Ok(result)
}
pub(super) fn is_model(option: &ConfigOption) -> bool {
    option.category.as_deref() == Some("model")
        || (option.category.is_none() && option.id == "model")
}
pub(super) fn is_thought(option: &ConfigOption) -> bool {
    option.category.as_deref() == Some("thought_level")
        || (option.category.is_none()
            && matches!(
                option.id.as_str(),
                "reasoning_effort" | "effort" | "thought_level"
            ))
}
pub(super) fn models(options: &[ConfigOption]) -> Option<(String, String, Vec<Model>)> {
    let option = options.iter().find(|option| is_model(option))?;
    let efforts = options
        .iter()
        .find(|option| is_thought(option))
        .map(|option| {
            option
                .options
                .iter()
                .map(|choice| choice.value.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some((
        option.id.clone(),
        option.current_value.clone(),
        option
            .options
            .iter()
            .map(|choice| Model {
                id: choice.value.clone(),
                name: choice.name.clone(),
                reasoning_efforts: if choice.value == option.current_value {
                    efforts.clone()
                } else {
                    vec![]
                },
            })
            .collect(),
    ))
}
