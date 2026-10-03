//! Bounded resumable compilation. Every source character enters a section;
//! Source revisions, compiler contracts and provider routes own saved results.
use super::*;
use rusqlite::{params, OptionalExtension};

struct Section {
    text: String,
    chars: usize,
    hash: String,
}

fn sections(content: &str) -> Vec<Section> {
    let mut result = Vec::new();
    let mut start = 0;
    while start < content.len() {
        let length = content[start..]
            .char_indices()
            .nth(COMPILE_INPUT_CHAR_BUDGET)
            .map(|(index, _)| index)
            .unwrap_or(content.len() - start);
        let text = content[start..start + length].to_owned();
        result.push(Section {
            chars: text.chars().count(),
            hash: blake3::hash(text.as_bytes()).to_hex().to_string(),
            text,
        });
        start += length;
    }
    result
}

fn request(
    model: &str,
    provider_type: Option<ProviderType>,
    text: &str,
    index: usize,
    total: usize,
) -> CompletionRequest {
    CompletionRequest {
        model: model.into(),
        messages: vec![Message::text(Role::System, COMPILE_SYSTEM_PROMPT), Message::text(Role::User, format!("Compile section {} of {total}. This is one bounded section; do not infer contents of other sections.\n\n{text}", index + 1))],
        max_tokens: None, temperature: Some(0.2), tools: None, stop: None, thinking_budget: None,
        reasoning_enabled: None, reasoning_effort: None, provider_type, routing_session_id: None, parallel_tool_calls: true,
    }
}

