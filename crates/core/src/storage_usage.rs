//! Read-only accounting for application-owned storage. Never follows links.
use crate::{
    db::{Database, DatabaseStorageStats},
    error::CoreError,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCategory {
    pub id: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub total_bytes: u64,
    pub categories: Vec<StorageCategory>,
    pub partial: bool,
    pub active_conversations: u64,
    pub archived_conversations: u64,
    pub database: DatabaseStorageStats,
}

fn table_category(table: &str) -> &'static str {
    if table.contains("conversation")
        || table.contains("message")
        || table.starts_with("agent_run")
        || table.starts_with("agent_task")
        || table.starts_with("agent_subtask")
        || table.starts_with("provider_turn")
        || table.starts_with("tool_")
        || table.starts_with("task_")
        || table.starts_with("context_")
    {
        "chats"
    } else if table.starts_with("fts_")
        || table.starts_with("document")
        || table.starts_with("chunk")
        || table.starts_with("embedding")
        || table.starts_with("knowledge")
        || table.starts_with("source")
        || table.starts_with("claim")
        || table.starts_with("graph_")
    {
        "knowledge"
    } else {
        "databaseOther"
    }
}

impl Database {
    pub fn storage_usage(&self) -> Result<StorageUsage, CoreError> {
        let database = self.storage_stats()?;
        let conn = self.conn();
        let (active, archived) = conn.query_row(
            "SELECT COUNT(*) FILTER (WHERE archived_at IS NULL), COUNT(*) FILTER (WHERE archived_at IS NOT NULL)
             FROM conversations WHERE parent_conversation_id IS NULL", [], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut sizes = BTreeMap::<String, u64>::new();
        // dbstat reads page metadata, not large message bodies or attachments.
        // A build without the virtual table still reports the measured database.
        if let Ok(mut statement) = conn.prepare("SELECT COALESCE(s.tbl_name, d.name), SUM(d.pgsize)
            FROM dbstat d LEFT JOIN sqlite_schema s ON s.name=d.name GROUP BY COALESCE(s.tbl_name,d.name)") {
            for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)))? {
                let (table, bytes) = row?;
                *sizes.entry(table_category(&table).into()).or_default() += bytes;
            }
        }
        let allocated = database
            .allocated_bytes
            .max(database.main_file_bytes.unwrap_or(0));
        let used: u64 = sizes.values().sum();
        if used == 0 {
            sizes.insert(
                "databaseOther".into(),
                allocated.saturating_sub(database.reusable_bytes),
            );
        } else {
            *sizes.entry("databaseOther".into()).or_default() += allocated
                .saturating_sub(used)
                .saturating_sub(database.reusable_bytes);
        }
        sizes.insert("reusable".into(), database.reusable_bytes);
        sizes.insert("wal".into(), database.wal_bytes.unwrap_or(0));
        let categories = sizes
            .into_iter()
            .map(|(id, bytes)| StorageCategory { id, bytes })
            .collect::<Vec<_>>();
        Ok(StorageUsage {
            total_bytes: categories.iter().map(|item| item.bytes).sum(),
            categories,
            partial: false,
            active_conversations: active,
            archived_conversations: archived,
            database,
        })
    }
}

fn is_link(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

/// Run on a blocking worker. A partial scan is explicit and never counts a
/// nested managed-model root twice or traverses a user's linked project.
pub fn add_files(
    usage: &mut StorageUsage,
    roots: &[(PathBuf, &str)],
    database_path: Option<&Path>,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let roots = roots
        .iter()
        .filter(|(path, _)| path.is_absolute())
        .collect::<Vec<_>>();
    let mut stack = roots
        .iter()
        .map(|(path, category)| (path.clone(), *category))
        .collect::<Vec<_>>();
    let mut visited = HashSet::new();
    let mut sizes = BTreeMap::<String, u64>::new();
    let db_files = database_path.map(|path| {
        [
            path.to_path_buf(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ]
    });
    let mut entries = 0usize;
    while let Some((path, inherited)) = stack.pop() {
        if Instant::now() >= deadline || entries >= 100_000 {
            usage.partial = true;
            break;
        }
        if !visited.insert(path.clone()) {
            continue;
        }
        entries += 1;
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                usage.partial = true;
                continue;
            }
        };
        if is_link(&metadata) {
            usage.partial = true;
            continue;
        }
        let category = roots
            .iter()
            .filter(|(root, _)| path.starts_with(root))
            .max_by_key(|(root, _)| root.components().count())
            .map(|(_, category)| *category)
            .unwrap_or(inherited);
        if metadata.is_dir() {
            match std::fs::read_dir(&path) {
                Ok(children) => {
                    for child in children {
                        if entries + stack.len() >= 100_000 {
                            usage.partial = true;
                            break;
                        }
                        match child {
                            Ok(child) => stack.push((child.path(), category)),
                            Err(_) => usage.partial = true,
                        }
                    }
                }
                Err(_) => usage.partial = true,
            }
        } else if metadata.is_file()
            && !db_files.as_ref().is_some_and(|files| files.contains(&path))
        {
            *sizes.entry(category.to_string()).or_default() += metadata.len();
        }
    }
    for (id, bytes) in sizes {
        usage.total_bytes = usage.total_bytes.saturating_add(bytes);
        if let Some(category) = usage
            .categories
            .iter_mut()
            .find(|category| category.id == id)
        {
            category.bytes += bytes;
        } else {
            usage.categories.push(StorageCategory { id, bytes });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_categories_partition_allocated_pages_and_nested_files_once() {
        let root = tempfile::tempdir().unwrap();
        let db = Database::new(root.path().join("nexa.db")).unwrap();
        let mut usage = db.storage_usage().unwrap();
        assert_eq!(
            usage.total_bytes,
            usage
                .database
                .allocated_bytes
                .max(usage.database.main_file_bytes.unwrap())
                + usage.database.wal_bytes.unwrap()
        );
        let previous = usage.total_bytes;
        let models = root.path().join("models");
        std::fs::create_dir(&models).unwrap();
        std::fs::write(models.join("model.bin"), [0u8; 256]).unwrap();
        std::fs::write(root.path().join("asset.bin"), [0u8; 128]).unwrap();
        add_files(
            &mut usage,
            &[(root.path().into(), "files"), (models, "models")],
            db.db_path(),
        );
        assert_eq!(usage.total_bytes, previous + 384);
        assert_eq!(
            usage
                .categories
                .iter()
                .find(|item| item.id == "models")
                .unwrap()
                .bytes,
            256
        );
        assert!(!usage.partial);
    }
}
