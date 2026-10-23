//! Validate a storage migration on an explicitly supplied disposable database.
//!
//! cargo run -p nexa-core --no-default-features --example storage_upgrade_probe -- \
//!   --baseline "$PWD/.artifacts/deep-storage-validation/baseline.db" \
//!   --copy "$PWD/.artifacts/deep-storage-validation/candidate.db" \
//!   --report "$PWD/.artifacts/deep-storage-validation/probe.json"
//!
//! Supply absolute paths. Both databases must be separate ordinary files inside the indicated
//! workspace directory. This program never copies databases, opens the live app
//! database, restores checkpoints, starts providers, or prints stored contents.
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use nexa_core::db::Database;
use nexa_core::db_executor::DatabaseExecutor;
use rusqlite::{types::Value as SqlValue, Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const PAGE_SIZE: u32 = 8;
const COUNT_TABLES: &[&str] = &[
    "conversations",
    "messages",
    "archived_messages",
    "conversation_turns",
    "agent_task_runs",
    "agent_run_events",
    "task_resume_checkpoints",
    "agent_task_artifacts",
    "agent_task_artifact_versions",
    "documents",
    "chunks",
    "user_memories",
    "provider_turn_envelopes",
];

fn fail(message: impl Into<String>) -> Box<dyn Error> {
    io::Error::other(message.into()).into()
}

fn require(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(fail(message))
    }
}

fn reject_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        require(
            !metadata.file_type().is_symlink(),
            "Linked probe paths are not allowed",
        )?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            require(
                metadata.file_attributes() & 0x400 == 0,
                "Reparse points are not allowed",
            )?;
        }
    }
    Ok(())
}

fn confined_file(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = std::env::current_dir()?.join(path);
    reject_links(&path)?;
    let path = path.canonicalize()?;
    require(
        path.starts_with(root) && path != root,
        "Database must be inside .artifacts/deep-storage-validation",
    )?;
    require(
        path.is_file() && fs::metadata(&path)?.len() >= 100,
        "Expected an existing database copy",
    )?;
    Ok(path)
}

fn arguments() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = workspace
        .join(".artifacts/deep-storage-validation")
        .canonicalize()?;
    reject_links(&root)?;
    let mut args = std::env::args().skip(1);
    let mut options = BTreeMap::new();
    while let Some(key) = args.next() {
        require(
            ["--baseline", "--copy", "--report"].contains(&key.as_str()),
            "Expected --baseline, --copy and --report",
        )?;
        let value = args.next().ok_or_else(|| fail("Missing option value"))?;
        require(
            options.insert(key, PathBuf::from(value)).is_none(),
            "Duplicate option",
        )?;
    }
    require(
        options.len() == 3,
        "Explicit --baseline, --copy and --report are required",
    )?;
    let baseline = confined_file(&root, &options["--baseline"])?;
    let copy = confined_file(&root, &options["--copy"])?;
    require(
        baseline != copy,
        "Baseline and migration copy must be different files",
    )?;
    require(
        copy.file_name().is_some_and(|name| name != "baseline.db"),
        "The preserved baseline cannot be the migration target",
    )?;
    let report = &options["--report"];
    let parent = report
        .parent()
        .ok_or_else(|| fail("Report needs a parent directory"))?;
    let parent = std::env::current_dir()?.join(parent);
    reject_links(&parent)?;
    let parent = parent.canonicalize()?;
    require(
        parent.starts_with(&root),
        "Report must stay inside the validation directory",
    )?;
    require(
        report.extension().is_some_and(|ext| ext == "json"),
        "Report must be a JSON file",
    )?;
    let report = parent.join(
        report
            .file_name()
            .ok_or_else(|| fail("Missing report filename"))?,
    );
    require(
        !report.exists(),
        "Report already exists; choose a new report filename",
    )?;
    Ok((baseline, copy, report))
}

