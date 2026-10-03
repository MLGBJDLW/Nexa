//! ChunkContextTool — retrieves a chunk and its surrounding context from the same document.

use std::sync::OnceLock;

use async_trait::async_trait;
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;

use crate::error::CoreError;

use super::{current_scope_miss_message, ensure_source_in_scope, Tool, ToolDef, ToolResult};

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/get_chunk_context.json");

/// Tool that retrieves a chunk and its surrounding chunks from the same document,
/// ordered by `chunk_index`.
pub struct ChunkContextTool;

#[derive(Deserialize)]
struct ChunkContextArgs {
    chunk_id: String,
    #[serde(default = "default_context_chunks")]
    context_chunks: usize,
}

fn default_context_chunks() -> usize {
    2
}

#[async_trait]
impl Tool for ChunkContextTool {
    fn name(&self) -> &str {
        "get_chunk_context"
    }

    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        ToolDef::from_json(&DEF, DEF_JSON).parameters.clone()
    }

    async fn execute(
        &self,
        context: crate::tools::ToolExecutionContext<'_>,
    ) -> Result<ToolResult, CoreError> {
        let crate::tools::ToolExecutionContext {
            call_id,
            arguments,
            db,
            source_scope,
            ..
        } = context;
        let args: ChunkContextArgs = serde_json::from_str(arguments).map_err(|e| {
            CoreError::InvalidInput(format!("Invalid get_chunk_context arguments: {e}"))
        })?;

        let db = db.clone();
        let call_id = call_id.to_string();
        let source_scope = source_scope.to_vec();
        tokio::task::spawn_blocking(move || {
            let card=match crate::search::get_evidence_card(&db,&args.chunk_id) {
                Ok(card)=>card,
                Err(CoreError::NotFound(_))=>return Ok(ToolResult {call_id,content:format!("Chunk '{}' not found.",args.chunk_id),is_error:true,artifacts:None}),
                Err(error)=>return Err(error),
            };
            if ensure_source_in_scope(&card.source_id.to_string(),&source_scope).is_err() {
                return Ok(ToolResult {call_id,content:current_scope_miss_message().to_string(),is_error:true,artifacts:None});
            }
            let reference=card.evidence_ref.as_ref().ok_or_else(||CoreError::NotFound("Evidence reference unavailable".into()))?;
            let context=crate::evidence::context(&db,reference,args.context_chunks)?;
            let total:usize=db.conn().query_row("SELECT COUNT(DISTINCT chunk_index) FROM evidence_records WHERE source_id=?1 AND document_id=?2 AND revision=?3",params![reference.source_id.to_string(),reference.document_id.to_string(),reference.revision],|row|row.get(0))?;
            let mut text=format!("Document: {}\nPath: {}\nIndexed version: {} ({})\nShowing {} of {total} chunks from this version.\n\n",card.document_title,card.document_path,reference.revision,reference.status,context.cards.len());
            let mut artifacts=Vec::new();
            for neighbor in &context.cards {
                let is_target=neighbor.chunk_id==card.chunk_id;
                text.push_str(&format!("--- [{} (index {}, kind: {})] ---\n[chunk_id: {}]\n{}\n\n",if is_target{"TARGET CHUNK"}else{"context chunk"},neighbor.chunk_index,neighbor.chunk_kind,neighbor.chunk_id,neighbor.content));
                let mut value=serde_json::to_value(neighbor)?;value["isTarget"]=json!(is_target);artifacts.push(value);
            }
            if context.truncated {text.push_str("The context text reached the 64000-character budget. Read the source for omitted text.\n");}
            Ok(ToolResult {call_id,content:text,is_error:false,artifacts:Some(json!({"documentId":card.document_id,"documentPath":card.document_path,"documentTitle":card.document_title,"sourceId":card.source_id,"targetChunkIndex":card.chunk_index,"totalChunks":total,"evidenceRef":reference,"truncated":context.truncated,"chunks":artifacts}))})

        })
        .await
        .map_err(|e| CoreError::Internal(format!("task join failed: {e}")))?
    }
}