fn snapshot(db: &Database, doc_id: &str) -> Result<(String, String), CoreError> {
    let conn = db.conn();
    let revision = conn.query_row(
        "SELECT index_revision FROM documents WHERE id=?1",
        [doc_id],
        |row| row.get::<_, String>(0),
    )?;
    let mut statement = conn.prepare("SELECT content,COALESCE(json_extract(metadata_json,'$.overlap_start'),0) FROM chunks WHERE document_id=?1 AND kind!='summary' ORDER BY chunk_index")?;
    let contents = statement
        .query_map([doc_id], |row| {
            let content: String = row.get(0)?;
            let offset: usize = row.get(1)?;
            Ok(content.get(offset..).unwrap_or(&content).to_string())
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok((revision, contents.join("\n\n")))
}

fn same_revision(
    conn: &rusqlite::Connection,
    doc_id: &str,
    revision: &str,
) -> Result<(), CoreError> {
    let current: Option<String> = conn
        .query_row(
            "SELECT index_revision FROM documents WHERE id=?1",
            [doc_id],
            |row| row.get(0),
        )
        .optional()?;
    if current.as_deref() != Some(revision) {
        return Err(CoreError::Conflict("Document changed while compiling; completed work for the old revision was not applied.".into()));
    }
    Ok(())
}

fn normalize_quote(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn validate_output(output: &mut LlmCompileOutput, input: &str) -> Result<(), CoreError> {
    if output.summary.chars().count() > 5_000
        || output.entities.len() > 200
        || output.key_points.len() > 200
        || output.tags.len() > 100
    {
        return Err(CoreError::InvalidInput(
            "Compilation output exceeds the section result budget".into(),
        ));
    }
    let source = normalize_quote(input);
    output.entities.retain(|entity| {
        !entity.name.trim().is_empty()
            && !entity.context.trim().is_empty()
            && source.contains(&normalize_quote(&entity.context))
    });
    for entity in &mut output.entities {
        entity.relations.retain(|relation| {
            relation.confidence.is_none_or(|confidence| {
                confidence.is_finite() && (0.0..=1.0).contains(&confidence)
            }) && relation.evidence.as_deref().is_some_and(|quote| {
                !quote.trim().is_empty() && source.contains(&normalize_quote(quote))
            })
        });
        entity.relations.truncate(50);
    }
    Ok(())
}

pub(super) async fn compile_document(
    db: &Database,
    doc_id: &str,
    provider: &dyn LlmProvider,
    model: &str,
    provider_type: Option<ProviderType>,
    budget: &mut usize,
) -> Result<CompileResult, CoreError> {
    let (revision, content) = snapshot(db, doc_id)?;
    if content.trim().is_empty() {
        return Err(CoreError::InvalidInput("Document has no content".into()));
    }
    let sections = sections(&content);
    let route_request = request(model, provider_type, "", 0, sections.len());
    let route = provider.route_snapshot(&route_request);
    let route_key = blake3::hash(&serde_json::to_vec(&(
        COMPILE_CONTRACT_VERSION,
        COMPILE_SYSTEM_PROMPT,
        &route,
    ))?)
    .to_hex()
    .to_string();
    let mut outputs = Vec::new();
    let mut compiled = 0;
    let mut covered_chars = 0;
    for (index, section) in sections.iter().enumerate() {
        let saved: Option<String> = db.conn().query_row("SELECT output_json FROM document_section_compilations WHERE document_id=?1 AND revision=?2 AND route_key=?3 AND section_index=?4 AND input_hash=?5", params![doc_id,revision,route_key,index,section.hash], |row| row.get(0)).optional()?;
        let mut output = if let Some(saved) = saved {
            serde_json::from_str::<LlmCompileOutput>(&saved)?
        } else {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let request = request(model, provider_type, &section.text, index, sections.len());
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(180),
                provider.complete(&request),
            )
            .await
            .map_err(|_| CoreError::Llm("Document section compilation timed out".into()))??;
            if provider.route_snapshot(&request) != route {
                return Err(CoreError::Conflict(
                    "Compilation provider route changed; retry using the selected route.".into(),
                ));
            }
            if response.content.len() > 256 * 1024 {
                return Err(CoreError::InvalidInput(
                    "Compilation section response exceeds 256 KiB".into(),
                ));
            }
            let mut output: LlmCompileOutput = serde_json::from_str(response.content.trim())
                .map_err(|error| {
                    CoreError::InvalidInput(format!(
                        "LLM returned invalid compilation JSON: {error}"
                    ))
                })?;
            validate_output(&mut output, &section.text)?;
            let mut conn = db.conn();
            let transaction = conn.transaction()?;
            same_revision(&transaction, doc_id, &revision)?;
            transaction.execute("INSERT OR REPLACE INTO document_section_compilations(document_id,revision,route_key,section_index,input_hash,char_count,output_json) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![doc_id,revision,route_key,index,section.hash,section.chars,serde_json::to_string(&output)?])?;
            transaction.commit()?;
            compiled += 1;
            output
        };
        validate_output(&mut output, &section.text)?;
        covered_chars += section.chars;
        outputs.push(output);
    }
    let mut coverage = CompileCoverage {
        total_sections: sections.len(),
        completed_sections: outputs.len(),
        total_chars: content.chars().count(),
        covered_chars,
        complete: outputs.len() == sections.len(),
        summary_truncated: false,
    };
    let summary_text = outputs
        .iter()
        .enumerate()
        .map(|(index, output)| format!("Section {}: {}", index + 1, output.summary))
        .collect::<Vec<_>>()
        .join("\n\n");
    coverage.summary_truncated = summary_text.chars().count() > 48_000;
    let summary_text = summary_text.chars().take(48_000).collect::<String>();
    let unique = |items: Vec<String>| {
        let mut seen = HashSet::new();
        items
            .into_iter()
            .filter(|value| seen.insert(value.to_lowercase()))
            .collect::<Vec<_>>()
    };
    let key_points = unique(
        outputs
            .iter()
            .flat_map(|output| output.key_points.clone())
            .collect(),
    );
    let tags = unique(
        outputs
            .iter()
            .flat_map(|output| output.tags.clone())
            .collect(),
    );
    let mut conn = db.conn();
    let transaction = conn.transaction()?;
    same_revision(&transaction, doc_id, &revision)?;
    let summary = store_summary_on(
        &transaction,
        DocumentSummary {
            id: uuid::Uuid::new_v4().to_string(),
            document_id: doc_id.into(),
            summary: summary_text,
            key_points,
            tags,
            model_used: model.into(),
            compiled_at: chrono::Utc::now().to_rfc3339(),
            input_revision: revision,
            stale: false,
            coverage,
        },
    )?;
    transaction.execute(
        "DELETE FROM document_entities WHERE document_id=?1",
        [doc_id],
    )?;
    transaction.execute(
        "DELETE FROM entity_link_support WHERE document_id=?1",
        [doc_id],
    )?;
    if summary.coverage.complete {
        store_summary_chunk_on(
            &transaction,
            doc_id,
            &summary.summary,
            &summary.key_points,
            &summary.tags,
        )?;
    } else {
        transaction.execute(
            "DELETE FROM chunks WHERE document_id=?1 AND kind='summary'",
            [doc_id],
        )?;
    }
    let mut entities = HashMap::new();
    for entity in outputs.iter().flat_map(|output| &output.entities) {
        let value = upsert_entity_on(
            &transaction,
            &entity.name,
            &entity.aliases,
            &parse_entity_type(&entity.entity_type),
            &entity.description,
            doc_id,
        )?;
        transaction.execute("INSERT INTO document_entities(document_id,entity_id,relevance,context_snippet) VALUES(?1,?2,1.0,?3) ON CONFLICT(document_id,entity_id) DO UPDATE SET context_snippet=excluded.context_snippet", params![doc_id,value.id,entity.context])?;
        entities.insert(normalize_entity_lookup_name(&entity.name), value.id);
    }
    let mut links = HashSet::new();
    for entity in outputs.iter().flat_map(|output| &output.entities) {
        let Some(source) = entities.get(&normalize_entity_lookup_name(&entity.name)) else {
            continue;
        };
        for relation in &entity.relations {
            let Some(target) = entities.get(&normalize_entity_lookup_name(&relation.target)) else {
                continue;
            };
            let relation_type = normalize_relation_type(&relation.relation_type);
            if !links.insert((source.clone(), target.clone(), relation_type.clone())) {
                continue;
            }
            upsert_entity_link_on(
                &transaction,
                source,
                target,
                &relation_type,
                EntityLinkEvidence {
                    strength: relation.confidence.unwrap_or(0.5),
                    evidence_doc: Some(doc_id),
                    evidence_snippet: relation.evidence.as_deref(),
                    confidence: relation.confidence,
                },
            )?;
        }
    }
    transaction.execute("UPDATE entities SET mention_count=(SELECT COUNT(*) FROM document_entities WHERE entity_id=entities.id) WHERE id IN (SELECT entity_id FROM document_entities WHERE document_id=?1)", [doc_id])?;
    transaction.commit()?;
    Ok(CompileResult {
        document_id: doc_id.into(),
        summary,
        entities_found: entities.len(),
        links_created: links.len(),
        sections_compiled: compiled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grounded_relations_reject_invalid_confidence_without_inventing_certainty() {
        let mut output: LlmCompileOutput = serde_json::from_value(serde_json::json!({
            "summary":"A protects B", "key_points":[], "tags":[], "entities":[{
                "name":"A","entity_type":"concept","description":"A","context":"A protects B",
                "relations":[{"target":"B","relation_type":"protects","evidence":"A protects B","confidence":0.5}]
            }]
        })).unwrap();
        let template = output.entities[0].relations[0].clone();
        output.entities[0].relations = [
            None,
            Some(0.0),
            Some(1.0),
            Some(-1.0),
            Some(85.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ]
        .into_iter()
        .map(|confidence| LlmRelation {
            confidence,
            ..template.clone()
        })
        .collect();
        validate_output(&mut output, "A protects B").unwrap();
        assert_eq!(output.entities[0].relations.len(), 3);
        assert_eq!(
            output.entities[0]
                .relations
                .iter()
                .map(|relation| relation.confidence)
                .collect::<Vec<_>>(),
            vec![None, Some(0.0), Some(1.0)]
        );
    }
    #[test]
    fn sections_cover_every_character_without_sampling_gaps() {
        let content = format!(
            "{}{}{}",
            "开始".repeat(5_000),
            "中段".repeat(5_000),
            "结尾".repeat(5_000)
        );
        let parts = sections(&content);
        assert!(parts.len() > 1);
        assert!(parts
            .iter()
            .all(|part| part.chars <= COMPILE_INPUT_CHAR_BUDGET));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            content
        );
    }
}
