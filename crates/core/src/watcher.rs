//! File watcher module — monitors source directories for changes.
//!
//! Uses the `notify` crate to watch source directories recursively and
//! emit debounced events when files are created, modified, or removed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{info, warn};

use crate::error::CoreError;

/// An event emitted when a watched file changes.
#[derive(Debug, Clone)]
pub struct WatcherEvent {
    pub path: PathBuf,
    pub kind: WatcherEventKind,
}

/// The kind of file change detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatcherEventKind {
    Created,
    Modified,
    Removed,
}

/// Route an event to every registered source containing it and normalize its
/// spelling before debounce. Overlapping sources own independent index rows.
pub fn source_paths_for_event<'a>(
    sources: impl IntoIterator<Item = (&'a str, &'a str)>,
    path: &Path,
) -> Vec<(String, PathBuf)> {
    let mut matches: Vec<_> = sources
        .into_iter()
        .filter_map(|(id, root)| {
            crate::ingest::relative_source_path(Path::new(root), path)
                .map(|relative| (id.to_owned(), Path::new(root).join(relative)))
        })
        .collect();
    matches.sort_by(|left, right| left.0.cmp(&right.0));
    matches
}

pub fn record_debounced_watcher_path(
    changed_paths: &mut HashSet<PathBuf>,
    removed_paths: &mut HashSet<PathBuf>,
    path: PathBuf,
    kind: WatcherEventKind,
) {
    if kind == WatcherEventKind::Removed {
        changed_paths.remove(&path);
        removed_paths.insert(path);
    } else {
        // Atomic saves commonly emit Removed followed by Created/Modified.
        // Keep the latest observed state for this source-relative identity.
        removed_paths.remove(&path);
        changed_paths.insert(path);
    }
}

/// Watches directories for file system changes.
pub struct FileWatcher {
    watcher: RecommendedWatcher,
}

impl FileWatcher {
    /// Create a new file watcher.
    ///
    /// Returns the watcher and a receiver that emits [`WatcherEvent`]s.
    /// Events are debounced with a 2-second delay to batch rapid changes.
    pub fn new() -> Result<(Self, mpsc::Receiver<WatcherEvent>), CoreError> {
        let (tx, rx) = mpsc::channel::<WatcherEvent>();

        let watcher = RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| match res {
                Ok(event) => {
                    let kind = match event.kind {
                        EventKind::Create(_) => Some(WatcherEventKind::Created),
                        EventKind::Modify(_) => Some(WatcherEventKind::Modified),
                        EventKind::Remove(_) => Some(WatcherEventKind::Removed),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        for path in event.paths {
                            let evt = WatcherEvent {
                                path: path.clone(),
                                kind: kind.clone(),
                            };
                            if tx.send(evt).is_err() {
                                warn!(
                                    "Watcher channel closed, dropping event for {}",
                                    path.display()
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("File watcher error: {e}");
                }
            },
            Config::default().with_poll_interval(Duration::from_secs(2)),
        )
        .map_err(|e| CoreError::Io(std::io::Error::other(e)))?;

        info!("File watcher initialized");
        Ok((Self { watcher }, rx))
    }

    /// Start watching a directory recursively.
    pub fn watch(&mut self, path: &Path) -> Result<(), CoreError> {
        self.watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| CoreError::Io(std::io::Error::other(e)))?;
        info!("Started watching: {}", path.display());
        Ok(())
    }

    /// Stop watching a directory.
    pub fn unwatch(&mut self, path: &Path) -> Result<(), CoreError> {
        self.watcher
            .unwatch(path)
            .map_err(|e| CoreError::Io(std::io::Error::other(e)))?;
        info!("Stopped watching: {}", path.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_alias_events_route_to_each_source_and_share_debounce_identity() {
        let folder = tempfile::tempdir().unwrap();
        let actual = folder.path().join("actual");
        std::fs::create_dir_all(actual.join("inner")).unwrap();
        let root = actual.join("inner").join("..");
        let unrelated = folder.path().join("outside");
        std::fs::create_dir(&unrelated).unwrap();
        let file = actual.join("event.md");
        std::fs::write(&file, "event").unwrap();
        let canonical = std::fs::canonicalize(&file).unwrap();
        let root = root.to_string_lossy().to_string();
        let parent = folder.path().to_string_lossy().to_string();
        let outside = unrelated.to_string_lossy().to_string();
        let sources = [
            ("child", root.as_str()),
            ("parent", parent.as_str()),
            ("outside", outside.as_str()),
        ];
        let created = source_paths_for_event(sources, &Path::new(&root).join("event.md"));
        assert_eq!(created.len(), 2);
        std::fs::remove_file(&file).unwrap();
        let removed = source_paths_for_event(sources, &canonical);
        assert_eq!(removed, created);
        for ((_, created), (_, removed)) in created.into_iter().zip(removed) {
            let mut changed = HashSet::new();
            let mut deleted = HashSet::new();
            record_debounced_watcher_path(
                &mut changed,
                &mut deleted,
                created.clone(),
                WatcherEventKind::Created,
            );
            record_debounced_watcher_path(
                &mut changed,
                &mut deleted,
                removed,
                WatcherEventKind::Removed,
            );
            assert!(changed.is_empty());
            assert_eq!(deleted.len(), 1);
            record_debounced_watcher_path(
                &mut changed,
                &mut deleted,
                created,
                WatcherEventKind::Modified,
            );
            assert!(deleted.is_empty());
            assert_eq!(changed.len(), 1);
        }
    }
}
