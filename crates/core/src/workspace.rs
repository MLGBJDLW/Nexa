//! Project-owned directories, independent of indexed knowledge sources.
use crate::{db::Database, error::CoreError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    /// Order is authoritative: relative paths and commands start in roots[0].
    pub roots: Vec<String>,
}

impl Workspace {
    pub fn validate(roots: &[String]) -> Result<Self, CoreError> {
        if roots.len() > 16 {
            return Err(CoreError::InvalidInput(
                "A workspace supports at most 16 root folders".into(),
            ));
        }
        let mut normalized = Vec::new();
        for root in roots {
            let path = Path::new(root.trim());
            if !path.is_absolute() || !path.is_dir() {
                return Err(CoreError::InvalidInput(format!(
                    "Workspace folder is unavailable: {}",
                    path.display()
                )));
            }
            let path = std::fs::canonicalize(path).map_err(|error| {
                CoreError::InvalidInput(format!("Cannot open workspace folder: {error}"))
            })?;
            let path = path.to_string_lossy().to_string();
            #[cfg(windows)]
            let path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
                format!(r"\\{unc}")
            } else {
                path.strip_prefix(r"\\?\").unwrap_or(&path).to_string()
            };
            if !normalized.contains(&path) {
                normalized.push(path);
            }
        }
        Ok(Self { roots: normalized })
    }
    pub fn cwd(&self) -> Option<&str> {
        self.roots.first().map(String::as_str)
    }
    pub fn resolve_relative(&self, path: &Path) -> Result<PathBuf, CoreError> {
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        self.cwd()
            .map(|cwd| Path::new(cwd).join(path))
            .ok_or_else(|| {
                CoreError::InvalidInput(
                    "Choose a workspace root folder before using relative paths".into(),
                )
            })
    }
    pub fn prompt(&self) -> String {
        if self.roots.is_empty() {
            return "This project has no workspace folders. Ask the user to choose a folder before workspace operations; do not infer a directory from unrelated knowledge sources.".into();
        }
        format!("## Project workspace\nDefault working directory: {}\nRoot folders: {}\nResolve relative paths from the default working directory. Keep project outputs in this workspace. Knowledge sources are reference material, not alternate working directories.", self.roots[0], self.roots.join(", "))
    }
}

impl Database {
    pub fn conversation_workspace(
        &self,
        conversation_id: &str,
    ) -> Result<Option<Workspace>, CoreError> {
        let conversation = self.get_conversation(conversation_id)?;
        let Some(project_id) = conversation.project_id else {
            return Ok(None);
        };
        let project = self.get_project(&project_id)?;
        Ok(project.workspace_roots.map(|roots| Workspace { roots }))
    }
}