fn read_only(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // busy_timeout returns a result row, which execute_batch rejects when
    // rusqlite's extra_check feature is enabled by the desktop feature graph.
    for pragma in ["PRAGMA query_only=ON", "PRAGMA busy_timeout=5000"] {
        let mut statement = connection.prepare(pragma)?;
        let mut rows = statement.query([])?;
        while rows.next()?.is_some() {}
    }
    Ok(connection)
}

fn feed_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

// Typed, length-delimited hashing avoids serializing another large JSON string.
// Object member order and whitespace are irrelevant; array order and SQL NULL
// versus JSON null are retained. Stored text and identifiers never leave memory.
fn feed_json(hasher: &mut blake3::Hasher, value: &Value) {
    match value {
        Value::Null => {
            hasher.update(b"n");
        }
        Value::Bool(value) => {
            hasher.update(if *value { b"t" } else { b"f" });
        }
        Value::Number(value) => {
            hasher.update(b"d");
            feed_bytes(hasher, value.to_string().as_bytes());
        }
        Value::String(value) => {
            hasher.update(b"s");
            feed_bytes(hasher, value.as_bytes());
        }
        Value::Array(values) => {
            hasher.update(b"a");
            hasher.update(&(values.len() as u64).to_le_bytes());
            for value in values {
                feed_json(hasher, value);
            }
        }
        Value::Object(values) => {
            hasher.update(b"o");
            hasher.update(&(values.len() as u64).to_le_bytes());
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (key, value) in entries {
                feed_bytes(hasher, key.as_bytes());
                feed_json(hasher, value);
            }
        }
    }
}

fn json_digest(value: Option<&Value>) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    match value {
        None => {
            hasher.update(b"sql-null");
        }
        Some(value) => {
            hasher.update(b"json");
            feed_json(&mut hasher, value);
        }
    }
    hasher.finalize()
}

struct ExpectedJson {
    id: String,
    digest: blake3::Hash,
    original_utf8_bytes: u64,
}

#[derive(Clone, Serialize, PartialEq, Eq)]
struct LogicalColumn {
    rows: usize,
    sql_nulls: usize,
    original_utf8_bytes: u64,
    largest_original_utf8_bytes: u64,
    logical_digest: String,
}

struct BaselineColumn {
    table: &'static str,
    column: &'static str,
    records: Vec<ExpectedJson>,
    report: LogicalColumn,
}

fn baseline_column(
    connection: &Connection,
    table: &'static str,
    column: &'static str,
) -> Result<BaselineColumn> {
    let mut statement =
        connection.prepare(&format!("SELECT id,{column} FROM {table} ORDER BY id"))?;
    let mut rows = statement.query([])?;
    let mut records = Vec::new();
    let mut hasher = blake3::Hasher::new();
    let mut sql_nulls = 0;
    let mut bytes = 0;
    let mut largest = 0;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let text: Option<String> = row
            .get(1)
            .map_err(|_| fail("Baseline must contain original TEXT or NULL snapshots"))?;
        let length = text.as_ref().map_or(0, |text| text.len() as u64);
        bytes += length;
        largest = largest.max(length);
        sql_nulls += usize::from(text.is_none());
        let value = text
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(|_| {
                fail(format!(
                    "Invalid baseline JSON in {table}; id digest {}",
                    blake3::hash(id.as_bytes())
                ))
            })?;
        let digest = json_digest(value.as_ref());
        feed_bytes(&mut hasher, id.as_bytes());
        hasher.update(digest.as_bytes());
        records.push(ExpectedJson {
            id,
            digest,
            original_utf8_bytes: length,
        });
    }
    let report = LogicalColumn {
        rows: records.len(),
        sql_nulls,
        original_utf8_bytes: bytes,
        largest_original_utf8_bytes: largest,
        logical_digest: hasher.finalize().to_hex().to_string(),
    };
    Ok(BaselineColumn {
        table,
        column,
        records,
        report,
    })
}

