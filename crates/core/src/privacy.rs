//! Privacy module — exclude rules, content redaction, and config persistence.
//!
//! Provides [`PrivacyConfig`] for controlling which files to skip during
//! ingestion and how to redact sensitive content from chunks before storage.

use regex::Regex;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::db::Database;
use crate::error::CoreError;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Top-level privacy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyConfig {
    /// File-path glob patterns that are always excluded during ingestion.
    pub exclude_patterns: Vec<String>,
    /// Content redaction rules applied to every chunk before storage.
    pub redact_patterns: Vec<RedactRule>,
    /// Master switch — when `false`, redaction is skipped entirely.
    pub enabled: bool,
}

/// A single content-redaction rule backed by a regex.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactRule {
    /// Human-readable name shown in UI / logs.
    pub name: String,
    /// Regex pattern to match sensitive content.
    pub pattern: String,
    /// Replacement text, e.g. `"[REDACTED]"`.
    pub replacement: String,
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

/// File-path patterns that should *always* be excluded from ingestion unless
/// the user explicitly overrides the config.
pub fn default_exclude_patterns() -> Vec<String> {
    vec![
        "**/.git/**".into(),
        "**/.env".into(),
        "**/.env.*".into(),
        "**/node_modules/**".into(),
        "**/*.key".into(),
        "**/*.pem".into(),
        "**/*.p12".into(),
        "**/secrets.*".into(),
        "**/credentials.*".into(),
        "**/.ssh/**".into(),
    ]
}

/// Built-in redaction rules that are always active when redaction is enabled.
pub fn builtin_redact_rules() -> Vec<RedactRule> {
    vec![
        RedactRule {
            name: "email".into(),
            pattern: r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}".into(),
            replacement: "[EMAIL]".into(),
        },
        RedactRule {
            name: "ipv4".into(),
            pattern: r"\b(?:\d{1,3}\.){3}\d{1,3}\b".into(),
            replacement: "[IP]".into(),
        },
        RedactRule {
            name: "api_key".into(),
            // Matches `key=`, `token=`, `secret=` (or `: `) followed by a long
            // alphanumeric string (≥16 chars, allowing hyphens/underscores).
            pattern: r#"(?i)(?:key|token|secret)\s*[=:]\s*["']?([A-Za-z0-9\-_]{16,})["']?"#.into(),
            replacement: "[REDACTED]".into(),
        },
    ]
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            exclude_patterns: default_exclude_patterns(),
            redact_patterns: Vec::new(), // only builtins unless user adds custom
            enabled: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Redaction engine
// ---------------------------------------------------------------------------

/// Apply redaction rules to `content`, returning the sanitised string.
///
/// Both the built-in rules and any custom rules in `extra_rules` are applied.
/// Built-in rules run first, then custom rules in order.
pub fn redact_content(content: &str, extra_rules: &[RedactRule]) -> String {
    let mut result = content.to_string();

    // Apply built-in rules first.
    for rule in &builtin_redact_rules() {
        result = apply_redact_rule(&result, rule);
    }

    // Then user-supplied rules.
    for rule in extra_rules {
        result = apply_redact_rule(&result, rule);
    }

    result
}

