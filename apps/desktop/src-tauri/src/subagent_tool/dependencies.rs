use super::*;

#[derive(Debug, Clone)]
pub(super) struct BatchDependencyGate {
    pub batch_id: String,
    pub indices: Vec<usize>,
}

#[derive(Debug, Clone)]
pub(super) struct WorkflowDependencyResult {
    pub worker_id: String,
    pub result: String,
    pub source_scope: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WorkflowDependencyEvidence {
    worker_id: String,
    content_digest: String,
    result_chars: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct WorkflowStageHandoff {
    pub depends_on: Vec<String>,
    pub gate: Option<BatchDependencyGate>,
    pub results: Vec<WorkflowDependencyResult>,
}

impl WorkflowStageHandoff {
    pub fn evidence(&self) -> Vec<WorkflowDependencyEvidence> {
        self.results
            .iter()
            .map(|result| WorkflowDependencyEvidence {
                worker_id: result.worker_id.clone(),
                content_digest: blake3::hash(result.result.as_bytes()).to_hex().to_string(),
                result_chars: result.result.chars().count(),
            })
            .collect()
    }
}

impl DelegationRuntime {
    fn dependency_results(
        &self,
        gate: &BatchDependencyGate,
    ) -> Result<Option<Vec<WorkflowDependencyResult>>, CoreError> {
        let batches = self
            .batches
            .lock()
            .map_err(|_| CoreError::Internal("Workflow batch state unavailable".into()))?;
        let batch = batches
            .get(&gate.batch_id)
            .ok_or_else(|| CoreError::NotFound(format!("Delegated batch {}", gate.batch_id)))?;
        let mut results = Vec::with_capacity(gate.indices.len());
        for index in &gate.indices {
            if *index >= batch.expected_workers {
                return Err(CoreError::InvalidInput(
                    "Dependency index is outside this batch".into(),
                ));
            }
            if let Some(run) = batch.results.get(index) {
                if run.is_error || !matches!(run.status.as_str(), "done" | "completed") {
                    return Err(CoreError::Agent(format!("dependency_failed: prerequisite {} did not succeed; this stage was not executed", run.id)));
                }
                results.push(WorkflowDependencyResult {
                    worker_id: run.id.clone(),
                    result: run.result.clone(),
                    source_scope: run.effective_source_scope.clone(),
                });
            }
        }
        Ok((results.len() == gate.indices.len()).then_some(results))
    }

    pub(super) async fn wait_for_dependencies(
        &self,
        gate: &BatchDependencyGate,
        cancellation: &CancellationToken,
    ) -> Result<Vec<WorkflowDependencyResult>, CoreError> {
        let changed = self
            .batch_notification(&gate.batch_id)
            .ok_or_else(|| CoreError::NotFound(format!("Delegated batch {}", gate.batch_id)))?;
        loop {
            let notified = changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if cancellation.is_cancelled() {
                return Err(CoreError::Cancelled(
                    "Workflow stage cancelled while awaiting dependencies".into(),
                ));
            }
            if let Some(results) = self.dependency_results(gate)? {
                return Ok(results);
            }
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(CoreError::Cancelled("Workflow stage cancelled while awaiting dependencies".into())),
                _ = &mut notified => {},
            }
        }
    }
}

pub(super) fn resolve_batch_dependencies(
    tasks: &[(Option<String>, SpawnSubagentArgs)],
) -> Result<Vec<Vec<usize>>, CoreError> {
    let ids = tasks
        .iter()
        .map(|(id, _)| id.clone().unwrap_or_default())
        .collect::<Vec<_>>();
    let dependencies = tasks
        .iter()
        .map(|(_, args)| args.stage_handoff.depends_on.clone())
        .collect::<Vec<_>>();
    nexa_core::workflow_graph::resolve_dependencies(&ids, &dependencies)
}

pub(super) struct BatchWorkerSettlement {
    pub runtime: DelegationRuntime,
    pub batch_id: String,
    pub index: usize,
    pub agent_id: String,
    pub cancellation: CancellationToken,
    pub label: String,
    pub fallback: SpawnSubagentArgs,
    pub parallel_group: Option<String>,
}

impl BatchWorkerSettlement {
    pub fn monitor(
        self,
        worker: tokio::task::JoinHandle<(usize, SubagentRunArtifact)>,
    ) -> tokio::task::JoinHandle<(usize, SubagentRunArtifact)> {
        tokio::spawn(async move {
            match worker.await {
                Ok(result) => result,
                Err(join_error) => {
                    let error = CoreError::Agent(format!(
                        "Delegated worker task terminated unexpectedly: {join_error}"
                    ));
                    let _ = settle_worker_lifecycle(
                        &self.runtime.lifecycle,
                        &self.agent_id,
                        &self.cancellation,
                        Err(&error),
                    )
                    .await;
                    let run = failed_subagent_run_artifact(
                        self.label,
                        self.fallback,
                        self.parallel_group,
                        &error,
                    );
                    self.runtime
                        .record_batch_result(&self.batch_id, self.index, run.clone());
                    (self.index, run)
                }
            }
        })
    }
}