fn validate_column(database: &Database, baseline: &BaselineColumn) -> Result<LogicalColumn> {
    let mut hasher = blake3::Hasher::new();
    let mut nulls = 0;
    for record in &baseline.records {
        let value = if baseline.table == "conversation_turns" {
            database
                .get_conversation_turn(&record.id)
                .map(|turn| turn.trace)
        } else {
            database
                .get_agent_task_run(&record.id)
                .map(|run| run.artifacts)
        }
        .map_err(|_| {
            fail(format!(
                "Public snapshot hydration failed in {}; id digest {}",
                baseline.table,
                blake3::hash(record.id.as_bytes())
            ))
        })?;
        nulls += usize::from(value.is_none());
        let digest = json_digest(value.as_ref());
        require(
            digest == record.digest,
            format!(
                "Logical snapshot changed in {}; id digest {}",
                baseline.table,
                blake3::hash(record.id.as_bytes())
            ),
        )?;
        feed_bytes(&mut hasher, record.id.as_bytes());
        hasher.update(digest.as_bytes());
    }
    let mut report = baseline.report.clone();
    report.sql_nulls = nulls;
    report.logical_digest = hasher.finalize().to_hex().to_string();
    require(
        report == baseline.report,
        "Aggregate snapshot fingerprint changed",
    )?;
    Ok(report)
}

fn table_counts(connection: &Connection) -> Result<BTreeMap<String, u64>> {
    COUNT_TABLES
        .iter()
        .map(|table| {
            let count =
                connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })?;
            Ok(((*table).to_owned(), count))
        })
        .collect()
}

fn scalar(value: SqlValue) -> Result<Value> {
    Ok(match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(value) => json!(value),
        SqlValue::Real(value) => json!(value),
        SqlValue::Text(value) => Value::String(value),
        SqlValue::Blob(_) => return Err(fail("Unexpected BLOB in functional metadata")),
    })
}

fn row_digest(row: &rusqlite::Row<'_>, columns: usize) -> Result<blake3::Hash> {
    let mut hasher = blake3::Hasher::new();
    for index in 0..columns {
        feed_json(&mut hasher, &scalar(row.get(index)?)?);
    }
    Ok(hasher.finalize())
}

fn functional_state(connection: &Connection) -> Result<BTreeMap<String, Value>> {
    let queries = [
        ("settings_schema_state", "SELECT singleton_id,active_version,migration_id FROM settings_schema_state ORDER BY singleton_id", 3),
        ("registry_activation_state", "SELECT capability_id,scope_kind,scope_id,read_mode,registry_revision,parity_status FROM registry_activation_state ORDER BY capability_id,scope_kind,scope_id", 6),
        ("turn_display_projection", "SELECT id,display_revision,display_trace_flags FROM conversation_turns ORDER BY id", 3),
    ];
    let mut result = BTreeMap::new();
    for (name, sql, columns) in queries {
        let mut statement = connection.prepare(sql)?;
        let mut rows = statement.query([])?;
        let mut count = 0;
        let mut hasher = blake3::Hasher::new();
        while let Some(row) = rows.next()? {
            hasher.update(row_digest(row, columns)?.as_bytes());
            count += 1;
        }
        result.insert(
            name.to_string(),
            json!({"rows":count,"digest":hasher.finalize().to_hex().to_string()}),
        );
    }
    Ok(result)
}

fn migrations(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare("SELECT name FROM _migrations ORDER BY name")?;
    let values = statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(values)
}

fn stored_formats(
    connection: &Connection,
    columns: &[BaselineColumn],
) -> Result<BTreeMap<String, Value>> {
    let mut result = BTreeMap::new();
    for column in columns {
        let mut statement = connection.prepare(&format!("SELECT typeof({0}),COUNT(*),COALESCE(SUM(length(CAST({0} AS BLOB))),0) FROM {1} GROUP BY typeof({0}) ORDER BY typeof({0})", column.column, column.table))?;
        let values = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        require(
            values
                .iter()
                .all(|(kind, _, _)| ["text", "blob", "null"].contains(&kind.as_str())),
            "Unexpected snapshot storage type",
        )?;
        result.insert(column.table.to_string(), json!(values));
    }
    Ok(result)
}

