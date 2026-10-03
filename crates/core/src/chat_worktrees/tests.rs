use super::*;
use serde_json::json;

async fn fixture() -> (tempfile::TempDir, Database, String, String, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source-日本語");
    std::fs::create_dir(&source).unwrap();
    git_text(&source, &["init"]).await.unwrap();
    git_text(&source, &["config", "core.autocrlf", "false"])
        .await
        .unwrap();
    std::fs::write(source.join("same.txt"), "original\n").unwrap();
    git_text(&source, &["add", "."]).await.unwrap();
    git_text(&source, &["commit", "-m", "initial"])
        .await
        .unwrap();
    let db = Database::new(dir.path().join("nexa.db")).unwrap();
    let project = db
        .create_project(
            &serde_json::from_value(json!({"name":"parallel","workspaceRoots":[source]})).unwrap(),
        )
        .unwrap();
    let first = db
        .create_conversation(
            &serde_json::from_value(
                json!({"provider":"open_ai","model":"test","projectId":project.id}),
            )
            .unwrap(),
        )
        .unwrap();
    let second = db
        .create_conversation(
            &serde_json::from_value(
                json!({"provider":"open_ai","model":"test","projectId":project.id}),
            )
            .unwrap(),
        )
        .unwrap();
    (dir, db, first.id, second.id, source)
}

#[tokio::test]
async fn two_chats_survive_restart_and_archive_preserves_working_and_staged_versions() {
    let (_dir, db, first, second, source) = fixture().await;
    // The source is deliberately dirty: creation starts at the selected commit.
    std::fs::write(source.join("same.txt"), "source-only").unwrap();
    let a = create(&db, &first, "HEAD").await.unwrap();
    let b = create(&db, &second, "HEAD").await.unwrap();
    assert_ne!(a.path, b.path);
    let a_root = Path::new(&a.path);
    let b_root = Path::new(&b.path);
    std::fs::write(a_root.join("same.txt"), "staged first").unwrap();
    git_text(a_root, &["add", "same.txt"]).await.unwrap();
    std::fs::write(a_root.join("same.txt"), "working first").unwrap();
    std::fs::write(a_root.join("新文件.txt"), "untracked first").unwrap();
    std::fs::write(b_root.join("same.txt"), "second").unwrap();
    let reopened = Database::new(db.db_path().unwrap()).unwrap();
    assert_eq!(
        reopened
            .conversation_workspace(&first)
            .unwrap()
            .unwrap()
            .cwd(),
        Some(a.path.as_str())
    );
    assert_eq!(
        crate::git_workspace::conversation_sources(&db, &second).unwrap()[0].1,
        b_root
    );
    let lease = activity(&db, &first).unwrap();
    assert!(archive(&db, &first)
        .await
        .unwrap_err()
        .to_string()
        .contains("close its terminals"));
    drop(lease);
    let archived = archive(&db, &first).await.unwrap();
    assert!(!a_root.exists());
    assert!(db.conversation_workspace(&first).is_err());
    assert_eq!(
        git_text(
            Path::new(&archived.common_dir),
            &[
                "show",
                &format!("{}:same.txt", archived.index_snapshot_sha.unwrap())
            ]
        )
        .await
        .unwrap(),
        "staged first"
    );
    let restored = restore(&reopened, &first).await.unwrap();
    assert_eq!(restored.status, "ready");
    assert_eq!(
        std::fs::read_to_string(a_root.join("same.txt")).unwrap(),
        "working first"
    );
    assert_eq!(
        std::fs::read_to_string(a_root.join("新文件.txt")).unwrap(),
        "untracked first"
    );
    assert_eq!(
        std::fs::read_to_string(b_root.join("same.txt")).unwrap(),
        "second"
    );
    assert_eq!(
        std::fs::read_to_string(source.join("same.txt")).unwrap(),
        "source-only"
    );
    assert!(db.remove_conversation_from_project(&first).is_err());
}

#[tokio::test]
async fn archive_refuses_ignored_embedded_and_tampered_paths_without_removing_files() {
    let (_dir, db, first, _, source) = fixture().await;
    let mut record = create(&db, &first, "HEAD").await.unwrap();
    let root = PathBuf::from(&record.path);
    std::fs::write(root.join(".gitignore"), "private.txt\n").unwrap();
    std::fs::write(root.join("private.txt"), "preserve").unwrap();
    assert!(archive(&db, &first)
        .await
        .unwrap_err()
        .to_string()
        .contains("ignored"));
    assert!(root.join("private.txt").exists());
    std::fs::remove_file(root.join("private.txt")).unwrap();
    std::fs::create_dir_all(root.join("nested/.git")).unwrap();
    assert!(archive(&db, &first)
        .await
        .unwrap_err()
        .to_string()
        .contains("embedded"));
    record.path = display(&source);
    db.save_chat_worktree(&record).unwrap();
    assert!(archive(&db, &first).await.is_err());
    assert!(source.join("same.txt").exists());
}

#[tokio::test]
async fn interrupted_operations_reconcile_without_implicit_deletion() {
    let (_dir, db, first, _, _) = fixture().await;
    let mut record = create(&db, &first, "HEAD").await.unwrap();
    record.status = "creating".into();
    db.save_chat_worktree(&record).unwrap();
    assert!(db.conversation_workspace(&first).is_err());
    assert_eq!(recover(&db, &first).await.unwrap().unwrap().status, "ready");
    let archived = archive(&db, &first).await.unwrap();
    let mut interrupted = archived.clone();
    interrupted.status = "archiving".into();
    db.save_chat_worktree(&interrupted).unwrap();
    assert_eq!(
        recover(&db, &first).await.unwrap().unwrap().status,
        "archived"
    );
    restore(&db, &first).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(Path::new(&archived.path).join("same.txt")).unwrap(),
        "original\n"
    );
}

#[tokio::test]
async fn failed_identity_is_durable_and_workspace_resolution_checks_git_pointers() {
    let (_directory, db, first, second, _) = fixture().await;
    let a = create(&db, &first, "HEAD").await.unwrap();
    let b = create(&db, &second, "HEAD").await.unwrap();
    let git_file = Path::new(&a.path).join(".git");
    let original = std::fs::read(&git_file).unwrap();
    let foreign = std::fs::read(Path::new(&b.path).join(".git")).unwrap();
    std::fs::write(&git_file, &foreign).unwrap();
    assert_eq!(
        inspect(&db, &first).await.unwrap().unwrap().status,
        "needs_recovery"
    );
    let reopened = Database::new(db.db_path().unwrap()).unwrap();
    assert_eq!(
        reopened.chat_worktree(&first).unwrap().unwrap().status,
        "needs_recovery"
    );
    assert!(reopened.conversation_workspace(&first).is_err());
    std::fs::write(&git_file, &original).unwrap();
    assert!(
        reopened.conversation_workspace(&first).is_err(),
        "repairing a pointer still requires explicit recovery"
    );
    recover(&reopened, &first).await.unwrap();
    assert!(reopened.conversation_workspace(&first).is_ok());
    std::fs::write(&git_file, foreign).unwrap();
    assert!(
        reopened.conversation_workspace(&first).is_err(),
        "launch must validate ownership without opening the panel first"
    );
    assert!(Path::new(&a.path).join("same.txt").exists());
    assert!(Path::new(&b.path).join("same.txt").exists());
}