/// Compile and apply a single [`RedactRule`] to `text`.
fn apply_redact_rule(text: &str, rule: &RedactRule) -> String {
    match Regex::new(&rule.pattern) {
        Ok(re) => re.replace_all(text, rule.replacement.as_str()).into_owned(),
        Err(e) => {
            debug!("Skipping invalid redaction rule '{}': {}", rule.name, e);
            text.to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// DB persistence
// ---------------------------------------------------------------------------

const PRIVACY_CONFIG_KEY: &str = "privacy_config";

pub(crate) fn config_fingerprint(config: &PrivacyConfig) -> Result<String, CoreError> {
    Ok(blake3::hash(&serde_json::to_vec(config)?)
        .to_hex()
        .to_string())
}

pub(crate) fn redaction_fingerprint(config: &PrivacyConfig) -> Result<String, CoreError> {
    Ok(blake3::hash(&serde_json::to_vec(&(
        1,
        config.enabled,
        builtin_redact_rules(),
        &config.redact_patterns,
    ))?)
    .to_hex()
    .to_string())
}

pub(crate) fn load_config_on(conn: &rusqlite::Connection) -> Result<PrivacyConfig, CoreError> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM privacy_config WHERE key=?1",
            [PRIVACY_CONFIG_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(stored
        .map(|value| serde_json::from_str(&value))
        .transpose()?
        .unwrap_or_default())
}

pub(crate) fn validate_config(config: &PrivacyConfig) -> Result<(), CoreError> {
    for rule in &config.redact_patterns {
        Regex::new(&rule.pattern).map_err(|error| {
            CoreError::InvalidInput(format!("Invalid redaction rule '{}': {error}", rule.name))
        })?;
    }
    for pattern in &config.exclude_patterns {
        globset::Glob::new(pattern).map_err(|error| {
            CoreError::InvalidInput(format!("Invalid privacy exclusion: {error}"))
        })?;
    }
    Ok(())
}

impl Database {
    /// Persist a [`PrivacyConfig`] to the database.
    pub fn save_privacy_config(&self, config: &PrivacyConfig) -> Result<(), CoreError> {
        validate_config(config)?;
        let json = serde_json::to_string(config)?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let old = load_config_on(&tx)?;
        let revoke =
            config.enabled && redaction_fingerprint(&old)? != redaction_fingerprint(config)?;
        tx.execute(
            "INSERT INTO privacy_config (key, value, updated_at)
             VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                                            updated_at = excluded.updated_at",
            params![PRIVACY_CONFIG_KEY, &json],
        )?;
        if revoke {
            // A privacy change revokes immutable references, including snapshots
            // for files already removed. Delete snapshots AFTER deletion triggers
            // run so old text cannot be archived back into the accessible corpus.
            tx.execute(
                "DELETE FROM entities WHERE id IN (SELECT entity_id FROM document_entities) AND NOT EXISTS(SELECT 1 FROM entity_links l WHERE l.evidence_doc_id IS NULL AND (l.source_entity_id=entities.id OR l.target_entity_id=entities.id))",
                [],
            )?;
            tx.execute("UPDATE documents SET index_revision=lower(hex(randomblob(16))),title='',metadata='{}'", [])?;
            tx.execute("DELETE FROM document_summaries", [])?;
            tx.execute("DELETE FROM chunks", [])?;
            tx.execute("DELETE FROM evidence_snapshots", [])?;
            tx.execute(
                "UPDATE knowledge_evidence SET locator_json='{}' WHERE document_id IS NOT NULL",
                [],
            )?;
        }
        tx.commit()?;
        drop(conn);
        if revoke {
            crate::vector_store::notify_sync();
        }
        Ok(())
    }

    /// Load the stored [`PrivacyConfig`], returning `PrivacyConfig::default()`
    /// if none has been saved yet.
    pub fn load_privacy_config(&self) -> Result<PrivacyConfig, CoreError> {
        let conn = self.conn();

        // Guard: table might not exist yet if migration hasn't run.
        let table_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='privacy_config')",
            [],
            |row| row.get(0),
        )?;
        if !table_exists {
            return Ok(PrivacyConfig::default());
        }

        let result = conn.query_row(
            "SELECT value FROM privacy_config WHERE key = ?1",
            params![PRIVACY_CONFIG_KEY],
            |row| row.get::<_, String>(0),
        );

        match result {
            Ok(json) => {
                let config: PrivacyConfig = serde_json::from_str(&json)?;
                Ok(config)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(PrivacyConfig::default()),
            Err(e) => Err(CoreError::Database(e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stricter_redaction_revokes_current_and_archived_evidence_before_rescan() {
        use crate::{
            ingest,
            models::{SearchFilters, SearchQuery},
            search,
            sources::CreateSourceInput,
        };
        let folder = tempfile::tempdir().unwrap();
        let current_path = folder.path().join("current.md");
        let removed_path = folder.path().join("removed.md");
        std::fs::write(
            &current_path,
            "# alice@example.com\nallowance privateCODE 500",
        )
        .unwrap();
        std::fs::write(&removed_path, "allowance privateCODE removed").unwrap();
        let db = Database::open_memory().unwrap();
        let mut config = PrivacyConfig {
            enabled: false,
            ..Default::default()
        };
        db.save_privacy_config(&config).unwrap();
        let source = db
            .add_source(CreateSourceInput {
                root_path: folder.path().to_string_lossy().into(),
                include_globs: vec!["**/*.md".into()],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();
        ingest::scan_source(&db, &source.id).unwrap();
        let query = SearchQuery {
            text: "allowance".into(),
            filters: SearchFilters::default(),
            limit: 20,
            offset: 0,
        };
        let old_cards = search::search(&db, &query).unwrap().evidence_cards;
        assert_eq!(old_cards.len(), 2);
        let current_document = old_cards
            .iter()
            .find(|card| card.document_path.ends_with("current.md"))
            .unwrap()
            .document_id
            .to_string();
        let manual_a = uuid::Uuid::new_v4().to_string();
        let manual_b = uuid::Uuid::new_v4().to_string();
        let generated = uuid::Uuid::new_v4().to_string();
        for entity in [&manual_a, &manual_b, &generated] {
            db.conn()
                .execute(
                    "INSERT INTO entities(id,name,entity_type) VALUES(?1,?1,'concept')",
                    [entity],
                )
                .unwrap();
        }
        db.conn().execute("INSERT INTO entity_links(id,source_entity_id,target_entity_id,relation_type,evidence_doc_id) VALUES(?1,?2,?3,'manual',NULL)", params![uuid::Uuid::new_v4().to_string(), manual_a, manual_b]).unwrap();
        let research = crate::research_workspace::create(
            &db,
            crate::research_workspace::CreateResearchSet {
                title: "Private comparison".into(),
                questions: vec!["allowance".into()],
                documents: vec![old_cards[0].evidence_ref.clone().unwrap()],
            },
        )
        .unwrap();
        let research =
            crate::research_workspace::refresh(&db, &research.summary.id, |_, _| {}).unwrap();
        crate::research_workspace::review(
            &db,
            crate::research_workspace::ReviewResearchCell {
                set_id: research.summary.id.clone(),
                document_id: research.documents[0].reference.document_id.to_string(),
                question_index: 0,
                expected_revision: research.summary.revision,
                review_state: "needs_review".into(),
                note: "Manually entered note".into(),
            },
        )
        .unwrap();
        std::fs::remove_file(&removed_path).unwrap();
        std::fs::write(
            &current_path,
            "# alice@example.com\nallowance privateCODE 600",
        )
        .unwrap();
        ingest::scan_source(&db, &source.id).unwrap();
        assert!(old_cards.iter().all(|card| search::get_evidence_card(
            &db,
            &card.chunk_id.to_string()
        )
        .is_ok()));
        // Model-generated membership belongs to the latest indexed revision;
        // the earlier file update correctly invalidated previous membership.
        for entity in [&manual_a, &generated] {
            db.conn()
                .execute(
                    "INSERT INTO document_entities(document_id,entity_id) VALUES(?1,?2)",
                    params![current_document, entity],
                )
                .unwrap();
        }
        config.enabled = true;
        config.redact_patterns.push(RedactRule {
            name: "code".into(),
            pattern: "privateCODE".into(),
            replacement: "[PRIVATE]".into(),
        });
        db.save_privacy_config(&config).unwrap();
        assert!(db.get_entity_by_id(&manual_a).is_ok());
        assert_eq!(db.get_entity_links(&manual_a).unwrap().len(), 1);
        assert!(db.get_entity_by_id(&generated).is_err());
        assert!(search::search(&db, &query)
            .unwrap()
            .evidence_cards
            .is_empty());
        let research = crate::research_workspace::get(&db, &research.summary.id).unwrap();
        assert!(research.documents[0].cells[0].stale);
        assert!(research.documents[0].cells[0].evidence.is_empty());
        assert_eq!(research.documents[0].cells[0].note, "Manually entered note");
        for card in old_cards {
            assert!(search::get_evidence_card(&db, &card.chunk_id.to_string()).is_err());
            assert!(
                search::resolve_evidence_ref(&db, card.evidence_ref.as_ref().unwrap()).is_err()
            );
        }
        assert_eq!(
            db.conn()
                .query_row("SELECT COUNT(*) FROM evidence_snapshots", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let result = ingest::scan_source(&db, &source.id).unwrap();
        assert_eq!(result.files_updated, 1);
        let cards = search::search(&db, &query).unwrap().evidence_cards;
        assert_eq!(cards.len(), 1);
        let encoded = serde_json::to_string(&cards[0]).unwrap();
        assert!(!encoded.contains("privateCODE"));
        assert!(!encoded.contains("alice@example.com"));
        assert!(encoded.contains("[PRIVATE]"));
        // Saving the same rules does not discard the rebuilt index or safe history.
        std::fs::write(&current_path, "allowance privateCODE 700").unwrap();
        ingest::scan_source(&db, &source.id).unwrap();
        db.save_privacy_config(&config).unwrap();
        assert!(search::resolve_evidence_ref(&db, cards[0].evidence_ref.as_ref().unwrap()).is_ok());
        let mut invalid = config.clone();
        invalid.redact_patterns[0].pattern = "[".into();
        assert!(db.save_privacy_config(&invalid).is_err());
        assert_eq!(
            config_fingerprint(&db.load_privacy_config().unwrap()).unwrap(),
            config_fingerprint(&config).unwrap()
        );
        assert!(search::resolve_evidence_ref(&db, cards[0].evidence_ref.as_ref().unwrap()).is_ok());
        // A later additional rule revokes previously valid history too.
        config.redact_patterns.push(RedactRule {
            name: "amount".into(),
            pattern: "600|700".into(),
            replacement: "[AMOUNT]".into(),
        });
        db.save_privacy_config(&config).unwrap();
        assert!(
            search::resolve_evidence_ref(&db, cards[0].evidence_ref.as_ref().unwrap()).is_err()
        );
        assert!(db.integrity_check().unwrap());
    }

    #[test]
    fn a_scan_started_before_a_privacy_change_cannot_commit_its_old_policy() {
        use crate::{ingest, sources::CreateSourceInput};
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("current.md"),
            "contact alice@example.com",
        )
        .unwrap();
        let db = Database::open_memory().unwrap();
        db.save_privacy_config(&PrivacyConfig {
            enabled: false,
            ..Default::default()
        })
        .unwrap();
        let source = db
            .add_source(CreateSourceInput {
                root_path: folder.path().to_string_lossy().into(),
                include_globs: vec!["**/*.md".into()],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();
        let switched = std::cell::Cell::new(false);
        let result = ingest::scan_source_with_progress(&db, &source.id, |_| {
            if !switched.replace(true) {
                db.save_privacy_config(&PrivacyConfig::default()).unwrap();
            }
        });
        assert!(switched.get());
        assert!(matches!(result, Err(CoreError::Conflict(_))));
        assert_eq!(
            db.conn()
                .query_row("SELECT COUNT(*) FROM chunks", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn explicit_scan_privacy_also_revokes_archives_from_removed_files() {
        use crate::{ingest, models::SearchQuery, search, sources::CreateSourceInput};
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("removed.md");
        std::fs::write(&path, "contact alice@example.com").unwrap();
        let db = Database::open_memory().unwrap();
        let disabled = PrivacyConfig {
            enabled: false,
            ..Default::default()
        };
        db.save_privacy_config(&disabled).unwrap();
        let source = db
            .add_source(CreateSourceInput {
                root_path: folder.path().to_string_lossy().into(),
                include_globs: vec!["**/*.md".into()],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();
        ingest::scan_source(&db, &source.id).unwrap();
        let card = search::search(
            &db,
            &SearchQuery {
                text: "contact".into(),
                filters: Default::default(),
                limit: 1,
                offset: 0,
            },
        )
        .unwrap()
        .evidence_cards
        .remove(0);
        std::fs::remove_file(&path).unwrap();
        ingest::scan_source(&db, &source.id).unwrap();
        assert!(search::resolve_evidence_ref(&db, card.evidence_ref.as_ref().unwrap()).is_ok());
        ingest::scan_source_with_privacy(&db, &source.id, Some(&PrivacyConfig::default())).unwrap();
        assert!(search::resolve_evidence_ref(&db, card.evidence_ref.as_ref().unwrap()).is_err());
        assert!(!db.load_privacy_config().unwrap().enabled);
    }

    // -- Exclude patterns ---------------------------------------------------

    #[test]
    fn test_default_exclude_patterns() {
        let patterns = default_exclude_patterns();
        assert!(patterns.contains(&"**/.git/**".to_string()));
        assert!(patterns.contains(&"**/.env".to_string()));
        assert!(patterns.contains(&"**/.env.*".to_string()));
        assert!(patterns.contains(&"**/node_modules/**".to_string()));
        assert!(patterns.contains(&"**/*.key".to_string()));
        assert!(patterns.contains(&"**/*.pem".to_string()));
        assert!(patterns.contains(&"**/*.p12".to_string()));
        assert!(patterns.contains(&"**/secrets.*".to_string()));
        assert!(patterns.contains(&"**/credentials.*".to_string()));
        assert!(patterns.contains(&"**/.ssh/**".to_string()));
    }

    #[test]
    fn test_default_exclude_patterns_valid_globs() {
        use globset::Glob;
        for p in default_exclude_patterns() {
            Glob::new(&p).unwrap_or_else(|_| panic!("invalid glob: {p}"));
        }
    }

    // -- Email redaction ----------------------------------------------------

    #[test]
    fn test_redact_email() {
        let input = "Contact alice@example.com for details.";
        let output = redact_content(input, &[]);
        assert_eq!(output, "Contact [EMAIL] for details.");
    }

    #[test]
    fn test_redact_multiple_emails() {
        let input = "From: a@b.co To: x@y.org";
        let output = redact_content(input, &[]);
        assert!(output.contains("[EMAIL]"));
        assert!(!output.contains("a@b.co"));
        assert!(!output.contains("x@y.org"));
    }

    // -- IP redaction -------------------------------------------------------

    #[test]
    fn test_redact_ipv4() {
        let input = "Server at 192.168.1.42 responded.";
        let output = redact_content(input, &[]);
        assert_eq!(output, "Server at [IP] responded.");
    }

    #[test]
    fn test_redact_multiple_ips() {
        let input = "10.0.0.1 and 172.16.0.5 are internal.";
        let output = redact_content(input, &[]);
        assert!(!output.contains("10.0.0.1"));
        assert!(!output.contains("172.16.0.5"));
        assert!(output.contains("[IP]"));
    }

    // -- API-key redaction --------------------------------------------------

    #[test]
    fn test_redact_api_key_equals() {
        let input = r#"api_key=ABCD1234EFGH5678IJKL"#;
        let output = redact_content(input, &[]);
        assert!(output.contains("[REDACTED]"), "got: {output}");
        assert!(!output.contains("ABCD1234EFGH5678IJKL"));
    }

    #[test]
    fn test_redact_token_colon() {
        let input = r#"token: sk_live_abc123def456ghi789"#;
        let output = redact_content(input, &[]);
        assert!(output.contains("[REDACTED]"), "got: {output}");
        assert!(!output.contains("sk_live_abc123def456ghi789"));
    }

    #[test]
    fn test_redact_secret_quoted() {
        let input = r#"secret="MyS3cretV4lue_That_Is_Long""#;
        let output = redact_content(input, &[]);
        assert!(output.contains("[REDACTED]"), "got: {output}");
    }

    #[test]
    fn test_short_value_not_redacted() {
        // Values shorter than 16 chars should NOT match the api_key rule.
        let input = "key=short";
        let output = redact_content(input, &[]);
        assert_eq!(output, "key=short");
    }

    // -- Custom rules -------------------------------------------------------

    #[test]
    fn test_custom_redact_rule() {
        let custom = vec![RedactRule {
            name: "ssn".into(),
            pattern: r"\d{3}-\d{2}-\d{4}".into(),
            replacement: "[SSN]".into(),
        }];
        let input = "SSN: 123-45-6789 on file.";
        let output = redact_content(input, &custom);
        assert_eq!(output, "SSN: [SSN] on file.");
    }

    #[test]
    fn test_invalid_custom_rule_ignored() {
        let custom = vec![RedactRule {
            name: "bad".into(),
            pattern: r"[invalid".into(), // unclosed bracket
            replacement: "[X]".into(),
        }];
        let input = "unchanged";
        let output = redact_content(input, &custom);
        assert_eq!(output, "unchanged");
    }

    // -- Mixed content ------------------------------------------------------

    #[test]
    fn test_redact_mixed_content() {
        let input = "Contact admin@corp.io at 10.0.0.1, token: AAAA1111BBBB2222CCCC";
        let output = redact_content(input, &[]);
        assert!(!output.contains("admin@corp.io"));
        assert!(!output.contains("10.0.0.1"));
        assert!(!output.contains("AAAA1111BBBB2222CCCC"));
    }

    // -- DB persistence -----------------------------------------------------

    #[test]
    fn test_save_and_load_privacy_config() {
        let db = Database::open_memory().unwrap();

        let config = PrivacyConfig {
            exclude_patterns: vec!["**/secret/**".into()],
            redact_patterns: vec![RedactRule {
                name: "phone".into(),
                pattern: r"\d{3}-\d{4}".into(),
                replacement: "[PHONE]".into(),
            }],
            enabled: true,
        };

        db.save_privacy_config(&config).unwrap();
        let loaded = db.load_privacy_config().unwrap();

        assert_eq!(loaded.exclude_patterns, config.exclude_patterns);
        assert_eq!(loaded.redact_patterns.len(), 1);
        assert_eq!(loaded.redact_patterns[0].name, "phone");
        assert!(loaded.enabled);
    }

    #[test]
    fn test_load_default_when_no_config_saved() {
        let db = Database::open_memory().unwrap();
        let loaded = db.load_privacy_config().unwrap();
        assert!(loaded.enabled);
        assert_eq!(loaded.exclude_patterns, default_exclude_patterns());
        assert!(loaded.redact_patterns.is_empty());
    }

    #[test]
    fn test_save_overwrites_existing() {
        let db = Database::open_memory().unwrap();

        let v1 = PrivacyConfig {
            exclude_patterns: vec!["a".into()],
            redact_patterns: vec![],
            enabled: true,
        };
        db.save_privacy_config(&v1).unwrap();

        let v2 = PrivacyConfig {
            exclude_patterns: vec!["b".into()],
            redact_patterns: vec![],
            enabled: false,
        };
        db.save_privacy_config(&v2).unwrap();

        let loaded = db.load_privacy_config().unwrap();
        assert_eq!(loaded.exclude_patterns, vec!["b".to_string()]);
        assert!(!loaded.enabled);
    }
}