fn selected_runs(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT run_id FROM agent_run_events GROUP BY run_id ORDER BY COUNT(*) DESC,run_id",
    )?;
    let runs = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut selected = runs
        .iter()
        .take(2)
        .chain(runs.iter().rev().take(2))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut newest = connection
        .prepare("SELECT id FROM agent_task_runs ORDER BY created_at DESC,id DESC LIMIT 2")?;
    for run in newest.query_map([], |row| row.get::<_, String>(0))? {
        selected.insert(run?);
    }
    Ok(selected.into_iter().collect())
}

fn check_event_page(
    database: &Database,
    baseline: &Connection,
    run: &str,
    after: u64,
    high: u64,
    capture_head: bool,
) -> Result<(usize, Option<u64>)> {
    let page = database
        .list_agent_run_event_page(
            run,
            after,
            if capture_head { None } else { Some(high) },
            PAGE_SIZE,
        )
        .map_err(|_| fail("Public run-event page failed to hydrate"))?;
    require(
        page.durable_high_water == high,
        "Run-event high-water changed",
    )?;
    let mut sequences = baseline.prepare("SELECT event_seq FROM agent_run_events WHERE run_id=?1 AND event_seq>?2 AND event_seq<=?3 ORDER BY event_seq LIMIT ?4")?;
    let expected = sequences
        .query_map(rusqlite::params![run, after, high, PAGE_SIZE], |row| {
            row.get::<_, u64>(0)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    require(
        page.events
            .iter()
            .map(|event| event.event_seq)
            .eq(expected.iter().copied()),
        "Run-event page changed sequence membership",
    )?;
    for event in &page.events {
        let mut statement = baseline.prepare("SELECT version,run_id,turn_id,event_seq,kind,phase,visibility,persistence,display_kind,importance,label,status,created_at,payload_json FROM agent_run_events WHERE run_id=?1 AND event_seq=?2")?;
        let mut rows = statement.query(rusqlite::params![run, event.event_seq])?;
        let row = rows
            .next()?
            .ok_or_else(|| fail("Baseline run event disappeared"))?;
        let header = json!([
            event.version,
            event.run_id,
            event.turn_id,
            event.event_seq,
            event.kind.as_str(),
            event.phase.as_str(),
            event.visibility.as_str(),
            event.persistence.as_str(),
            event.display_kind.as_str(),
            event.importance.as_str(),
            event.label,
            event.status,
            event.created_at
        ]);
        let mut header_hasher = blake3::Hasher::new();
        for value in header.as_array().expect("array literal") {
            feed_json(&mut header_hasher, value);
        }
        require(
            header_hasher.finalize() == row_digest(row, 13)?,
            "Run-event metadata changed",
        )?;
        let payload: String = row.get(13)?;
        let payload: Value = serde_json::from_str(&payload)?;
        require(
            json_digest(Some(&payload)) == json_digest(Some(&event.payload)),
            "Run-event payload changed",
        )?;
    }
    let next = page.events.last().map(|event| event.event_seq);
    require(
        next == page.next_after_event_seq,
        "Run-event cursor disagrees with its page",
    )?;
    let has_more: bool = baseline.query_row("SELECT EXISTS(SELECT 1 FROM agent_run_events WHERE run_id=?1 AND event_seq>?2 AND event_seq<=?3)", rusqlite::params![run, next.unwrap_or(after), high], |row| row.get(0))?;
    require(
        page.has_more == has_more,
        "Run-event continuation flag changed",
    )?;
    Ok((page.events.len(), next))
}

fn check_recovery(database: &Database, baseline: &Connection) -> Result<Value> {
    let runs = selected_runs(baseline)?;
    let mut pages = 0;
    let mut event_observations = 0;
    let mut checkpoints = BTreeSet::new();
    for run in &runs {
        let high: u64 = baseline.query_row(
            "SELECT COALESCE(MAX(event_seq),0) FROM agent_run_events WHERE run_id=?1",
            [run],
            |row| row.get(0),
        )?;
        let (count, next) = check_event_page(database, baseline, run, 0, high, true)?;
        pages += 1;
        event_observations += count;
        if let Some(next) = next {
            let (count, _) = check_event_page(database, baseline, run, next, high, false)?;
            pages += 1;
            event_observations += count;
        }
        let (count, _) = check_event_page(
            database,
            baseline,
            run,
            high.saturating_sub(u64::from(PAGE_SIZE)),
            high,
            false,
        )?;
        pages += 1;
        event_observations += count;
        for direction in ["ASC", "DESC"] {
            let mut statement = baseline.prepare(&format!("SELECT id FROM task_resume_checkpoints WHERE run_id=?1 ORDER BY created_at {direction},id {direction} LIMIT 1"))?;
            for id in statement.query_map([run], |row| row.get::<_, String>(0))? {
                checkpoints.insert(id?);
            }
        }
        // Both UI visibility modes exercise the payload-free Task Center path.
        database
            .get_agent_task_history(run, false)
            .map_err(|_| fail("User task-history projection failed"))?;
        database
            .get_agent_task_history(run, true)
            .map_err(|_| fail("Developer task-history projection failed"))?;
    }
    let mut largest = baseline.prepare("SELECT id FROM task_resume_checkpoints ORDER BY length(CAST(state_json AS BLOB)) DESC,id LIMIT 1")?;
    for id in largest.query_map([], |row| row.get::<_, String>(0))? {
        checkpoints.insert(id?);
    }
    for id in &checkpoints {
        let checkpoint = database
            .get_task_resume_checkpoint(id)
            .map_err(|_| fail("Public resume checkpoint failed to hydrate"))?;
        let mut statement = baseline.prepare("SELECT id,run_id,reason,status,phase,resume_prompt,created_at,state_json FROM task_resume_checkpoints WHERE id=?1")?;
        let mut rows = statement.query([id])?;
        let row = rows
            .next()?
            .ok_or_else(|| fail("Baseline checkpoint disappeared"))?;
        let metadata = json!([
            checkpoint.id,
            checkpoint.run_id,
            checkpoint.reason,
            checkpoint.status,
            checkpoint.phase,
            checkpoint.resume_prompt,
            checkpoint.created_at
        ]);
        let mut hasher = blake3::Hasher::new();
        for value in metadata.as_array().expect("array literal") {
            feed_json(&mut hasher, value);
        }
        require(
            hasher.finalize() == row_digest(row, 7)?,
            "Checkpoint metadata changed",
        )?;
        let state: String = row.get(7)?;
        let state: Value = serde_json::from_str(&state)?;
        require(
            json_digest(Some(&state)) == json_digest(Some(&checkpoint.state)),
            "Checkpoint state changed",
        )?;
    }
    Ok(
        json!({"scope":"two largest, two smallest and two newest runs; first, continuation and suffix pages; boundary and largest checkpoints",
        "run_count":runs.len(),"pages":pages,"event_observations_including_overlap":event_observations,
        "checkpoint_records":checkpoints.len(),"page_limit":PAGE_SIZE}),
    )
}

fn check_reader_lanes(
    database: &Database,
    connection: &Connection,
    columns: &[BaselineColumn],
) -> Result<usize> {
    let executor = DatabaseExecutor::new(database.clone(), 2)
        .map_err(|_| fail("Read-only database lane initialization failed"))?;
    let mut checked = 0;
    for column in columns {
        let id: String = connection
            .query_row(
                &format!(
                    "SELECT id FROM {} WHERE typeof({})='blob' ORDER BY length({}) DESC,id LIMIT 1",
                    column.table, column.column, column.column
                ),
                [],
                |row| row.get(0),
            )
            .map_err(|_| {
                fail(format!(
                    "Migration produced no compressed rows in {}",
                    column.table
                ))
            })?;
        let expected = column
            .records
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| fail("Compressed row lacks a baseline"))?
            .digest;
        for control in [false, true] {
            let id = id.clone();
            let is_turn = column.table == "conversation_turns";
            let read = move |database: &Database| {
                let value = if is_turn {
                    database.get_conversation_turn(&id)?.trace
                } else {
                    database.get_agent_task_run(&id)?.artifacts
                };
                Ok(json_digest(value.as_ref()))
            };
            let result = if control {
                futures::executor::block_on(executor.read_control(read))
            } else {
                futures::executor::block_on(executor.read(read))
            }
            .map_err(|_| fail("Compressed snapshot failed through a read-only database lane"))?;
            require(
                result.value == expected,
                "Read-only lane returned different snapshot content",
            )?;
            checked += 1;
        }
    }
    Ok(checked)
}

#[derive(Default)]
struct ByteCounter(u64);

impl io::Write for ByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0 += buffer.len() as u64;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn measure_serialized<T: Serialize>(
    label: &str,
    operation: impl Fn() -> std::result::Result<T, nexa_core::error::CoreError>,
) -> Result<Value> {
    let mut samples = Vec::new();
    let mut read_times = Vec::new();
    let mut total_times = Vec::new();
    let mut stable_bytes = None;
    for _ in 0..3 {
        let started = Instant::now();
        let value = operation().map_err(|_| fail(format!("{label} measurement failed")))?;
        let read_ms = started.elapsed().as_secs_f64() * 1000.;
        // Count the real IPC serialization without retaining or exporting it.
        let mut counter = ByteCounter::default();
        serde_json::to_writer(&mut counter, &value)?;
        let total_ms = started.elapsed().as_secs_f64() * 1000.;
        if let Some(bytes) = stable_bytes {
            require(
                bytes == counter.0,
                "Presentation size changed during a read-only measurement",
            )?;
        }
        stable_bytes = Some(counter.0);
        samples.push(json!({"read_ms":read_ms,"read_and_serialize_ms":total_ms,"serialized_bytes":counter.0}));
        read_times.push(read_ms);
        total_times.push(total_ms);
    }
    let first = samples[0].clone();
    read_times.sort_by(f64::total_cmp);
    total_times.sort_by(f64::total_cmp);
    Ok(json!({"first":first,"median_read_ms":read_times[1],
        "median_read_and_serialize_ms":total_times[1],"serialized_bytes":stable_bytes,
        "samples":samples}))
}

fn measure_largest_task(database: &Database, columns: &[BaselineColumn]) -> Result<Value> {
    let largest = columns
        .iter()
        .find(|column| column.table == "agent_task_runs")
        .and_then(|column| {
            column
                .records
                .iter()
                .max_by_key(|record| record.original_utf8_bytes)
        })
        .ok_or_else(|| fail("No artifact task is available for the real-data measurement"))?;
    let full = measure_serialized("Full task detail", || {
        database.get_agent_task_run(&largest.id)
    })?;
    let presentation = measure_serialized("Task presentation", || {
        database.get_agent_task_run_presentation(&largest.id)
    })?;
    let metadata = measure_serialized("Artifact card metadata", || {
        database.list_agent_task_artifact_metadata(&largest.id)
    })?;
    Ok(
        json!({"scope":"largest original artifacts_json row; same immutable validation copy, three reads per API",
        "cache_state":"after correctness validation; OS caches were not flushed; first observed call is reported separately",
        "original_artifact_json_bytes":largest.original_utf8_bytes,
        "full_task_detail":full,"task_presentation":presentation,"artifact_card_metadata":metadata}),
    )
}

fn main() -> Result<()> {
    let started = Instant::now();
    let (baseline_path, copy_path, report_path) = arguments()?;
    let baseline = read_only(&baseline_path)?;
    let counts = table_counts(&baseline)?;
    let state = functional_state(&baseline)?;
    let baseline_migrations = migrations(&baseline)?;
    let columns = vec![
        baseline_column(&baseline, "conversation_turns", "trace_json")?,
        baseline_column(&baseline, "agent_task_runs", "artifacts_json")?,
    ];
    let baseline_json: BTreeMap<_, _> = columns
        .iter()
        .map(|column| (column.table, column.report.clone()))
        .collect();
    println!(
        "Baseline logical snapshots fingerprinted: {} turn rows, {} task rows",
        columns[0].records.len(),
        columns[1].records.len()
    );
    {
        let untouched_copy = read_only(&copy_path)?;
        require(
            table_counts(&untouched_copy)? == counts,
            "Copy counts do not match the preserved baseline",
        )?;
        require(
            functional_state(&untouched_copy)? == state,
            "Copy functional state differs before migration",
        )?;
        for column in &columns {
            require(
                baseline_column(&untouched_copy, column.table, column.column)?.report
                    == column.report,
                "Copy logical snapshots differ before migration",
            )?;
        }
    }
    let open_started = Instant::now();
    let database = Database::new(&copy_path)
        .map_err(|_| fail("Database::new(copy) failed; live data was not opened"))?;
    let migration_ms = open_started.elapsed().as_secs_f64() * 1000.;
    println!("Copy opened and migrated; validating public hydration and recovery");
    let after: BTreeMap<_, _> = columns
        .iter()
        .map(|column| Ok((column.table, validate_column(&database, column)?)))
        .collect::<Result<_>>()?;
    let after_reader = read_only(&copy_path)?;
    let journal_mode: String =
        after_reader.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    require(
        journal_mode == "wal",
        "File-backed storage did not activate WAL",
    )?;
    require(
        table_counts(&after_reader)? == counts,
        "Authoritative row counts changed",
    )?;
    require(
        functional_state(&after_reader)? == state,
        "Settings or registry activation changed or fell back",
    )?;
    let after_migrations = migrations(&after_reader)?;
    require(
        baseline_migrations
            .iter()
            .all(|name| after_migrations.contains(name)),
        "Existing migration markers disappeared",
    )?;
    let formats = stored_formats(&after_reader, &columns)?;
    let reader_lane_checks = check_reader_lanes(&database, &after_reader, &columns)?;
    let recovery = check_recovery(&database, &baseline)?;
    let presentation_measurement = measure_largest_task(&database, &columns)?;
    drop(after_reader);
    drop(database);
    let database =
        Database::new(&copy_path).map_err(|_| fail("Reopening the migrated copy failed"))?;
    let reopened: BTreeMap<_, _> = columns
        .iter()
        .map(|column| Ok((column.table, validate_column(&database, column)?)))
        .collect::<Result<_>>()?;
    let reopened_reader = read_only(&copy_path)?;
    require(
        table_counts(&reopened_reader)? == counts,
        "Row counts changed on reopen",
    )?;
    require(
        functional_state(&reopened_reader)? == state,
        "Activation state changed on reopen",
    )?;
    require(
        migrations(&reopened_reader)? == after_migrations,
        "Migration markers changed on reopen",
    )?;
    require(
        stored_formats(&reopened_reader, &columns)? == formats,
        "Stored snapshot representation changed on reopen",
    )?;
    require(after == reopened, "Logical hydration is not idempotent")?;
    let report = json!({"success":true,"baseline":baseline_path,"copy":copy_path,
        "digest_algorithm":"BLAKE3 typed canonical JSON v1; SQL NULL distinct from JSON null",
        "baseline_columns":baseline_json,"after_columns":after,"reopened_columns":reopened,
        "authoritative_row_counts":counts,"functional_state":state,"stored_formats":formats,
        "migration_names_before":baseline_migrations,"migration_names_after":after_migrations,
        "migration_open_ms":migration_ms,"reader_lane_checks":reader_lane_checks,"recovery":recovery,
        "journal_mode_after":journal_mode,
        "largest_task_presentation_measurement":presentation_measurement,
        "reopen_idempotent":true,"elapsed_seconds":started.elapsed().as_secs_f64(),
        "scope":"Integrity/FK and all untouched-table row-content checks are separate; no restore/resume/provider/account commands executed"});
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(report_path)?;
    serde_json::to_writer_pretty(output, &report)?;
    println!("Storage upgrade probe passed; all snapshot rows preserved, read lanes and scoped recovery passed");
    Ok(())
}
