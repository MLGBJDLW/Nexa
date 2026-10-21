//! Durable, cancellable context-maintenance operations.
//!
//! The external seam is intentionally small: start, observe, cancel, and load
//! the active context projection. Provider transport, planning, fallback,
//! persistence, leases, and terminal-event rules stay behind it.

mod live;
mod model;
mod planner;
mod service;
mod store;

pub use live::ContextForkRequest;
pub use model::{
    ContextCompactionHandle, ContextCompactionJob, ContextCompactionPhase, ContextCompactionPolicy,
    ContextCompactionResult, ContextProjection, StartContextCompactionRequest,
};
pub use service::ContextCompactionService;
pub use store::load_context_projection;

/// Automatic handoffs and manual maintenance publish the same durable context
/// projection. Archiving alone would reintroduce the old trajectory next turn.
pub(crate) fn commit_live_history_handoff(
    database: &crate::db::Database,
    archive: crate::context_history::ContextHistoryArchive,
    model: &str,
    cancellations: &[&tokio_util::sync::CancellationToken],
) -> Result<bool, crate::error::CoreError> {
    let snapshot = store::load_compaction_snapshot(database, &archive.conversation_id)?;
    let has_canonical_trajectory = snapshot.messages.iter().any(|message| {
        matches!(
            message.role,
            crate::llm::Role::Assistant | crate::llm::Role::Tool
        )
    });
    let plan = planner::plan_compaction(
        snapshot.messages,
        model,
        None,
        0,
        snapshot.checkpoint_generation,
        crate::context_history::ContextManagementMode::History,
    );
    let planner::PlanOutcome::Planned(plan) = plan else {
        if has_canonical_trajectory {
            return Ok(false);
        }
        // A caller with no persisted assistant history still needs a verified
        // archive, but there is no canonical trajectory to suppress on reopen.
        database.archive_context_history(&archive, cancellations)?;
        return Ok(true);
    };
    let summary = archive.checkpoint_text();
    let operation_id = format!("history-handoff:{}", archive.id);
    let input = model::ContextCheckpointInput {
        conversation_id: archive.conversation_id.clone(),
        history_archive: Some(archive),
        operation_id: operation_id.clone(),
        idempotency_key: operation_id,
        snapshot_high_watermark: plan.snapshot_high_watermark,
        source_message_ids: plan.source_message_ids,
        source_start_sort_order: plan.source_start_sort_order,
        source_boundary_sort_order: plan.source_boundary_sort_order,
        source_digest: plan.source_digest,
        expected_checkpoint_generation: plan.expected_checkpoint_generation,
        tokens_before: plan.tokens_before,
        tokens_after: plan.retained_tokens.saturating_add(
            crate::conversation::memory::estimate_tokens_for_model(model, &summary),
        ),
        summary,
        retained_tail_message_ids: plan.retained_tail_message_ids,
        retained_start_sort_order: plan.retained_start_sort_order,
        provider: "local-history".into(),
        provider_type: None,
        model: model.into(),
        usage: None,
    };
    Ok(matches!(
        store::commit_context_checkpoint_cancellable(database, &input, cancellations)?,
        store::CommitOutcome::Committed { .. }
    ))
}
