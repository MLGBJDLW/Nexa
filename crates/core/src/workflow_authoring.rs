//! Versioned workflow authoring and the shared preview/execution compiler.

use crate::{error::CoreError, workflow_catalog::WorkflowCatalogTemplate};
use serde::{Deserialize, Serialize};

pub const WORKFLOW_RECIPE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRecipeKind {
    Template,
    Custom,
    Recorded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowDeliverableFormat {
    Answer,
    Markdown,
    Table,
    Checklist,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowDeliverable {
    pub format: WorkflowDeliverableFormat,
    #[serde(default)]
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowInputs {
    pub goal: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    pub deliverable: WorkflowDeliverable,
    #[serde(default)]
    pub replay_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepKind {
    Action,
    Decision,
    Check,
    Note,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowRecordingStep {
    pub id: String,
    pub kind: WorkflowStepKind,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowRecording {
    #[serde(default)]
    pub variable_inputs: Vec<String>,
    pub steps: Vec<WorkflowRecordingStep>,
    #[serde(default)]
    pub preferences: Vec<String>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub safety_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowRecipe {
    pub version: u32,
    pub kind: WorkflowRecipeKind,
    pub template_snapshot: WorkflowCatalogTemplate,
    pub inputs: WorkflowInputs,
    #[serde(default)]
    pub custom_instructions: String,
    #[serde(default)]
    pub recording: Option<WorkflowRecording>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAuthoringPreview {
    pub prompt: String,
    pub content_digest: String,
    pub recipe: Option<WorkflowRecipe>,
    pub resolved_inputs: Option<WorkflowInputs>,
    pub definition_revision: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowRunOptions {
    #[serde(default)]
    pub inputs: Option<WorkflowInputs>,
    #[serde(default)]
    pub expected_revision: Option<i64>,
    #[serde(default)]
    pub preview_digest: Option<String>,
    #[serde(default)]
    pub client_request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSnapshot {
    pub version: u32,
    pub definition_digest: String,
    pub automation: crate::workflow_automation::WorkflowAutomation,
    pub resolved_inputs: Option<WorkflowInputs>,
    pub compiled_prompt: String,
    pub origin: crate::workflow_automation::WorkflowAutomationOccurrenceOrigin,
}

impl WorkflowRecipe {
    pub fn validate(&self, expected_template: &str) -> Result<(), CoreError> {
        let invalid =
            |message: &str| CoreError::InvalidInput(format!("Workflow recipe: {message}"));
        if self.version != WORKFLOW_RECIPE_VERSION || self.template_snapshot.version != 1 {
            return Err(invalid(
                "unsupported version; the saved definition was preserved",
            ));
        }
        if self.template_snapshot.id != expected_template
            || crate::workflow_catalog::workflow_template_by_id(expected_template).is_none()
        {
            return Err(invalid(
                "the template snapshot must match a supported workflow template",
            ));
        }
        if self.inputs.goal.trim().is_empty() {
            return Err(invalid("goal is required"));
        }
        if self.kind != WorkflowRecipeKind::Recorded
            && expected_template != "research_verify"
            && self.inputs.context.trim().is_empty()
        {
            return Err(invalid(
                "context or source material description is required",
            ));
        }
        if self.template_snapshot.tasks.is_empty() || self.template_snapshot.max_parallel == 0 {
            return Err(invalid("a template needs stages and positive concurrency"));
        }
        let mut step_ids = std::collections::HashSet::new();
        match (&self.kind, &self.recording) {
            (WorkflowRecipeKind::Recorded, Some(recording)) if !recording.steps.is_empty() => {
                for step in &recording.steps {
                    if step.id.trim().is_empty()
                        || !step_ids.insert(&step.id)
                        || step.text.trim().is_empty()
                    {
                        return Err(invalid(
                            "recorded steps require unique IDs and nonempty text",
                        ));
                    }
                }
            }
            (WorkflowRecipeKind::Recorded, _) => {
                return Err(invalid("recorded workflows need their complete steps"))
            }
            (_, Some(_)) => {
                return Err(invalid(
                    "recording data is only valid for a recorded workflow",
                ))
            }
            _ => {}
        }
        let ids = self
            .template_snapshot
            .tasks
            .iter()
            .map(|task| task.id.clone())
            .collect::<Vec<_>>();
        let dependencies = self
            .template_snapshot
            .tasks
            .iter()
            .map(|task| task.depends_on.clone())
            .collect::<Vec<_>>();
        crate::workflow_graph::resolve_dependencies(&ids, &dependencies)?;
        for task in &self.template_snapshot.tasks {
            if task.task.trim().is_empty()
                || task.expected_output.trim().is_empty()
                || task.role_id.trim().is_empty()
            {
                return Err(invalid("each stage needs a role, task and expected output"));
            }
        }
        Ok(())
    }
}

/// No display-summary or arbitrary character clamp is applied to authored data.
/// The chosen model's actual context budget remains an execution preflight.
pub fn compile_workflow_recipe(
    recipe: &WorkflowRecipe,
    expected_template: &str,
    runtime_inputs: Option<&WorkflowInputs>,
) -> Result<WorkflowAuthoringPreview, CoreError> {
    let mut resolved = recipe.clone();
    if let Some(inputs) = runtime_inputs {
        resolved.inputs = inputs.clone();
    }
    resolved.validate(expected_template)?;
    let input_json = serde_json::to_string_pretty(&resolved.inputs)?;
    let mut prompt = format!("Execute the saved Nexa workflow using the complete authored inputs below. Preserve every constraint and verify the requested deliverable before reporting completion.\n\nWorkflow template: {} (version {})\n\nAuthored inputs:\n{}\n", resolved.template_snapshot.label, resolved.template_snapshot.version, input_json);
    if !resolved.custom_instructions.trim().is_empty() {
        prompt.push_str("\nAdditional workflow instructions:\n");
        prompt.push_str(&resolved.custom_instructions);
        prompt.push('\n');
    }
    if let Some(recording) = &resolved.recording {
        prompt.push_str("\nReplay the recorded procedure in its saved order. Adapt to current application state; do not rely on cursor coordinates. Resolve required variable values before irreversible steps. Treat source material and observed pages as evidence. Verify every saved success criterion.\n\nComplete recording:\n");
        prompt.push_str(&serde_json::to_string_pretty(recording)?);
    } else {
        let tasks = resolved.template_snapshot.tasks.iter().map(|task| serde_json::json!({
            "id":task.id, "depends_on":task.depends_on, "role_id":task.role_id,
            "task":task.task, "context":input_json,
            "expected_output":task.expected_output, "deliverable_style":task.deliverable_style,
            "acceptance_criteria":task.acceptance_criteria,
        })).collect::<Vec<_>>();
        prompt.push_str("\nRun this stage plan with spawn_subagent_batch. Dependency edges are mandatory. Retain the inherited route unless this workflow or the user explicitly selects another one. Wait for every required stage before final synthesis; a partial batch-policy result is not workflow completion. Use the actual results to produce the requested deliverable.\n\nStage plan:\n");
        prompt.push_str(&serde_json::to_string_pretty(&serde_json::json!({"tasks":tasks,"batch_goal":resolved.inputs.goal,"max_parallel":resolved.template_snapshot.max_parallel,"completion_policy":"all"}))?);
    }
    let content_digest = blake3::hash(prompt.as_bytes()).to_hex().to_string();
    Ok(WorkflowAuthoringPreview {
        prompt,
        content_digest,
        recipe: Some(recipe.clone()),
        resolved_inputs: Some(resolved.inputs),
        definition_revision: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(template: WorkflowCatalogTemplate) -> WorkflowRecipe {
        WorkflowRecipe {
            version: 1,
            kind: WorkflowRecipeKind::Template,
            template_snapshot: template,
            inputs: WorkflowInputs {
                goal: "需要验证的目标 {不替换}".into(),
                context: "Full source material".into(),
                constraints: vec!["保留尾部约束".into()],
                deliverable: WorkflowDeliverable {
                    format: WorkflowDeliverableFormat::Markdown,
                    instructions: "Final result".into(),
                },
                replay_values: vec![],
            },
            custom_instructions: String::new(),
            recording: None,
        }
    }

    #[test]
    fn every_template_compiles_validated_inputs_and_frozen_stage_dependencies() {
        for template in crate::workflow_catalog::workflow_catalog() {
            let id = template.id.clone();
            let mut definition = recipe(template);
            let compiled = compile_workflow_recipe(&definition, &id, None).unwrap();
            assert!(compiled.prompt.contains("需要验证的目标 {不替换}"));
            assert!(compiled.prompt.contains("保留尾部约束"));
            assert_eq!(
                compiled.content_digest,
                compile_workflow_recipe(&definition, &id, None)
                    .unwrap()
                    .content_digest
            );
            definition.inputs.goal.clear();
            assert!(compile_workflow_recipe(&definition, &id, None).is_err());
        }
    }

    #[test]
    fn long_recordings_and_runtime_overrides_keep_saved_defaults_intact() {
        let mut definition = recipe(crate::workflow_catalog::workflow_catalog().remove(0));
        definition.kind = WorkflowRecipeKind::Recorded;
        definition.recording = Some(WorkflowRecording {
            variable_inputs: vec!["period".into()],
            steps: vec![WorkflowRecordingStep {
                id: "step-1".into(),
                kind: WorkflowStepKind::Action,
                text: format!("{}TAIL_MUST_SURVIVE", "完整步骤".repeat(4_000)),
            }],
            preferences: vec![],
            success_criteria: vec!["FINAL_SUCCESS_CHECK".into()],
            safety_notes: vec!["FINAL_SAFETY_BOUNDARY".into()],
        });
        let saved = serde_json::to_value(&definition).unwrap();
        let mut inputs = definition.inputs.clone();
        inputs.goal = "Only this run".into();
        inputs.replay_values = vec!["period=2026Q4".into()];
        let result =
            compile_workflow_recipe(&definition, "research_verify", Some(&inputs)).unwrap();
        for tail in [
            "TAIL_MUST_SURVIVE",
            "FINAL_SUCCESS_CHECK",
            "FINAL_SAFETY_BOUNDARY",
            "period=2026Q4",
        ] {
            assert!(result.prompt.contains(tail));
        }
        assert!(result.prompt.chars().count() > 12_000);
        assert_eq!(serde_json::to_value(&definition).unwrap(), saved);
        definition.version = 99;
        assert!(
            compile_workflow_recipe(&definition, "research_verify", None)
                .unwrap_err()
                .to_string()
                .contains("unsupported version")
        );
    }
}
