use std::fs;
use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    fn visit(path: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).expect("read source directory") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    visit(root, &mut files);
    files
}

#[test]
fn desktop_host_cannot_escape_through_raw_database_connections() {
    let desktop = repository_root().join("apps/desktop/src-tauri/src");
    let offenders = rust_sources(&desktop)
        .into_iter()
        .filter(|path| {
            fs::read_to_string(path)
                .expect("read Rust source")
                .contains(".conn()")
        })
        .collect::<Vec<_>>();

    assert!(
        offenders.is_empty(),
        "desktop code must use storage APIs instead of Database::conn(): {offenders:?}"
    );
}

#[test]
fn desktop_host_cannot_construct_the_builtin_tool_registry_directly() {
    let desktop = repository_root().join("apps/desktop/src-tauri/src");
    let offenders = rust_sources(&desktop)
        .into_iter()
        .filter(|path| {
            fs::read_to_string(path)
                .expect("read Rust source")
                .contains("default_tool_registry()")
        })
        .collect::<Vec<_>>();

    assert!(
        offenders.is_empty(),
        "desktop code must obtain tools from PackageRuntimeAssembler: {offenders:?}"
    );
}

#[test]
fn manual_context_compaction_has_one_tool_free_service_path() {
    let root = repository_root();
    let command =
        fs::read_to_string(root.join("apps/desktop/src-tauri/src/commands/conversation.rs"))
            .expect("read conversation commands");
    let service = fs::read_to_string(root.join("crates/core/src/context_maintenance/service.rs"))
        .expect("read context maintenance service");

    for required in [
        "start_context_compaction_cmd",
        "observe_context_compaction_cmd",
        "cancel_context_compaction_cmd",
        ".context_compaction",
        ".db_executor",
    ] {
        assert!(
            command.contains(required),
            "desktop compaction protocol must include {required}"
        );
    }
    for forbidden in [
        "AgentExecutor::new",
        "ToolRegistry::new",
        "replace_messages_if_unchanged",
        "create_checkpoint_with_messages",
    ] {
        assert!(
            !service.contains(forbidden),
            "context maintenance must not depend on {forbidden}"
        );
    }
}

#[test]
fn readme_frontend_versions_match_the_manifest() {
    let root = repository_root();
    let package: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("apps/desktop/package.json")).expect("read package.json"),
    )
    .expect("parse package.json");
    let readme = fs::read_to_string(root.join("README.md")).expect("read README");

    for (dependency, label) in [("react", "React"), ("react-router", "React Router")] {
        let version = package["dependencies"][dependency]
            .as_str()
            .expect("frontend dependency version")
            .trim_start_matches(|character: char| !character.is_ascii_digit());
        let documented = version.split('.').take(2).collect::<Vec<_>>().join(".");
        assert!(
            readme.contains(&format!("{label} {documented}")),
            "README must document {label} {documented}"
        );
    }
}
