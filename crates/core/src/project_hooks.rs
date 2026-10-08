//! Explicitly enabled, project-owned lifecycle checks. Repository files never enable execution.
use crate::{
    approval::ToolApprovalMode,
    db::Database,
    error::CoreError,
    tools::{ToolExecutionContext, ToolRegistry},
    workspace::Workspace,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    AfterFileChange,
    BeforeComplete,
}
impl HookEvent {
    fn name(self) -> &'static str {
        match self {
            Self::AfterFileChange => "after_file_change",
            Self::BeforeComplete => "before_complete",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectHook {
    pub id: String,
    pub name: String,
    pub manifest_hash: String,
    pub event: HookEvent,
    pub arguments: Value,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookRun {
    pub id: String,
    pub hook_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub event: String,
    pub status: String,
    pub detail: String,
    pub created_at: String,
    pub completed_file_revision: Option<u64>,
}

impl Database {
    pub fn recover_project_hook_runs(&self) -> Result<usize, CoreError> {
        Ok(self.conn().execute("UPDATE project_hook_runs SET status='cancelled',detail='The application restarted during execution; no automatic replay is permitted.' WHERE status='running'", [])?)
    }

    pub fn project_hooks(&self, project_id: &str) -> Result<Vec<ProjectHook>, CoreError> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT config_json FROM project_hooks WHERE project_id=?1 ORDER BY created_at,id",
        )?;
        let rows = statement
            .query_map([project_id], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect();
        rows
    }

    /// Only host UI commands call this. The model-facing tool registry has no enable action.
    pub fn save_project_hook(
        &self,
        project_id: &str,
        mut hook: ProjectHook,
    ) -> Result<ProjectHook, CoreError> {
        let project = self.get_project(project_id)?;
        if hook.name.trim().is_empty()
            || hook.name.len() > 120
            || hook.manifest_hash.len() != 64
            || !hook.arguments.is_object()
            || serde_json::to_vec(&hook.arguments)?.len() > 16_384
        {
            return Err(CoreError::InvalidInput("A hook needs a discovered tool, its full manifest hash, and a JSON argument object of at most 16 KiB.".into()));
        }
        if hook.enabled {
            let workspace = Workspace::validate(&project.workspace_roots.unwrap_or_default())?;
            crate::tools::project_tool::validate_project_hook(
                self,
                &workspace,
                &hook.name,
                &hook.manifest_hash,
                &hook.arguments,
            )?;
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if hook.id.is_empty() {
            let count: usize = tx.query_row(
                "SELECT COUNT(*) FROM project_hooks WHERE project_id=?1",
                [project_id],
                |row| row.get(0),
            )?;
            if count >= 16 {
                return Err(CoreError::InvalidInput(
                    "A project supports at most 16 lifecycle hooks.".into(),
                ));
            }
            hook.id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO project_hooks(id,project_id,config_json) VALUES(?1,?2,?3)",
                params![hook.id, project_id, serde_json::to_string(&hook)?],
            )?;
        } else {
            let changed = tx.execute(
                "UPDATE project_hooks SET config_json=?3 WHERE id=?1 AND project_id=?2",
                params![hook.id, project_id, serde_json::to_string(&hook)?],
            )?;
            if changed == 0 {
                return Err(CoreError::NotFound("Project hook not found".into()));
            }
        }
        tx.commit()?;
        Ok(hook)
    }

    pub fn delete_project_hook(&self, project_id: &str, id: &str) -> Result<(), CoreError> {
        self.conn().execute(
            "DELETE FROM project_hooks WHERE id=?1 AND project_id=?2",
            params![id, project_id],
        )?;
        Ok(())
    }

    pub fn project_hook_runs(&self, project_id: &str) -> Result<Vec<HookRun>, CoreError> {
        let conn = self.conn();
        let mut statement = conn.prepare("SELECT id,hook_id,conversation_id,turn_id,event,status,detail,created_at,completed_file_revision FROM project_hook_runs WHERE project_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 100")?;
        let rows = statement
            .query_map([project_id], |row| {
                Ok(HookRun {
                    id: row.get(0)?,
                    hook_id: row.get(1)?,
                    conversation_id: row.get(2)?,
                    turn_id: row.get(3)?,
                    event: row.get(4)?,
                    status: row.get(5)?,
                    detail: row.get(6)?,
                    created_at: row.get(7)?,
                    completed_file_revision: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

pub struct HookContext<'a> {
    pub db: &'a Database,
    pub tools: &'a ToolRegistry,
    pub workspace: Option<&'a Workspace>,
    pub source_scope: &'a [String],
    pub conversation_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub cancel: &'a CancellationToken,
    pub plan_mode: bool,
    pub isolated: bool,
    pub approval_mode: ToolApprovalMode,
}

fn tracked_revision(db: &Database, conversation: &str, turn: &str) -> Result<u64, CoreError> {
    Ok(db.conn().query_row("SELECT COALESCE(MAX(id),0) FROM turn_file_change_events WHERE conversation_id=?1 AND turn_id=?2", params![conversation,turn], |row| row.get(0))?)
}

/// Returns observations, not instructions. Failure blocks completion but never changes a committed tool's success.
pub async fn run_event(
    context: &HookContext<'_>,
    event: HookEvent,
) -> Result<Vec<HookRun>, CoreError> {
    if context.plan_mode {
        return Ok(vec![]);
    }
    let (Some(conversation_id), Some(turn_id)) = (context.conversation_id, context.turn_id) else {
        return Ok(vec![]);
    };
    let Some(project_id) = context.db.get_conversation(conversation_id)?.project_id else {
        return Ok(vec![]);
    };
    let hooks = context
        .db
        .project_hooks(&project_id)?
        .into_iter()
        .filter(|hook| hook.enabled && hook.event == event)
        .collect::<Vec<_>>();
    if hooks.is_empty() {
        return Ok(vec![]);
    }
    let changes = context
        .db
        .conversation_file_changes_for_turns(conversation_id, Some(&[turn_id.to_string()]))?
        .into_iter()
        .next()
        .unwrap_or_default();
    // Formatter output stays in the file-change history without recursively triggering itself.
    let trigger_revision: u64 = context.db.conn().query_row("SELECT COALESCE(MAX(id),0) FROM turn_file_change_events WHERE conversation_id=?1 AND turn_id=?2 AND mutation_id NOT LIKE 'hook:%'", params![conversation_id, turn_id], |row| row.get(0))?;
    if event == HookEvent::AfterFileChange && trigger_revision == 0 {
        return Ok(vec![]);
    }
    if changes.pending {
        return Err(CoreError::Agent(
            "Lifecycle checks are waiting for pending file changes to settle.".into(),
        ));
    }
    let workspace = context.workspace.or(context.tools.workspace());
    let mut runs = Vec::new();
    for hook in hooks {
        let revision = blake3::hash(&serde_json::to_vec(
            &json!({"hook":hook,"files":trigger_revision,"workspace":workspace}),
        )?)
        .to_hex()
        .to_string();
        let run_id = uuid::Uuid::new_v4().to_string();
        let inserted = context.db.conn().execute("INSERT OR IGNORE INTO project_hook_runs(id,project_id,hook_id,conversation_id,turn_id,event,revision,status) VALUES(?1,?2,?3,?4,?5,?6,?7,'running')", params![run_id, project_id, hook.id, conversation_id, turn_id, event.name(), revision])?;
        if inserted == 0 {
            let stored: (String, String, String, String, Option<u64>) = context.db.conn().query_row("SELECT id,status,detail,created_at,completed_file_revision FROM project_hook_runs WHERE hook_id=?1 AND conversation_id=?2 AND turn_id=?3 AND event=?4 AND revision=?5", params![hook.id, conversation_id, turn_id, event.name(), revision], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?;
            runs.push(HookRun {
                id: stored.0,
                hook_id: hook.id,
                conversation_id: conversation_id.into(),
                turn_id: turn_id.into(),
                event: event.name().into(),
                status: stored.1,
                detail: stored.2,
                created_at: stored.3,
                completed_file_revision: stored.4,
            });
            continue;
        }
        let mut guard = RunningHook {
            db: context.db,
            id: &run_id,
            settled: false,
        };
        let args = json!({"action":"run","name":hook.name,"manifestHash":hook.manifest_hash,"arguments":hook.arguments});
        let permission = crate::approval::permission_key_for_tool("project_tool", &args);
        let denied = context.approval_mode == ToolApprovalMode::DenyAll
            || context
                .db
                .resolve_tool_permission_policy(&permission)?
                .as_deref()
                == Some("never");
        let result = if context.isolated {
            Err(CoreError::InvalidInput("Project hooks cannot run in the controller sandbox. Disable this hook or use a normal workspace run; completion remains blocked.".into()))
        } else if denied || !context.tools.contains("project_tool") {
            Err(CoreError::InvalidInput(
                "Project hook execution is denied by the current tool permissions.".into(),
            ))
        } else if let Some(workspace) = workspace {
            let validation = crate::tools::project_tool::validate_project_hook(
                context.db,
                workspace,
                &hook.name,
                &hook.manifest_hash,
                &hook.arguments,
            );
            match validation {
                Err(error) => Err(error),
                Ok(()) => {
                    let mut tool_context =
                        ToolExecutionContext::new(&run_id, "{}", context.db, context.source_scope)
                            .with_conversation_id(Some(conversation_id))
                            .with_turn_id(Some(turn_id));
                    tool_context.workspace = Some(workspace);
                    tool_context.file_change_owner =
                        Some(crate::turn_file_changes::FileChangeOwner {
                            conversation_id: conversation_id.into(),
                            turn_id: turn_id.into(),
                            mutation_namespace: Some("hook".into()),
                        });
                    tool_context.cancel_token = Some(context.cancel);
                    let current_changes = context
                        .db
                        .conversation_file_changes_for_turns(
                            conversation_id,
                            Some(&[turn_id.to_string()]),
                        )?
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    let event_data = json!({"event":event.name(),"conversationId":conversation_id,"turnId":turn_id,"fileRevision":current_changes.revision,"files":current_changes.files});
                    tokio::select! {
                        biased;
                        _ = context.cancel.cancelled() => Err(CoreError::Agent("Project hook cancelled".into())),
                        result = crate::tools::project_tool::execute_project_hook(&tool_context, &hook.name, &hook.manifest_hash, hook.arguments.clone(), event_data) => result,
                    }
                }
            }
        } else {
            Err(CoreError::InvalidInput(
                "Project hook requires a selected workspace.".into(),
            ))
        };
        let (status, detail) = match result {
            Ok(result) => (
                if result.is_error { "failed" } else { "passed" },
                result.llm_context_content(),
            ),
            Err(error) => (
                if context.cancel.is_cancelled() {
                    "cancelled"
                } else {
                    "failed"
                },
                error.to_string(),
            ),
        };
        let detail = format!(
            "{}: {}",
            hook.name,
            detail.chars().take(32_000).collect::<String>()
        );
        let completed_file_revision = tracked_revision(context.db, conversation_id, turn_id)?;
        context.db.conn().execute(
            "UPDATE project_hook_runs SET status=?2,detail=?3,completed_file_revision=?4 WHERE id=?1",
            params![run_id, status, detail, completed_file_revision],
        )?;
        guard.settled = true;
        runs.push(HookRun {
            id: run_id.clone(),
            hook_id: hook.id,
            conversation_id: conversation_id.into(),
            turn_id: turn_id.into(),
            event: event.name().into(),
            status: status.into(),
            detail,
            created_at: String::new(),
            completed_file_revision: Some(completed_file_revision),
        });
        if context.cancel.is_cancelled() {
            break;
        }
    }
    Ok(runs)
}

/// Also catches native external-agent file writes before their final answer is persisted.
pub async fn completion_blocker(context: &HookContext<'_>) -> Result<Option<String>, CoreError> {
    Ok(completion_feedback(context).await?.blocker)
}

pub(crate) struct CompletionFeedback {
    pub(crate) blocker: Option<String>,
    pub(crate) completed_hooks: Vec<String>,
}

pub(crate) async fn completion_feedback(
    context: &HookContext<'_>,
) -> Result<CompletionFeedback, CoreError> {
    let mut runs = run_event(context, HookEvent::AfterFileChange).await?;
    runs.extend(run_event(context, HookEvent::BeforeComplete).await?);
    if let (Some(conversation), Some(turn)) = (context.conversation_id, context.turn_id) {
        let changes = context
            .db
            .conversation_file_changes_for_turns(conversation, Some(&[turn.to_string()]))?
            .into_iter()
            .next()
            .unwrap_or_default();
        if !runs.is_empty() && changes.pending {
            return Ok(CompletionFeedback {
                blocker: Some(
                    "Project lifecycle checks are waiting for pending file changes to settle."
                        .into(),
                ),
                completed_hooks: Vec::new(),
            });
        }
        let final_revision = tracked_revision(context.db, conversation, turn)?;
        for run in &mut runs {
            if run.status == "passed" && run.completed_file_revision != Some(final_revision) {
                run.status = "failed".into();
                run.detail = format!("Completion check is stale because a later hook changed files. Inspect the final changes and repair or reconfigure the checks; successful commands were not replayed. Previous result: {}", run.detail);
                context.db.conn().execute("UPDATE project_hook_runs SET status='failed',detail=?2 WHERE id=?1 AND status='passed'", params![run.id,run.detail])?;
            }
        }
    }
    let failed = runs
        .iter()
        .filter(|run| run.status != "passed")
        .map(|run| format!("{}: {}", run.status, run.detail))
        .collect::<Vec<_>>();
    Ok(CompletionFeedback {
        blocker: (!failed.is_empty()).then(|| format!("Project lifecycle checks prevent completion. Repair the files and try again; checks rerun when the tracked file revision changes. Do not replay successful file writes.\n{}", failed.join("\n"))),
        completed_hooks: runs.iter().filter(|run| run.status == "passed").map(|run| run.hook_id.clone()).collect(),
    })
}

struct RunningHook<'a> {
    db: &'a Database,
    id: &'a str,
    settled: bool,
}
impl Drop for RunningHook<'_> {
    fn drop(&mut self) {
        if !self.settled {
            let _ = self.db.conn().execute("UPDATE project_hook_runs SET status='cancelled',detail='Execution was interrupted; no automatic replay is permitted.' WHERE id=?1 AND status='running'", [self.id]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FinalOnlyProvider;
    #[async_trait::async_trait]
    impl crate::llm::LlmProvider for FinalOnlyProvider {
        fn name(&self) -> &str {
            "hook-finalization-test"
        }
        async fn list_models(&self) -> Result<Vec<String>, CoreError> {
            Ok(vec!["test".into()])
        }
        async fn complete(
            &self,
            _: &crate::llm::CompletionRequest,
        ) -> Result<crate::llm::CompletionResponse, CoreError> {
            Err(CoreError::Llm("Streaming fixture only".into()))
        }
        async fn stream_events(
            &self,
            _: &crate::llm::CompletionRequest,
        ) -> Result<futures::stream::BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError>
        {
            crate::llm::provider_events_from_chunk_stream(Box::pin(futures::stream::iter(vec![
                Ok(crate::llm::StreamChunk {
                    delta: "The work is complete.".into(),
                    tool_call_delta: None,
                    finish_reason: Some(crate::llm::FinishReason::Stop),
                    usage: None,
                    thinking_delta: None,
                }),
            ])))
        }
        async fn health_check(&self) -> Result<(), CoreError> {
            Ok(())
        }
    }

    struct Fixture {
        directory: tempfile::TempDir,
        db: Database,
        tools: ToolRegistry,
        workspace: Workspace,
        cancel: CancellationToken,
    }
    impl Fixture {
        fn new(script: &str) -> Self {
            let directory = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(directory.path().join(".nexa/tools")).unwrap();
            #[cfg(windows)]
            let command = json!({"program":"cmd.exe","args":["/d","/c",script],"timeoutSecs":3});
            #[cfg(not(windows))]
            let command = json!({"program":"sh","args":["-c",script],"timeoutSecs":3});
            std::fs::write(directory.path().join(".nexa/tools/check.json"), serde_json::to_vec(&json!({"name":"check","description":"Check the fixture","command":command,"access":{"read":true,"write":true,"execute":true}})).unwrap()).unwrap();
            let db = Database::open_memory().unwrap();
            db.execute_batch_for_test("INSERT INTO projects(id,name) VALUES('project','test'); INSERT INTO conversations(id,provider,model,project_id) VALUES('chat','open_ai','test','project'); INSERT INTO messages(id,conversation_id,role,content) VALUES('user','chat','user','test'); INSERT INTO conversation_turns(id,conversation_id,user_message_id) VALUES('turn','chat','user');").unwrap();
            let workspace =
                Workspace::validate(&[directory.path().to_string_lossy().into()]).unwrap();
            db.conn()
                .execute(
                    "UPDATE projects SET workspace_roots_json=?1 WHERE id='project'",
                    [serde_json::to_string(&workspace.roots).unwrap()],
                )
                .unwrap();
            let mut tools = ToolRegistry::new();
            tools.register(Box::new(crate::tools::project_tool::ProjectTool));
            Self {
                directory,
                db,
                tools,
                workspace,
                cancel: CancellationToken::new(),
            }
        }
        fn context(&self) -> HookContext<'_> {
            HookContext {
                db: &self.db,
                tools: &self.tools,
                workspace: Some(&self.workspace),
                source_scope: &[],
                conversation_id: Some("chat"),
                turn_id: Some("turn"),
                cancel: &self.cancel,
                plan_mode: false,
                isolated: false,
                approval_mode: ToolApprovalMode::Ask,
            }
        }
        fn enable(&self, event: HookEvent) -> ProjectHook {
            let catalog = crate::tools::project_tool::workspace_project_tool_catalog(
                &self.db,
                &self.workspace,
            )
            .unwrap();
            self.db
                .save_project_hook(
                    "project",
                    ProjectHook {
                        id: String::new(),
                        name: "check".into(),
                        manifest_hash: catalog.tools[0].manifest_hash.clone(),
                        event,
                        arguments: json!({}),
                        enabled: true,
                    },
                )
                .unwrap()
        }
        fn mutation(&self, id: &str) {
            self.db.conn().execute("INSERT INTO turn_file_change_events(conversation_id,turn_id,mutation_id) VALUES('chat','turn',?1)", [id]).unwrap();
        }
    }

    #[tokio::test]
    async fn hooks_require_opt_in_deduplicate_and_track_their_own_writes_without_recursion() {
        let fixture = Fixture::new("echo check >> count.txt");
        assert!(completion_blocker(&fixture.context())
            .await
            .unwrap()
            .is_none());
        assert!(!fixture.directory.path().join("count.txt").exists());
        fixture.enable(HookEvent::BeforeComplete);
        let blocker = completion_blocker(&fixture.context()).await.unwrap();
        assert!(blocker.is_none(), "{blocker:?}");
        assert!(completion_blocker(&fixture.context())
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            std::fs::read_to_string(fixture.directory.path().join("count.txt"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert!(fixture.db.conversation_file_changes("chat").unwrap()[0]
            .files
            .iter()
            .any(|file| file.path.ends_with("count.txt")));
        fixture.mutation("agent-edit");
        assert!(completion_blocker(&fixture.context())
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            std::fs::read_to_string(fixture.directory.path().join("count.txt"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        assert_eq!(fixture.db.project_hook_runs("project").unwrap().len(), 2);
    }

    #[tokio::test]
    async fn failed_hooks_block_completion_and_manifest_changes_never_run_without_reapproval() {
        let fixture = Fixture::new("exit 7");
        fixture.enable(HookEvent::BeforeComplete);
        assert!(completion_blocker(&fixture.context())
            .await
            .unwrap()
            .unwrap()
            .contains("failed"));
        let manifest = fixture.directory.path().join(".nexa/tools/check.json");
        let mut value: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        *value["command"]["args"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap() = json!("echo forbidden > should-not-exist.txt");
        std::fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        fixture.mutation("repair");
        assert!(completion_blocker(&fixture.context())
            .await
            .unwrap()
            .unwrap()
            .contains("changed"));
        assert!(!fixture
            .directory
            .path()
            .join("should-not-exist.txt")
            .exists());
    }

    #[tokio::test]
    async fn a_later_hook_cannot_invalidate_an_earlier_completion_check() {
        let validator = if cfg!(windows) {
            "findstr /x valid generated.txt >nul"
        } else {
            "grep -qx valid generated.txt"
        };
        let fixture = Fixture::new(validator);
        // Windows FINDSTR /x requires CRLF for its whole-line comparison.
        std::fs::write(
            fixture.directory.path().join("generated.txt"),
            if cfg!(windows) {
                "valid\r\n"
            } else {
                "valid\n"
            },
        )
        .unwrap();
        let first = fixture.enable(HookEvent::BeforeComplete);
        let mut manifest: Value = serde_json::from_slice(
            &std::fs::read(fixture.directory.path().join(".nexa/tools/check.json")).unwrap(),
        )
        .unwrap();
        manifest["name"] = json!("mutate");
        *manifest["command"]["args"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap() = json!(if cfg!(windows) {
            "echo invalid>generated.txt & echo mutation>>mutations.txt"
        } else {
            "printf 'invalid\n' > generated.txt; printf 'mutation\n' >> mutations.txt"
        });
        std::fs::write(
            fixture.directory.path().join(".nexa/tools/mutate.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let catalog = crate::tools::project_tool::workspace_project_tool_catalog(
            &fixture.db,
            &fixture.workspace,
        )
        .unwrap();
        let tool = catalog
            .tools
            .iter()
            .find(|tool| tool.name == "mutate")
            .unwrap();
        let second = fixture
            .db
            .save_project_hook(
                "project",
                ProjectHook {
                    id: String::new(),
                    name: "mutate".into(),
                    manifest_hash: tool.manifest_hash.clone(),
                    event: HookEvent::BeforeComplete,
                    arguments: json!({}),
                    enabled: true,
                },
            )
            .unwrap();
        fixture
            .db
            .conn()
            .execute(
                "UPDATE project_hooks SET created_at='2000-01-01 00:00:00' WHERE id=?1",
                [&first.id],
            )
            .unwrap();
        fixture
            .db
            .conn()
            .execute(
                "UPDATE project_hooks SET created_at='2000-01-01 00:00:01' WHERE id=?1",
                [&second.id],
            )
            .unwrap();
        let blocker = completion_blocker(&fixture.context())
            .await
            .unwrap()
            .expect("later hook writes must invalidate earlier checks");
        assert!(blocker.contains("stale"), "{blocker}");
        assert!(completion_blocker(&fixture.context())
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            std::fs::read_to_string(fixture.directory.path().join("mutations.txt"))
                .unwrap()
                .lines()
                .count(),
            1,
            "a stale check must not replay the later mutating command"
        );
        assert!(fixture
            .db
            .project_hook_runs("project")
            .unwrap()
            .iter()
            .any(|run| run.hook_id == first.id && run.status == "failed"));
    }

    #[tokio::test]
    async fn plan_and_permissions_remain_authoritative_for_hooks() {
        let fixture = Fixture::new("echo forbidden > should-not-exist.txt");
        fixture.enable(HookEvent::AfterFileChange);
        assert!(run_event(&fixture.context(), HookEvent::AfterFileChange)
            .await
            .unwrap()
            .is_empty());
        fixture.mutation("edit");
        let mut context = fixture.context();
        context.plan_mode = true;
        assert!(completion_blocker(&context).await.unwrap().is_none());
        assert!(fixture.db.project_hook_runs("project").unwrap().is_empty());
        context.plan_mode = false;
        context.approval_mode = ToolApprovalMode::DenyAll;
        assert!(completion_blocker(&context)
            .await
            .unwrap()
            .unwrap()
            .contains("denied"));
        assert!(!fixture
            .directory
            .path()
            .join("should-not-exist.txt")
            .exists());
    }

    #[tokio::test]
    async fn hook_cancellation_is_recorded_and_not_replayed() {
        let fixture = Fixture::new("echo forbidden > should-not-exist.txt");
        fixture.enable(HookEvent::BeforeComplete);
        fixture.cancel.cancel();
        let blocker = completion_blocker(&fixture.context())
            .await
            .unwrap()
            .unwrap();
        assert!(blocker.contains("cancelled"));
        assert!(!fixture
            .directory
            .path()
            .join("should-not-exist.txt")
            .exists());
        assert_eq!(
            fixture.db.project_hook_runs("project").unwrap()[0].status,
            "cancelled"
        );
    }

    #[tokio::test]
    async fn failed_hooks_never_publish_a_done_event_even_after_the_repair_limit() {
        let fixture = Fixture::new("exit 7");
        fixture.enable(HookEvent::BeforeComplete);
        let tools = fixture
            .tools
            .clone()
            .with_workspace(Some(fixture.workspace.clone()));
        let executor = crate::agent::AgentExecutor::new(
            Box::new(FinalOnlyProvider),
            tools,
            crate::agent::AgentConfig {
                model: Some("test".into()),
                max_iterations: 2,
                context_window: Some(16_000),
                ..Default::default()
            },
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let result = executor
            .run(
                Vec::new(),
                vec![crate::llm::ContentPart::Text {
                    text: "Finish the task.".into(),
                }],
                &fixture.db,
                Some("chat"),
                Some("turn"),
                tx,
                1,
            )
            .await;
        assert!(
            result.is_err(),
            "A failing completion check cannot become a successful partial answer"
        );
        let runs = fixture.db.project_hook_runs("project").unwrap();
        assert_eq!(
            runs.len(),
            1,
            "The real agent must reach the check and deduplicate repair attempts"
        );
        assert_eq!(runs[0].status, "failed");
        while let Ok(event) = rx.try_recv() {
            assert!(
                !matches!(event, crate::agent::AgentEvent::Done { .. }),
                "A Done event would publish a completed run"
            );
        }
        assert!(fixture
            .db
            .get_messages("chat")
            .unwrap()
            .iter()
            .all(|message| message.role != crate::llm::Role::Assistant));
    }

    #[tokio::test]
    async fn productive_file_repairs_continue_beyond_the_profile_retry_count() {
        use crate::llm::{ContentPart, FinishReason, StreamChunk, ToolCallDelta};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct RepairProvider {
            calls: Arc<AtomicUsize>,
            directory: std::path::PathBuf,
        }
        #[async_trait::async_trait]
        impl crate::llm::LlmProvider for RepairProvider {
            fn name(&self) -> &str {
                "progressive-hook-repair"
            }
            async fn list_models(&self) -> Result<Vec<String>, CoreError> {
                Ok(vec!["test".into()])
            }
            async fn health_check(&self) -> Result<(), CoreError> {
                Ok(())
            }
            async fn complete(
                &self,
                _: &crate::llm::CompletionRequest,
            ) -> Result<crate::llm::CompletionResponse, CoreError> {
                unreachable!()
            }
            async fn stream_events(
                &self,
                request: &crate::llm::CompletionRequest,
            ) -> Result<futures::stream::BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError>
            {
                let index = self.calls.fetch_add(1, Ordering::SeqCst);
                assert!(
                    request
                        .messages
                        .iter()
                        .filter(|message| message.role == crate::llm::Role::System)
                        .all(|message| !message.text_content().contains("HOOK_UNTRUSTED_MARKER")),
                    "repository-controlled check output must never enter controller authority"
                );
                if index == 2 {
                    assert!(
                        request
                            .messages
                            .iter()
                            .any(|message| message.role == crate::llm::Role::User
                                && message.text_content().contains("HOOK_UNTRUSTED_MARKER")),
                        "the repair still needs lower-authority check evidence"
                    );
                }
                assert!(
                    index < 8,
                    "the real check must pass after the fourth mutation"
                );
                let write = index % 2 == 0;
                let path = self.directory.join(if index == 6 {
                    "ready.txt".into()
                } else {
                    format!("candidate-{index}.txt")
                });
                let chunk = StreamChunk {
                    delta: if write {
                        String::new()
                    } else {
                        "The requested work is complete.".into()
                    },
                    tool_call_delta: write.then(|| ToolCallDelta {
                        id: format!("repair-{index}"),
                        name: Some("create_file".into()),
                        arguments_delta:
                            json!({"path":path,"content":format!("candidate {index}")})
                                .to_string()
                                .into(),
                        index: Some(0),
                        thought_signature: None,
                    }),
                    finish_reason: Some(if write {
                        FinishReason::ToolCalls
                    } else {
                        FinishReason::Stop
                    }),
                    usage: None,
                    thinking_delta: None,
                };
                crate::llm::provider_events_from_chunk_stream(Box::pin(futures::stream::iter([
                    Ok(chunk),
                ])))
            }
        }
        #[cfg(windows)]
        let script = "if exist ready.txt (exit /b 0) else (echo HOOK_UNTRUSTED_MARKER & exit /b 7)";
        #[cfg(not(windows))]
        let script =
            "if test -f ready.txt; then exit 0; else echo HOOK_UNTRUSTED_MARKER; exit 7; fi";
        let fixture = Fixture::new(script);
        fixture.enable(HookEvent::BeforeComplete);
        let mut tools = fixture
            .tools
            .clone()
            .with_workspace(Some(fixture.workspace.clone()));
        tools.register(Box::new(crate::tools::create_file_tool::CreateFileTool));
        let calls = Arc::new(AtomicUsize::new(0));
        let executor = crate::agent::AgentExecutor::new(
            Box::new(RepairProvider {
                calls: calls.clone(),
                directory: fixture.directory.path().into(),
            }),
            tools,
            crate::agent::AgentConfig {
                model: Some("test".into()),
                context_window: Some(32_000),
                tool_approval_mode: ToolApprovalMode::AllowAll,
                ..Default::default()
            },
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let result = executor
            .run(
                Vec::new(),
                vec![ContentPart::Text {
                    text: "Complete the file changes and pass project checks.".into(),
                }],
                &fixture.db,
                Some("chat"),
                Some("turn"),
                tx,
                1,
            )
            .await
            .unwrap();
        assert_eq!(result.text_content(), "The requested work is complete.");
        assert_eq!(calls.load(Ordering::SeqCst), 8);
        let runs = fixture.db.project_hook_runs("project").unwrap();
        assert_eq!(runs.iter().filter(|run| run.status == "failed").count(), 3);
        assert_eq!(runs.iter().filter(|run| run.status == "passed").count(), 1);
        assert_eq!(
            fixture.db.get_conversation_turn("turn").unwrap().status,
            "success"
        );
        drain.await.unwrap();
    }
}
