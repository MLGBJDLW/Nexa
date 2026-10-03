use super::*;
use std::path::PathBuf;

async fn fixture() -> (tempfile::TempDir, Database, String, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    git_text(&root, &["init"]).await.unwrap();
    git_text(&root, &["config", "core.autocrlf", "false"])
        .await
        .unwrap();
    std::fs::write(root.join("代码.txt"), "one\ntwo\nthree\n").unwrap();
    git_text(&root, &["add", "."]).await.unwrap();
    git_text(&root, &["commit", "-m", "base"]).await.unwrap();
    let db = Database::new(dir.path().join("nexa.db")).unwrap();
    let project = db
        .create_project(
            &serde_json::from_value(json!({"name":"review","workspaceRoots":[root]})).unwrap(),
        )
        .unwrap();
    let conversation = db
        .create_conversation(
            &serde_json::from_value(
                json!({"provider":"open_ai","model":"test","projectId":project.id}),
            )
            .unwrap(),
        )
        .unwrap();
    (dir, db, conversation.id, root)
}
fn finding(review: &Review) -> FindingInput {
    FindingInput {
        anchor: Anchor {
            revision: review.snapshot.revision.clone(),
            path: "代码.txt".into(),
            side: "new".into(),
            line: 2,
        },
        priority: 1,
        title: "Wrong result".into(),
        body: "The new value violates the expected result.".into(),
    }
}
#[tokio::test]
async fn stages_unicode_untracked_and_stale_dispositions_are_real_git_bound() {
    let (_dir, db, conversation, root) = fixture().await;
    std::fs::write(root.join("代码.txt"), "one\nstaged\nthree\n").unwrap();
    git_text(&root, &["add", "."]).await.unwrap();
    std::fs::write(root.join("代码.txt"), "one\nworking\nthree\n").unwrap();
    std::fs::write(root.join("新文件.txt"), "new\n").unwrap();
    let staged = start(&db, &conversation, "staged", "HEAD").await.unwrap();
    assert_eq!(staged.snapshot.files.len(), 1);
    assert!(staged.snapshot.files[0].patch.contains("+staged"));
    let unstaged = start(&db, &conversation, "unstaged", "HEAD").await.unwrap();
    assert_eq!(unstaged.snapshot.files.len(), 2);
    assert!(unstaged
        .snapshot
        .files
        .iter()
        .any(|file| file.patch.contains("-staged") && file.patch.contains("+working")));
    let mut review = start(&db, &conversation, "worktree", "HEAD").await.unwrap();
    assert!(review
        .snapshot
        .files
        .iter()
        .any(|file| file.path == "新文件.txt" && file.untracked));
    let input = finding(&review);
    review = add(&db, &conversation, &review.id, input.clone())
        .await
        .unwrap();
    review = add(&db, &conversation, &review.id, input.clone())
        .await
        .unwrap();
    assert_eq!(review.findings.len(), 1);
    let selected = vec![review.findings[0].id.clone()];
    let packet = feedback(
        &db,
        &conversation,
        &review.id,
        &review.snapshot.revision,
        &selected,
    )
    .await
    .unwrap();
    assert_eq!(
        packet,
        feedback(
            &db,
            &conversation,
            &review.id,
            &review.snapshot.revision,
            &selected
        )
        .await
        .unwrap()
    );
    let old_revision = review.snapshot.revision.clone();
    std::fs::write(root.join("代码.txt"), "inserted\none\nworking\nthree\n").unwrap();
    assert!(add(&db, &conversation, &review.id, input).await.is_err());
    assert!(
        feedback(&db, &conversation, &review.id, &old_revision, &selected)
            .await
            .is_err()
    );
    review = get(&db, &conversation).await.unwrap().unwrap();
    assert_eq!(review.view(false)["findings"][0]["stale"], true);
    assert!(disposition(
        &db,
        &conversation,
        &review.id,
        &review.snapshot.revision,
        &selected[0],
        "accepted"
    )
    .await
    .is_err());
    review = disposition(
        &db,
        &conversation,
        &review.id,
        &review.snapshot.revision,
        &selected[0],
        "resolved",
    )
    .await
    .unwrap();
    assert_eq!(review.view(false)["findings"][0]["stale"], false);
    assert_eq!(review.findings[0].content.anchor.revision, old_revision);
    let reopened = Database::new(db.db_path().unwrap()).unwrap();
    assert_eq!(
        get(&reopened, &conversation)
            .await
            .unwrap()
            .unwrap()
            .findings[0]
            .status,
        "resolved"
    );
    assert_eq!(list(&reopened, &conversation).unwrap().len(), 3);
    assert_eq!(
        select(&db, &conversation, &staged.id).await.unwrap().mode,
        "staged"
    );
}
#[tokio::test]
async fn commits_invalidate_reviews_and_branch_baseline_is_pinned() {
    let (_dir, db, conversation, root) = fixture().await;
    let base = git_text(&root, &["rev-parse", "HEAD"]).await.unwrap();
    std::fs::write(root.join("代码.txt"), "one\nchanged\nthree\n").unwrap();
    git_text(&root, &["add", "."]).await.unwrap();
    git_text(&root, &["commit", "-m", "feature"]).await.unwrap();
    let review = start(&db, &conversation, "branch", &base).await.unwrap();
    assert_eq!(review.base_sha, base);
    assert_eq!(review.snapshot.files.len(), 1);
    let review = add(&db, &conversation, &review.id, finding(&review))
        .await
        .unwrap();
    git_text(&root, &["commit", "--allow-empty", "-m", "new head"])
        .await
        .unwrap();
    let updated = get(&db, &conversation).await.unwrap().unwrap();
    assert_ne!(updated.snapshot.revision, review.snapshot.revision);
    assert_eq!(updated.base_sha, base);
    assert_eq!(updated.view(false)["findings"][0]["stale"], true);
}
#[tokio::test]
async fn subdirectory_scope_and_anchor_validation_refuse_unrelated_content() {
    let (_dir, db, conversation, root) = fixture().await;
    std::fs::create_dir(root.join("child")).unwrap();
    std::fs::write(root.join("child/inside.txt"), "inside\n").unwrap();
    git_text(&root, &["add", "."]).await.unwrap();
    git_text(&root, &["commit", "-m", "child"]).await.unwrap();
    let project = db
        .create_project(
            &serde_json::from_value(json!({"name":"child","workspaceRoots":[root.join("child")]}))
                .unwrap(),
        )
        .unwrap();
    db.conn()
        .execute(
            "UPDATE conversations SET project_id=?1 WHERE id=?2",
            rusqlite::params![project.id, conversation],
        )
        .unwrap();
    std::fs::write(root.join("代码.txt"), "outside\n").unwrap();
    std::fs::write(root.join("child/inside.txt"), "new inside\n").unwrap();
    let review = start(&db, &conversation, "worktree", "HEAD").await.unwrap();
    assert_eq!(review.snapshot.files.len(), 1);
    assert_eq!(review.snapshot.files[0].path, "child/inside.txt");
    let mut input = finding(&review);
    assert!(add(&db, &conversation, &review.id, input.clone())
        .await
        .is_err());
    input.anchor.path = "child/inside.txt".into();
    input.anchor.line = 999;
    assert!(add(&db, &conversation, &review.id, input.clone())
        .await
        .is_err());
    input.anchor.line = 1;
    assert!(add(&db, &conversation, &review.id, input).await.is_ok());
}
#[test]
fn malformed_review_tools_fail_before_dispatch() {
    use crate::tools::ToolRegistry;
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(crate::tools::code_review_tool::CodeReviewTool));
    for args in [
        r#"{"action":"snapshot","path":"secret"}"#,
        r#"{"action":"file","path":"test"}"#,
        r#"{"action":"add_finding","review_id":"x"}"#,
    ] {
        assert!(registry
            .prepare_arguments_for_scheduling("code_review", "bad", args)
            .1
            .is_some());
    }
}

