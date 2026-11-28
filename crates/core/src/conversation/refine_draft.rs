use crate::{
    db::Database,
    error::CoreError,
    llm::{create_provider, CompletionRequest, Message, ProviderConfig, Role},
};

/// A draft preview uses the explicitly selected model without creating a chat turn or exposing tools.
pub async fn refine_draft(
    db: &Database,
    config: ProviderConfig,
    model: &str,
    draft: &str,
) -> Result<String, CoreError> {
    if draft.trim().is_empty() {
        return Err(CoreError::InvalidInput("Enter a draft first.".into()));
    }
    let privacy = db.privacy_lease(&tokio_util::sync::CancellationToken::new())?;
    let provider_type = config.provider_type;
    let provider = create_provider(config)?;
    let request = CompletionRequest {
        model: model.to_owned(),
        messages: vec![
            Message::text(Role::System, "Edit the user's draft into a clearer prompt in its original language. Retain the user's facts, identifiers, quotations, constraints and requested scope. Clarify the desired outcome and form when the draft supports them. Return only the proposed prompt. Never answer or carry out the request."),
            Message::text(Role::User, privacy.policy.redact_text(draft)),
        ],
        tools: None, stop: None, temperature: None, max_tokens: None,
        thinking_budget: None, reasoning_enabled: None, reasoning_effort: None,
        provider_type: Some(provider_type), service_tier: None, routing_session_id: None, parallel_tool_calls: false,
    };
    let start = std::time::Instant::now();
    let response = tokio::select! {
        biased;
        _ = privacy.cancelled() => return Err(CoreError::Cancelled("Draft request cancelled after a privacy change".into())),
        response = privacy.scope(provider.complete(&request)) => response?,
    };
    db.record_model_step_usage(
        None,
        None,
        Some(&uuid::Uuid::new_v4().to_string()),
        None,
        None,
        0,
        Some(provider_type),
        model,
        "draft_refinement",
        Some(&response.usage),
        0,
        None,
        Some(start.elapsed().as_millis() as u64),
        None,
        None,
    )?;
    privacy.ensure_current()?;
    let text = response.content.trim();
    if text.is_empty() {
        return Err(CoreError::InvalidInput(
            "The selected model returned an empty draft.".into(),
        ));
    }
    Ok(text.to_owned())
}
