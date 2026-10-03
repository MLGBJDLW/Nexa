//! Directory-scoped, user-owned instructions. Discovery never leaves an
//! explicitly selected workspace and never treats indexed knowledge as rules.

use crate::{error::CoreError, workspace::Workspace};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_RULE_BYTES: usize = 32 * 1024;
const MAX_RULE_FILES: usize = 64;
const MAX_SCOPE_DEPTH: usize = 128;
const MAX_TRACKED_TURNS: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRule {
    pub path: String,
    pub scope: String,
    pub revision: String,
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRules {
    pub revision: String,
    pub files: Vec<WorkspaceRule>,
    pub diagnostics: Vec<String>,
}

impl WorkspaceRules {
    pub fn prompt(&self) -> String {
        if self.files.is_empty() && self.diagnostics.is_empty() {
            return String::new();
        }
        let mut text = String::from("## Workspace file instructions\nThese are user-owned rules from explicitly selected workspace folders. Each applies only to its scope and descendants; more specific directories override their ancestors. Explicit user and Project instructions take precedence. Read applicable child rules before modifying files. When a tool reports new rules, review them and call workspace_rules with action=acknowledge, the target path, and the reported revision before retrying the operation.\n");
        for rule in &self.files {
            text.push_str(&format!(
                "\n### {}\nScope: {}\nRevision: {}\n{}\n",
                rule.path, rule.scope, rule.revision, rule.content
            ));
            if rule.truncated {
                text.push_str("[Rule exceeds the instruction budget. Read the full file before editing; acknowledgement is unavailable until it fits.]\n");
            }
        }
        for diagnostic in &self.diagnostics {
            text.push_str(&format!("\nRule discovery: {diagnostic}\n"));
        }
        text
    }

    fn acknowledgeable(&self) -> bool {
        self.diagnostics.is_empty() && self.files.iter().all(|file| !file.truncated)
    }
}

fn canonical_target(path: &Path) -> Result<PathBuf, CoreError> {
    crate::file_mutation::canonical_file_identity(path)
}

/// Load root rules and only the ancestor chains of the requested paths. Child
/// trees are not scanned. The file budget is shared across every selected root.
pub fn load(workspace: &Workspace, targets: &[PathBuf]) -> WorkspaceRules {
    let mut result = WorkspaceRules::default();
    let mut directories = Vec::new();
    let mut seen = BTreeSet::new();
    for root in &workspace.roots {
        let root = match std::fs::canonicalize(root) {
            Ok(root) if root.is_dir() => root,
            _ => {
                result
                    .diagnostics
                    .push(format!("Workspace folder unavailable: {root}"));
                continue;
            }
        };
        let mut scoped = vec![root.clone()];
        for target in targets {
            let path = if target.is_absolute() {
                target.clone()
            } else {
                Path::new(workspace.cwd().unwrap_or_default()).join(target)
            };
            let Ok(target) = canonical_target(&path) else {
                continue;
            };
            if !target.starts_with(&root) {
                continue;
            }
            let directory = if target.is_dir() {
                target.as_path()
            } else {
                target.parent().unwrap_or(&root)
            };
            let mut chain = Vec::new();
            let mut current = directory;
            while current != root {
                if chain.len() >= MAX_SCOPE_DEPTH {
                    result
                        .diagnostics
                        .push(format!("Rule scope is too deep: {}", target.display()));
                    break;
                }
                chain.push(current.to_path_buf());
                let Some(parent) = current.parent() else {
                    break;
                };
                current = parent;
            }
            chain.reverse();
            scoped.extend(chain);
        }
        for directory in scoped {
            if seen.insert(directory.clone()) {
                directories.push((root.clone(), directory));
            }
        }
    }
    let mut remaining = MAX_RULE_BYTES;
    for (root, directory) in directories {
        for name in ["AGENTS.override.md", "AGENTS.md"] {
            let path = directory.join(name);
            match std::fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    result
                        .diagnostics
                        .push(format!("Cannot inspect {}: {error}", path.display()));
                    break;
                }
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    result
                        .diagnostics
                        .push(format!("Rule must be a regular file: {}", path.display()));
                    break;
                }
                Ok(_) => {}
            }
            let canonical = match std::fs::canonicalize(&path) {
                Ok(path) if path.starts_with(&root) => path,
                _ => {
                    result
                        .diagnostics
                        .push(format!("Rule escapes its workspace: {}", path.display()));
                    break;
                }
            };
            let mut bytes = Vec::new();
            if let Err(error) = std::fs::File::open(&canonical).and_then(|file| {
                file.take((MAX_RULE_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
            }) {
                result
                    .diagnostics
                    .push(format!("Cannot read {}: {error}", path.display()));
                break;
            }
            if bytes.len() <= MAX_RULE_BYTES && bytes.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            if result.files.len() >= MAX_RULE_FILES {
                result
                    .diagnostics
                    .push("Too many applicable instruction files".into());
                break;
            }
            let revision = blake3::hash(&bytes).to_hex().to_string();
            let content = match std::str::from_utf8(&bytes) {
                Ok(text) => text,
                Err(error) if bytes.len() > MAX_RULE_BYTES && error.error_len().is_none() => {
                    std::str::from_utf8(&bytes[..error.valid_up_to()]).expect("valid UTF-8 prefix")
                }
                Err(_) => {
                    result
                        .diagnostics
                        .push(format!("Rule is not UTF-8: {}", path.display()));
                    break;
                }
            };
            let mut end = remaining.min(content.len());
            while !content.is_char_boundary(end) {
                end -= 1;
            }
            result.files.push(WorkspaceRule {
                path: canonical.to_string_lossy().into_owned(),
                scope: directory.to_string_lossy().into_owned(),
                revision,
                content: content[..end].to_string(),
                truncated: end < bytes.len(),
            });
            remaining -= end;
            break;
        }
    }
    result.revision = blake3::hash(
        &serde_json::to_vec(&(&result.files, &result.diagnostics)).unwrap_or_default(),
    )
    .to_hex()
    .to_string();
    result
}

