//! Publish inspected skill bundles through one database and filesystem owner.

use super::{Skill, SkillInstallSelection};
use crate::{db::Database, error::CoreError};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

struct PublishedDirectory {
    target: PathBuf,
    staged: PathBuf,
    original: Option<PathBuf>,
    backup: PathBuf,
    published: bool,
}

struct SkillPublication {
    temporary: Option<tempfile::TempDir>,
    directories: Vec<PublishedDirectory>,
    committed: bool,
}

impl Drop for SkillPublication {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut failed = false;
        for item in self.directories.iter().rev() {
            if item.published && fs::rename(&item.target, &item.staged).is_err() {
                failed = true;
                continue;
            }
            if let Some(original) = &item.original {
                if item.backup.exists() && fs::rename(&item.backup, original).is_err() {
                    failed = true;
                }
            }
        }
        if failed {
            if let Some(temporary) = self.temporary.take() {
                let recovery = temporary.keep();
                tracing::error!(path = %recovery.display(), "Skill install rollback could not restore every directory; recovery files were preserved");
            }
        }
    }
}

fn copy_existing_directory(source: &Path, destination: &Path) -> Result<(), CoreError> {
    fs::create_dir(destination)?;
    for entry in walkdir::WalkDir::new(source).min_depth(1) {
        let entry = entry.map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        let target = destination.join(relative);
        if entry.file_type().is_symlink() {
            return Err(CoreError::Conflict(format!(
                "Existing skill resource is a link and was preserved: {}",
                entry.path().display()
            )));
        } else if entry.file_type().is_dir() {
            fs::create_dir(&target)?;
        } else if entry.file_type().is_file() {
            fs::copy(entry.path(), &target)?;
        } else {
            return Err(CoreError::Conflict(format!(
                "Existing skill contains an unsupported filesystem entry: {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

impl SkillPublication {
    fn prepare(
        root: &Path,
        skills: &[Skill],
        previous: &HashMap<String, Skill>,
        sources: &[PathBuf],
    ) -> Result<Self, CoreError> {
        fs::create_dir_all(root)?;
        if fs::symlink_metadata(root)?.file_type().is_symlink() {
            return Err(CoreError::Conflict(
                "User skill directory is a link; installation left it unchanged".into(),
            ));
        }
        let root = fs::canonicalize(root)?;
        let temporary = tempfile::Builder::new()
            .prefix(".nexa-install-")
            .tempdir_in(&root)?;
        let stage_root = temporary.path().join("staged");
        let backup_root = temporary.path().join("backup");
        fs::create_dir(&stage_root)?;
        fs::create_dir(&backup_root)?;
        let source_paths = sources
            .iter()
            .map(fs::canonicalize)
            .collect::<Result<Vec<_>, _>>()?;
        let mut publication = Self {
            temporary: Some(temporary),
            directories: Vec::new(),
            committed: false,
        };
        // Prepare every complete directory before the first visible rename.
        for skill in skills {
            super::storage::validate_canonical_skill_name(&skill.canonical_name)?;
            let target = root.join(&skill.canonical_name);
            let legacy = root.join(&skill.id);
            if legacy != target
                && fs::symlink_metadata(&target).is_ok()
                && fs::symlink_metadata(&legacy).is_ok()
            {
                return Err(CoreError::Conflict(format!("Both canonical and legacy skill directories exist and were preserved: {} and {}. Resolve this conflict before importing.", target.display(), legacy.display())));
            }
            let original = if fs::symlink_metadata(&target).is_ok() {
                Some(target.clone())
            } else if previous.contains_key(&skill.id) && fs::symlink_metadata(&legacy).is_ok() {
                Some(legacy)
            } else {
                None
            };
            if let Some(original) = &original {
                let metadata = fs::symlink_metadata(original)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(CoreError::Conflict(format!(
                        "Existing skill path is not a real directory: {}",
                        original.display()
                    )));
                }
                if !previous.contains_key(&skill.id)
                    && !source_paths.iter().any(|source| {
                        (source.is_dir() && original.starts_with(source))
                            || (source.is_file()
                                && source.parent() == Some(original.as_path())
                                && source
                                    .extension()
                                    .and_then(|extension| extension.to_str())
                                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md")))
                    })
                {
                    return Err(CoreError::Conflict(format!("Unregistered skill directory already exists and was preserved: {}. Import that directory to register its current contents first.", original.display())));
                }
                copy_existing_directory(original, &stage_root.join(&skill.canonical_name))?;
            }
            if let Some(old) = previous.get(&skill.id) {
                super::storage::remove_obsolete_user_skill_resources_from_directory(
                    &stage_root,
                    old,
                    skill,
                )?;
            }
            super::storage::materialize_user_skill_to_directory(&stage_root, skill)?;
            publication.directories.push(PublishedDirectory {
                target,
                staged: stage_root.join(&skill.canonical_name),
                original,
                backup: backup_root.join(&skill.canonical_name),
                published: false,
            });
        }
        for item in &mut publication.directories {
            if let Some(original) = &item.original {
                fs::rename(original, &item.backup)?;
            }
            fs::rename(&item.staged, &item.target)?;
            item.published = true;
        }
        Ok(publication)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(root: &Path, name: &str, body: &str) -> PathBuf {
        let directory = root.join(name);
        fs::create_dir_all(directory.join("references")).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Test workflow\n---\n\n{body}\n"),
        )
        .unwrap();
        fs::write(
            directory.join("references/guide.md"),
            format!("Guide for {body}"),
        )
        .unwrap();
        directory
    }

    fn selected(sources: &[PathBuf]) -> Vec<SkillInstallSelection> {
        super::super::inspect_skill_install_sources(sources)
            .unwrap()
            .into_iter()
            .map(|skill| SkillInstallSelection {
                skill_file: skill.skill_file,
                content_digest: skill.content_digest,
            })
            .collect()
    }

    #[test]
    fn selected_sources_install_complete_bundles_and_survive_database_reopen() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("test.db");
        let db = Database::new(&database).unwrap();
        let sources = vec![
            source(temporary.path(), "alpha-demo", "Alpha body"),
            source(temporary.path(), "beta-demo", "Beta body"),
        ];
        let destination = temporary.path().join("installed");
        let selection = selected(&sources);
        assert_eq!(
            selection[0].content_digest,
            selected(&sources)[0].content_digest
        );
        let installed = install_skills_from_sources(
            &db,
            &sources,
            Some(&selection),
            false,
            false,
            &destination,
        )
        .unwrap();
        assert_eq!(installed.len(), 2);
        drop(db);
        let reopened = Database::new(&database).unwrap();
        let saved = reopened.list_skills().unwrap();
        assert_eq!(saved.len(), 2);
        for skill in saved {
            assert!(destination
                .join(&skill.canonical_name)
                .join("SKILL.md")
                .is_file());
            assert_eq!(
                fs::read_to_string(
                    destination
                        .join(&skill.canonical_name)
                        .join("references/guide.md")
                )
                .unwrap(),
                skill.resource_bundle[0].content
            );
        }
    }

    #[test]
    fn selection_skips_conflicts_and_detects_changed_resources_before_writes() {
        let temporary = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let sources = vec![
            source(temporary.path(), "alpha-demo", "Alpha"),
            source(temporary.path(), "beta-demo", "Beta"),
        ];
        let destination = temporary.path().join("installed");
        let mut selection = selected(&sources);
        install_skills_from_sources(
            &db,
            &sources,
            Some(&selection[..1]),
            false,
            false,
            &destination,
        )
        .unwrap();
        fs::write(
            sources[1].join("references/guide.md"),
            "Changed after preview",
        )
        .unwrap();
        assert!(install_skills_from_sources(
            &db,
            &sources,
            Some(&selection[1..]),
            false,
            false,
            &destination
        )
        .unwrap_err()
        .to_string()
        .contains("changed after preview"));
        assert_eq!(db.list_skills().unwrap().len(), 1);
        assert!(!destination.join("beta-demo").exists());
        selection = selected(&sources);
        install_skills_from_sources(
            &db,
            &sources,
            Some(&selection[1..]),
            false,
            false,
            &destination,
        )
        .unwrap();
        assert_eq!(db.list_skills().unwrap().len(), 2);
    }

    #[test]
    fn failed_publication_and_late_commit_leave_no_partially_installed_batch() {
        for fail_commit in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let db = Database::open_memory().unwrap();
            let sources = vec![
                source(temporary.path(), "alpha-demo", "Alpha"),
                source(temporary.path(), "beta-demo", "Beta"),
            ];
            let destination = temporary.path().join("installed");
            fs::create_dir(&destination).unwrap();
            if fail_commit {
                db.conn().execute_batch("CREATE TABLE skill_commit_guard (id TEXT REFERENCES skills(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER skill_commit_failure AFTER INSERT ON skills BEGIN INSERT INTO skill_commit_guard VALUES ('missing-record'); END;").unwrap();
            } else {
                fs::write(destination.join("beta-demo"), "User-owned file").unwrap();
            }
            assert!(install_skills_from_sources(
                &db,
                &sources,
                Some(&selected(&sources)),
                false,
                false,
                &destination
            )
            .is_err());
            assert!(db.list_skills().unwrap().is_empty());
            assert!(!destination.join("alpha-demo").exists());
            if fail_commit {
                assert!(!destination.join("beta-demo").exists());
            } else {
                assert_eq!(
                    fs::read_to_string(destination.join("beta-demo")).unwrap(),
                    "User-owned file"
                );
            }
            assert_eq!(
                fs::read_dir(&destination).unwrap().count(),
                usize::from(!fail_commit)
            );
        }
    }

    #[test]
    fn explicit_replacement_preserves_user_settings_and_unmodeled_files() {
        let temporary = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let sources = vec![source(temporary.path(), "alpha-demo", "Original")];
        let destination = temporary.path().join("installed");
        let old = install_skills_from_sources(&db, &sources, None, false, false, &destination)
            .unwrap()
            .remove(0);
        db.toggle_skill(&old.id, false).unwrap();
        fs::write(destination.join("alpha-demo/personal-notes.txt"), "Keep me").unwrap();
        fs::remove_file(sources[0].join("references/guide.md")).unwrap();
        fs::write(sources[0].join("references/new.md"), "New guide").unwrap();
        let changed = install_skills_from_sources(
            &db,
            &sources,
            Some(&selected(&sources)),
            true,
            false,
            &destination,
        )
        .unwrap()
        .remove(0);
        assert_eq!(old.id, changed.id);
        assert!(!changed.enabled);
        assert!(!destination.join("alpha-demo/references/guide.md").exists());
        assert_eq!(
            fs::read_to_string(destination.join("alpha-demo/references/new.md")).unwrap(),
            "New guide"
        );
        assert_eq!(
            fs::read_to_string(destination.join("alpha-demo/personal-notes.txt")).unwrap(),
            "Keep me"
        );
    }
}

/// Install exactly the inspected selection. Failed validation, preparation or
/// SQL writes publish no active skills; failed publication restores old files.
pub fn install_skills_from_sources(
    db: &Database,
    sources: &[PathBuf],
    selection: Option<&[SkillInstallSelection]>,
    replace_existing: bool,
    accept_blocked_warnings: bool,
    user_skills_dir: &Path,
) -> Result<Vec<Skill>, CoreError> {
    let inputs = super::importer::prepare_skill_install_inputs(
        db,
        sources,
        selection,
        replace_existing,
        accept_blocked_warnings,
    )?;
    let previous: HashMap<String, Skill> = db
        .list_skills()?
        .into_iter()
        .map(|skill| (skill.id.clone(), skill))
        .collect();
    let (mut skills, mut publication) =
        db.save_skills_atomically_with_guard(&inputs, |skills, current| {
            for actual in current {
                let expected = previous.get(&actual.id).ok_or_else(|| {
                    CoreError::Conflict(
                        "Installed skill changed during import; inspect again".into(),
                    )
                })?;
                if actual.name != expected.name
                    || actual.description != expected.description
                    || actual.enabled != expected.enabled
                    || actual.content != expected.content
                    || actual.resource_bundle != expected.resource_bundle
                    || actual.canonical_name != expected.canonical_name
                {
                    return Err(CoreError::Conflict(format!(
                        "Installed skill {} changed during import; inspect again",
                        actual.name
                    )));
                }
            }
            SkillPublication::prepare(user_skills_dir, skills, &previous, sources)
        })?;
    publication.committed = true;
    for skill in &mut skills {
        skill.source_path = Some(
            user_skills_dir
                .join(&skill.canonical_name)
                .join("SKILL.md")
                .to_string_lossy()
                .into_owned(),
        );
    }
    Ok(skills)
}
