use super::*;
use crate::{
    conversation::CreateConversationInput,
    project::{CreateProjectInput, UpdateProjectInput},
    tools::{ToolExecutionContext, ToolRegistry},
};
use serde_json::json;

#[tokio::test]
async fn primary_root_controls_files_commands_and_git_without_registered_sources() {
    let first = tempfile::Builder::new()
        .prefix("workspace-日本語-العربية-")
        .tempdir()
        .unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("same.txt"), "primary marker").unwrap();
    std::fs::write(second.path().join("same.txt"), "other marker").unwrap();
    let db = Database::open_memory().unwrap();
    let project = db
        .create_project(
            &serde_json::from_value::<CreateProjectInput>(json!({
                "name":"Workspace", "workspaceRoots":[first.path(), second.path(), first.path()]
            }))
            .unwrap(),
        )
        .unwrap();
    let conversation = db
        .create_conversation(
            &serde_json::from_value::<CreateConversationInput>(json!({
                "provider":"open_ai", "model":"test", "projectId":project.id
            }))
            .unwrap(),
        )
        .unwrap();
    let workspace = db
        .conversation_workspace(&conversation.id)
        .unwrap()
        .unwrap();
    assert_eq!(workspace.roots.len(), 2);
    assert!(db.list_sources().unwrap().is_empty());
    let mut tools = ToolRegistry::new().with_workspace(Some(workspace.clone()));
    tools.register(Box::new(crate::tools::file_tool::FileTool));
    tools.register(Box::new(crate::tools::create_file_tool::CreateFileTool));
    tools.register(Box::new(crate::tools::run_shell_tool::RunShellTool));
    let result = tools
        .execute(
            "read_file",
            ToolExecutionContext::new("read", r#"{"path":"same.txt"}"#, &db, &[]),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("primary marker"));
    let isolated = Workspace::validate(&[second.path().to_string_lossy().to_string()]).unwrap();
    let mut context =
        ToolExecutionContext::new("isolated-read", r#"{"path":"same.txt"}"#, &db, &[]);
    context.workspace = Some(&isolated);
    let result = tools.execute("read_file", context).await.unwrap();
    assert!(
        result.content.contains("other marker"),
        "an owned isolated workspace must override the parent registry"
    );
    let result = tools
        .execute(
            "create_file",
            ToolExecutionContext::new(
                "write",
                r#"{"path":"output.txt","content":"created in primary"}"#,
                &db,
                &[],
            ),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(first.path().join("output.txt").exists());
    assert!(!second.path().join("output.txt").exists());
    let roots = crate::git_workspace::conversation_sources(&db, &conversation.id).unwrap();
    assert_eq!(
        std::fs::canonicalize(&roots[0].1).unwrap(),
        std::fs::canonicalize(first.path()).unwrap()
    );
    let mut config = db.load_app_config().unwrap();
    config.shell_access_mode = crate::app_settings::ShellAccessMode::Open;
    db.save_app_config(&config).unwrap();
    let args = json!({"program": if cfg!(windows) {"python"} else {"python3"}, "args":["-c","import pathlib;print(pathlib.Path('same.txt').read_text())"]}).to_string();
    let result = tools
        .execute(
            "run_shell",
            ToolExecutionContext::new("shell", &args, &db, &[]),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("primary marker"));
    // A running turn and its filtered worker retain their captured workspace.
    db.update_project(
        &project.id,
        &serde_json::from_value::<UpdateProjectInput>(
            json!({"workspaceRoots":[second.path(),first.path()]}),
        )
        .unwrap(),
    )
    .unwrap();
    let filtered = tools.filtered(&["read_file".to_string()]);
    let result = filtered
        .execute(
            "read_file",
            ToolExecutionContext::new("read-again", r#"{"path":"same.txt"}"#, &db, &[]),
        )
        .await
        .unwrap();
    assert!(result.content.contains("primary marker"));
    let next = db
        .conversation_workspace(&conversation.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::canonicalize(next.cwd().unwrap()).unwrap(),
        std::fs::canonicalize(second.path()).unwrap()
    );
}

#[test]
fn workspace_update_is_atomic_and_empty_is_distinct_from_legacy_unconfigured() {
    let db = Database::open_memory().unwrap();
    let project = db
        .create_project(
            &serde_json::from_value::<CreateProjectInput>(json!({"name":"Original"})).unwrap(),
        )
        .unwrap();
    assert!(project.workspace_roots.is_none());
    let bad = serde_json::from_value::<UpdateProjectInput>(
        json!({"name":"Must not persist","workspaceRoots":["relative/not-a-directory"]}),
    )
    .unwrap();
    assert!(db.update_project(&project.id, &bad).is_err());
    assert_eq!(db.get_project(&project.id).unwrap().name, "Original");
    let empty = db
        .update_project(
            &project.id,
            &serde_json::from_value::<UpdateProjectInput>(json!({"workspaceRoots":[]})).unwrap(),
        )
        .unwrap();
    assert_eq!(empty.workspace_roots, Some(vec![]));
}