/// Paths are discovered only from declared filesystem arguments, never from
/// free-form prompts, shell source, retrieved documents, or MCP results.
pub fn invocation_targets(arguments: &serde_json::Value) -> Vec<PathBuf> {
    let mut targets = Vec::new();
    for key in [
        "path",
        "file_path",
        "filePath",
        "cwd",
        "directory",
        "root",
        "source",
        "destination",
        "output_path",
        "outputPath",
    ] {
        if let Some(path) = arguments.get(key).and_then(serde_json::Value::as_str) {
            if !path.is_empty() {
                targets.push(PathBuf::from(path));
            }
        }
    }
    for key in ["paths", "files"] {
        if let Some(paths) = arguments.get(key).and_then(serde_json::Value::as_array) {
            for path in paths.iter().take(128) {
                if let Some(path) = path
                    .as_str()
                    .or_else(|| path.get("path").and_then(serde_json::Value::as_str))
                {
                    targets.push(PathBuf::from(path));
                }
            }
        }
    }
    targets
}

type ConversationTurn = (String, String);
type RuleFileRevision = (String, String);

#[derive(Default)]
pub struct WorkspaceRuleState {
    acknowledged: Mutex<BTreeMap<ConversationTurn, BTreeSet<RuleFileRevision>>>,
}

impl WorkspaceRuleState {
    pub fn acknowledge(
        &self,
        conversation: &str,
        turn: &str,
        rules: &WorkspaceRules,
        revision: &str,
    ) -> Result<(), CoreError> {
        if rules.revision != revision || !rules.acknowledgeable() {
            return Err(CoreError::InvalidInput("Workspace rules changed or could not be fully loaded. Read the current rules and resolve the displayed diagnostics before acknowledging.".into()));
        }
        let mut state = self
            .acknowledged
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.len() >= MAX_TRACKED_TURNS
            && !state.contains_key(&(conversation.into(), turn.into()))
        {
            state.pop_first();
        }
        let known = state.entry((conversation.into(), turn.into())).or_default();
        for rule in &rules.files {
            known.retain(|(path, _)| path != &rule.path);
            known.insert((rule.path.clone(), rule.revision.clone()));
        }
        Ok(())
    }