#[tokio::test]
async fn binary_bytes_change_revision_and_incomplete_snapshots_refuse_feedback() {
    let (_dir, db, conversation, root) = fixture().await;
    std::fs::write(root.join("binary.dat"), [0, 1, 2]).unwrap();
    let first = start(&db, &conversation, "worktree", "HEAD").await.unwrap();
    std::fs::write(root.join("binary.dat"), [0, 1, 3]).unwrap();
    let second = get(&db, &conversation).await.unwrap().unwrap();
    assert_ne!(first.snapshot.revision, second.snapshot.revision);
    assert!(second.snapshot.files[0].binary);
    std::fs::write(root.join("oversized.txt"), vec![b'x'; 256 * 1024 + 1]).unwrap();
    let incomplete = get(&db, &conversation).await.unwrap().unwrap();
    assert!(incomplete.snapshot.truncated);
    assert!(feedback(
        &db,
        &conversation,
        &incomplete.id,
        &incomplete.snapshot.revision,
        &[]
    )
    .await
    .is_err());
}

#[tokio::test]
async fn registered_review_tool_reads_and_records_findings_in_the_owned_workspace() {
    use crate::tools::{ToolExecutionContext, ToolRegistry};
    let (_dir, db, conversation, root) = fixture().await;
    std::fs::write(root.join("代码.txt"), "one\nchanged\nthree\n").unwrap();
    let review = start(&db, &conversation, "worktree", "HEAD").await.unwrap();
    let mut registry =
        ToolRegistry::new().with_workspace(db.conversation_workspace(&conversation).unwrap());
    registry.register(Box::new(crate::tools::code_review_tool::CodeReviewTool));
    let result = registry
        .execute(
            "code_review",
            ToolExecutionContext::new("snapshot", r#"{"action":"snapshot"}"#, &db, &[])
                .with_conversation_id(Some(&conversation)),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert!(value["snapshot"]["files"][0].get("patch").is_none());
    let args = json!({"action":"add_finding","review_id":review.id,"finding":finding(&review)})
        .to_string();
    let result = registry
        .execute(
            "code_review",
            ToolExecutionContext::new("add", &args, &db, &[])
                .with_conversation_id(Some(&conversation)),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        get(&db, &conversation)
            .await
            .unwrap()
            .unwrap()
            .findings
            .len(),
        1
    );
}
