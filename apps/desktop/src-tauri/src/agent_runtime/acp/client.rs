//! Reverse ACP file/terminal services. Native agents retain tool and permission
//! ownership; filesystem requests share Nexa's path policy and file journal.
use super::{error, terminals::Terminals, Result};
use nexa_core::{
    agent::CancellationToken,
    db::Database,
    tools::{Tool, ToolExecutionContext},
    workspace::Workspace,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) struct FileContext {
    pub db: Arc<Database>,
    pub conversation: String,
    pub turn: String,
    pub workspace: Option<Workspace>,
    pub source_scope: Vec<String>,
    pub cancellation: CancellationToken,
    pub native_provider: Option<&'static str>,
}
impl FileContext {
    fn context<'a>(&'a self, id: &'a str, args: &'a str) -> ToolExecutionContext<'a> {
        let mut context = ToolExecutionContext::new(id, args, &self.db, &self.source_scope)
            .with_conversation_id(Some(&self.conversation))
            .with_turn_id(Some(&self.turn));
        context.workspace = self.workspace.as_ref();
        context.cancel_token = Some(&self.cancellation);
        context
    }
    fn path(params: &Value) -> Result<&Path> {
        params["path"]
            .as_str()
            .map(Path::new)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| error("ACP filesystem paths must be absolute"))
    }
    pub(super) fn terminal_cwd(&self, cwd: &str) -> Result<PathBuf> {
        let cwd = Path::new(cwd);
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(error(
                "ACP terminal cwd must be an existing absolute directory",
            ));
        }
        // Resolve a non-created child so directories use the same canonical
        // workspace/source policy as file writes, including symlink checks.
        let child = nexa_core::tools::resolve_agent_writable_file_path(
            &self.context("acp-cwd", "{}"),
            &cwd.join(".nexa-acp-cwd"),
        )?;
        child
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| error("Invalid ACP terminal cwd"))
    }
    pub(super) async fn handle(
        &self,
        id: &str,
        method: &str,
        params: &Value,
        cwd: &str,
        terminals: &Terminals,
    ) -> Result<Value> {
        if self.cancellation.is_cancelled() {
            return Err(error("ACP client request cancelled"));
        }
        match method {
            "fs/read_text_file" => {
                let requested = Self::path(params)?;
                if !requested.exists() {
                    // Agents read before creating. Resolve the missing target
                    // through policy first, then return ACP's ENOENT category.
                    nexa_core::tools::resolve_agent_writable_file_path(
                        &self.context(id, "{}"),
                        requested,
                    )?;
                    return Err(nexa_core::error::CoreError::NotFound(
                        "Resource not found".into(),
                    ));
                }
                let path = nexa_core::tools::resolve_agent_file_path(
                    &self.context(id, "{}"),
                    Self::path(params)?,
                )?;
                let line = params
                    .get("line")
                    .map(|v| {
                        v.as_u64()
                            .filter(|v| *v > 0)
                            .ok_or_else(|| error("ACP line must be a positive integer"))
                    })
                    .transpose()?
                    .unwrap_or(1);
                let limit = params
                    .get("limit")
                    .map(|v| {
                        v.as_u64()
                            .ok_or_else(|| error("ACP limit must be an integer"))
                    })
                    .transpose()?;
                tokio::task::spawn_blocking(move || {
                    use std::io::{BufRead, Read};
                    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
                    for _ in 0..line - 1 {
                        if reader.skip_until(b'\n')? == 0 {
                            return Ok(json!({"content":""}));
                        }
                    }
                    let mut bytes = Vec::new();
                    let mut bounded = reader.take(4 * 1024 * 1024 + 1);
                    for _ in 0..limit.unwrap_or(u64::MAX) {
                        if bounded.read_until(b'\n', &mut bytes)? == 0
                            || bytes.len() > 4 * 1024 * 1024
                        {
                            break;
                        }
                    }
                    if bytes.len() > 4 * 1024 * 1024 {
                        return Err(error(
                            "ACP text range exceeds the read limit; request fewer lines",
                        ));
                    }
                    let content = String::from_utf8(bytes)
                        .map_err(|_| error("ACP file is not UTF-8 text"))?;
                    Ok(json!({"content":content}))
                })
                .await
                .map_err(error)?
            }
            "fs/write_text_file" => {
                let path = Self::path(params)?;
                let content = params["content"]
                    .as_str()
                    .filter(|text| text.len() <= 4 * 1024 * 1024)
                    .ok_or_else(|| error("ACP write content must be bounded text"))?;
                let args = json!({"path":path,"content":content,"mode":"overwrite"}).to_string();
                let result = nexa_core::tools::create_file_tool::CreateFileTool
                    .execute(self.context(id, &args))
                    .await?;
                if result.is_error {
                    return Err(error(result.content));
                }
                Ok(json!({}))
            }
            "terminal/create" => {
                let cwd = params.get("cwd").and_then(Value::as_str).unwrap_or(cwd);
                terminals.create(params, &self.terminal_cwd(cwd)?, self.native_provider)
            }
            _ => {
                let id = params["terminalId"]
                    .as_str()
                    .ok_or_else(|| error("ACP terminalId is required"))?;
                terminals.request(method, id).await
            }
        }
    }
}

pub(super) fn is_service(method: &str) -> bool {
    matches!(
        method,
        "fs/read_text_file"
            | "fs/write_text_file"
            | "terminal/create"
            | "terminal/output"
            | "terminal/wait_for_exit"
            | "terminal/kill"
            | "terminal/release"
    )
}

pub(super) fn rpc_error(cause: &nexa_core::error::CoreError) -> Value {
    use nexa_core::error::CoreError;
    let missing = matches!(cause, CoreError::NotFound(_))
        || matches!(cause, CoreError::Io(error) if error.kind() == std::io::ErrorKind::NotFound);
    if missing {
        json!({"code":-32002,"message":"Resource not found"})
    } else {
        json!({"code":if matches!(cause, CoreError::InvalidInput(_) | CoreError::Agent(_)) { -32602 } else { -32603 },"message":cause.to_string()})
    }
}
