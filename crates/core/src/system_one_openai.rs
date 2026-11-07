use super::*;

fn text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

pub(super) fn encode(request: &DecisionRequest) -> Result<Value, CoreError> {
    if request.model.as_deref() != Some("gpt-6-luna") || request.questions.len() > 400 {
        return Err(CoreError::InvalidInput(
            "OpenAI Decisions requires GPT-6 Luna and at most 400 questions.".into(),
        ));
    }
    let mut questions = Vec::with_capacity(request.questions.len());
    for (name, question) in &request.questions {
        questions.push(match question {
            Question::Noul { instructions, criteria } => {
                let mut instruction = text(instructions);
                if let Some(criteria) = criteria {
                    instruction.push_str(&format!("\nPredicate criteria: {}", json!(criteria)));
                }
                json!({"name":name,"type":"predicate","instructions":instruction})
            }
            Question::Choice { instructions, criteria } => {
                if criteria.len() < 2 {
                    return Err(CoreError::InvalidInput("OpenAI Decisions requires at least two choices.".into()));
                }
                let choices = criteria.iter().map(|(key, value)| {
                    json!({"value":key,"description":text(value)})
                }).collect::<Vec<_>>();
                json!({"name":name,"type":"choice","instructions":text(instructions),"choices":choices})
            }
            Question::Score { instructions, criteria } => {
                let levels = criteria.iter().enumerate().map(|(index,value)| {
                    json!({"label":format!("Level {index}"),"description":text(value)})
                }).collect::<Vec<_>>();
                json!({"name":name,"type":"score","instructions":text(instructions),"levels":levels})
            }
        });
    }
    Ok(
        json!({"model":request.model,"input":text(&request.state),"questions":questions,
        "prompt_cache_options":{"mode":"explicit"}}),
    )
}

pub(super) fn decode(
    bytes: &[u8],
    request: &DecisionRequest,
) -> Result<DecisionResponse, CoreError> {
    let invalid = || {
        CoreError::InvalidInput("OpenAI Decisions returned an invalid structured response.".into())
    };
    let result: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let mut answers = BTreeMap::new();
    for row in result["answers"].as_array().ok_or_else(invalid)? {
        let name = row["name"].as_str().ok_or_else(invalid)?;
        let question = request.questions.get(name).ok_or_else(invalid)?;
        let answer = match (row["type"].as_str(), question) {
            (Some("refusal"), _) => {
                return Err(CoreError::Llm(
                    "OpenAI declined to answer a decision question.".into(),
                ))
            }
            (Some("predicate"), Question::Noul { .. }) => Answer::Noul {
                noul: row["probability"].as_f64().ok_or_else(invalid)?,
            },
            (Some("choice"), Question::Choice { .. }) => Answer::Choice {
                choice: row["choice"].as_str().ok_or_else(invalid)?.into(),
                probabilities: probabilities(row, false)?,
                confidence: row["confidence"].as_f64().ok_or_else(invalid)?,
            },
            (Some("score"), Question::Score { criteria, .. }) => Answer::Score {
                score: row["score"].as_f64().ok_or_else(invalid)?,
                probabilities: probabilities(row, true)?,
                confidence: row["confidence"].as_f64().ok_or_else(invalid)?,
                legend: criteria
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (index.to_string(), value.clone()))
                    .collect(),
            },
            _ => return Err(invalid()),
        };
        if answers.insert(name.to_string(), answer).is_some() {
            return Err(invalid());
        }
    }
    Ok(DecisionResponse {
        model: result["model"].as_str().ok_or_else(invalid)?.into(),
        answers,
        usage: serde_json::from_value(result["usage"].clone()).map_err(|_| invalid())?,
        annotations: Vec::new(),
    })
}

fn probabilities(row: &Value, numeric: bool) -> Result<BTreeMap<String, f64>, CoreError> {
    let invalid = || {
        CoreError::InvalidInput(
            "OpenAI Decisions returned an invalid probability distribution.".into(),
        )
    };
    let mut values = BTreeMap::new();
    for item in row["probabilities"].as_array().ok_or_else(invalid)? {
        let label = if numeric {
            item["value"].as_u64().map(|value| value.to_string())
        } else {
            item["value"].as_str().map(str::to_owned)
        }
        .ok_or_else(invalid)?;
        if values
            .insert(label, item["probability"].as_f64().ok_or_else(invalid)?)
            .is_some()
        {
            return Err(invalid());
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_schema_roundtrips_all_three_decision_types() {
        let request: DecisionRequest = serde_json::from_value(json!({
            "model":"gpt-6-luna","state":{"message":"hello"},"questions":{
                "q0":{"type":"noul","instructions":"Does it greet?"},
                "q1":{"type":"choice","instructions":"Select topic","criteria":{"c0":"greeting","c1":"other"}},
                "q2":{"type":"score","instructions":"Tone","criteria":["cold","friendly"]}
            }
        })).unwrap();
        let wire = encode(&request).unwrap();
        assert_eq!(wire["questions"][0]["type"], "predicate");
        assert_eq!(wire["questions"][1]["choices"][0]["value"], "c0");
        assert_eq!(wire["questions"][2]["levels"][1]["description"], "friendly");
        let response = json!({"model":"gpt-6-luna","usage":{"input_tokens":123,"output_tokens":0},"answers":[
            {"name":"q0","type":"predicate","probability":0.9},
            {"name":"q1","type":"choice","choice":"c0","confidence":0.8,"probabilities":[
                {"value":"c0","probability":0.8},{"value":"c1","probability":0.2}]},
            {"name":"q2","type":"score","score":0.75,"confidence":0.8,"probabilities":[
                {"value":0,"probability":0.25},{"value":1,"probability":0.75}]}
        ]});
        decode(response.to_string().as_bytes(), &request)
            .unwrap()
            .validate(&request)
            .unwrap();
    }
}