    pub fn needs_acknowledgement(
        &self,
        conversation: &str,
        turn: &str,
        rules: &WorkspaceRules,
    ) -> bool {
        if !rules.acknowledgeable() {
            return true;
        }
        let state = self
            .acknowledged
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let known = state.get(&(conversation.into(), turn.into()));
        rules.files.iter().any(|rule| {
            !known.is_some_and(|known| known.contains(&(rule.path.clone(), rule.revision.clone())))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_only_selected_ancestors_and_refreshes_revisions() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("前端/components")).unwrap();
        std::fs::create_dir(root.path().join("other")).unwrap();
        std::fs::write(root.path().join("AGENTS.md"), "root rules").unwrap();
        std::fs::write(root.path().join("前端/AGENTS.md"), "child rules").unwrap();
        std::fs::write(root.path().join("other/AGENTS.md"), "never load").unwrap();
        let workspace = Workspace::validate(&[root.path().to_string_lossy().into_owned()]).unwrap();
        let rules = load(&workspace, &[PathBuf::from("前端/components/new.ts")]);
        assert_eq!(rules.files.len(), 2);
        assert_eq!(rules.files[0].content, "root rules");
        assert_eq!(rules.files[1].content, "child rules");
        let state = WorkspaceRuleState::default();
        assert!(state.needs_acknowledgement("chat", "turn", &rules));
        state
            .acknowledge("chat", "turn", &rules, &rules.revision)
            .unwrap();
        assert!(!state.needs_acknowledgement("chat", "turn", &rules));
        assert!(state.needs_acknowledgement("chat", "next-turn", &rules));
        std::fs::write(root.path().join("前端/AGENTS.md"), "updated rules").unwrap();
        let changed = load(&workspace, &[PathBuf::from("前端/components/new.ts")]);
        assert!(state.needs_acknowledgement("chat", "turn", &changed));
        assert!(state
            .acknowledge("chat", "turn", &changed, &rules.revision)
            .is_err());
    }

    #[test]
    fn override_budget_and_unselected_parent_are_explicit() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(parent.path().join("AGENTS.md"), "outside").unwrap();
        std::fs::write(root.join("AGENTS.md"), "fallback").unwrap();
        std::fs::write(root.join("AGENTS.override.md"), "  ").unwrap();
        let workspace = Workspace::validate(&[root.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(load(&workspace, &[]).files[0].content, "fallback");
        std::fs::write(
            root.join("AGENTS.override.md"),
            "规则".repeat(MAX_RULE_BYTES),
        )
        .unwrap();
        let rules = load(&workspace, &[]);
        assert_eq!(rules.files.len(), 1);
        assert!(rules.files[0].truncated);
        assert!(rules.files[0].content.len() <= MAX_RULE_BYTES);
        assert!(WorkspaceRuleState::default()
            .acknowledge("c", "t", &rules, &rules.revision)
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_rules_and_targets_cannot_escape_the_selected_workspace() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("AGENTS.md"), "outside").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
        let workspace = Workspace::validate(&[root.path().to_string_lossy().into_owned()]).unwrap();
        assert!(load(&workspace, &[PathBuf::from("linked/file")])
            .files
            .is_empty());
        std::os::unix::fs::symlink(
            outside.path().join("AGENTS.md"),
            root.path().join("AGENTS.md"),
        )
        .unwrap();
        let rules = load(&workspace, &[]);
        assert!(rules.files.is_empty());
        assert!(!rules.diagnostics.is_empty());
    }
}
