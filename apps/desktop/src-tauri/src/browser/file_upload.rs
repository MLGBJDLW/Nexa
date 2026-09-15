use std::collections::HashSet;
use std::path::Path;

use nexa_core::browser_runtime::BrowserFileMetadata;
use nexa_core::tools::ToolExecutionContext;

pub(super) struct PreparedUpload {
    pub paths: Vec<String>,
    pub files: Vec<BrowserFileMetadata>,
}

pub(super) fn prepare_upload(
    context: &ToolExecutionContext<'_>,
    requested: &[String],
) -> Result<PreparedUpload, String> {
    if requested.len() > 20 {
        return Err("upload_files accepts at most 20 files".into());
    }
    let mut paths = Vec::new();
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut total = 0_u64;
    for path in requested {
        let path = nexa_core::tools::resolve_agent_file_path(
            context.db,
            context.source_scope,
            Path::new(path),
        )
        .map_err(|error| error.to_string())?;
        if !seen.insert(path.clone()) {
            return Err("upload_files contains a duplicate file".into());
        }
        let metadata = path
            .metadata()
            .map_err(|error| format!("Could not read upload metadata: {error}"))?;
        if !metadata.is_file() {
            return Err("upload_files only accepts regular files".into());
        }
        total = total
            .checked_add(metadata.len())
            .ok_or("Upload byte count overflow")?;
        if total > 100 * 1024 * 1024 {
            return Err("upload_files exceeds the 100 MiB total size limit".into());
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Upload filename is not Unicode")?
            .to_string();
        paths.push(
            path.to_str()
                .ok_or("Upload path is not Unicode")?
                .to_string(),
        );
        files.push(BrowserFileMetadata {
            name,
            size: metadata.len(),
        });
    }
    Ok(PreparedUpload { paths, files })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upload_paths_use_the_registered_file_policy_and_validate_metadata() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("proof.txt");
        std::fs::write(&path, "proof").unwrap();
        let db = nexa_core::db::Database::open_memory().unwrap();
        let source = db
            .add_source(nexa_core::sources::CreateSourceInput {
                root_path: root.path().to_string_lossy().into(),
                include_globs: vec![],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();
        let scope = vec![source.id];
        let context = ToolExecutionContext::new("upload", "{}", &db, &scope);
        let upload = prepare_upload(&context, &["proof.txt".into()]).unwrap();
        assert_eq!(
            upload.files,
            vec![BrowserFileMetadata {
                name: "proof.txt".into(),
                size: 5
            }]
        );
        assert!(prepare_upload(&context, &["proof.txt".into(), "proof.txt".into()]).is_err());
        assert!(prepare_upload(&context, &["../outside.txt".into()]).is_err());
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("private.txt");
        std::fs::write(&outside_file, "private fixture").unwrap();
        assert!(prepare_upload(&context, &[outside_file.to_string_lossy().into()]).is_err());
        let oversized = root.path().join("large.bin");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(100 * 1024 * 1024 + 1)
            .unwrap();
        assert!(prepare_upload(&context, &["large.bin".into()]).is_err());
        assert!(prepare_upload(&context, &[]).unwrap().files.is_empty());
    }
}
