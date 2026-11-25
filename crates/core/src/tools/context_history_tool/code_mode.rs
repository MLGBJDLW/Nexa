use super::{CoreError, Database, ReadResult};
use mlua::{
    chunk::ChunkMode, serde::SerializeOptions, Lua, LuaOptions, LuaSerdeExt, LuaString, StdLib,
    Value, VmState,
};
use serde_json::{json, Value as Json};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

// These are per-cell allocation bounds. Pages and cells can be processed for
// the lifetime of the task. The caller's cancellation/timeout stops the VM.
pub(super) const MAX_SOURCE_BYTES: usize = 64 * 1024;
const VM_MEMORY_BYTES: usize = 16 * 1024 * 1024;
pub(super) const VALUE_BYTES: usize = crate::tools::MAX_TOOL_CALL_ARGUMENT_BYTES;
const CELL_BUDGET: Duration = Duration::from_secs(30);
// Keep concurrent VM heaps within 32 MiB on a phone. Further calls queue.
static WORKERS: Semaphore = Semaphore::const_new(2);

fn readback_id(id: LuaString) -> mlua::Result<String> {
    if id.as_bytes().len() > VALUE_BYTES {
        return Err(mlua::Error::runtime("readback identifier exceeds 1 MiB"));
    }
    Ok(id.to_str()?.to_string())
}

pub(super) async fn run(
    db: Database,
    conversation: String,
    code: String,
    parent: Option<&CancellationToken>,
) -> Result<Json, CoreError> {
    if code.len() > MAX_SOURCE_BYTES {
        return Err(CoreError::InvalidInput(
            "Code Mode source exceeds 64 KiB. Split the script into cells.".into(),
        ));
    }
    let cancel = parent
        .map(CancellationToken::child_token)
        .unwrap_or_default();
    // A scheduler timeout can drop this future without cancelling its parent.
    let _worker_guard = cancel.clone().drop_guard();
    let permit = tokio::select! {
        permit = WORKERS.acquire() => permit.expect("Code Mode semaphore stays open"),
        _ = cancel.cancelled() => return Err(CoreError::Cancelled("Code Mode was cancelled.".into())),
    };
    let bridge = super::live::BRIDGE.try_with(Clone::clone).ok();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if !db.load_app_config()?.experimental_code_mode {
            return Err(CoreError::InvalidInput(
                "Code Mode is disabled in settings.".into(),
            ));
        }
        let reader = db.read_only_lane()?;
        let deadline = Instant::now() + CELL_BUDGET;
        if reader.db_path().is_some() {
            // This hook belongs only to this cell's private SQLite connection.
            let cancelled = cancel.clone();
            reader.conn().progress_handler(
                1000,
                Some(move || cancelled.is_cancelled() || Instant::now() >= deadline),
            )?;
        }
        let result = evaluate(
            reader,
            conversation,
            &code,
            cancel.clone(),
            deadline,
            bridge,
        );
        if cancel.is_cancelled() {
            return Err(CoreError::Cancelled("Code Mode was cancelled.".into()));
        }
        result.map_err(|error| CoreError::InvalidInput(format!("Code Mode: {error}")))
    })
    .await
    .map_err(|error| CoreError::Internal(format!("Code Mode worker: {error}")))?
}

fn evaluate(
    db: Database,
    conversation: String,
    code: &str,
    cancel: CancellationToken,
    deadline: Instant,
    bridge: Option<super::live::Bridge>,
) -> mlua::Result<Json> {
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
        LuaOptions::default(),
    )?;
    lua.set_memory_limit(VM_MEMORY_BYTES)?;
    // mlua installs its own filesystem module loader even in Luau. Remove it
    // before freezing globals and expose only the bindings below to the chunk.
    let globals = lua.globals();
    globals.raw_set("require", Value::Nil)?;
    globals.raw_set("loadstring", Value::Nil)?;
    lua.sandbox(true)?;
    let env = lua.create_table()?;
    if let Some(bridge) = bridge {
        bridge.bind(&lua, &env, cancel.clone(), deadline)?;
    }
    for name in [
        "pairs", "ipairs", "next", "select", "tonumber", "tostring", "type", "math", "string",
        "table", "utf8",
    ] {
        env.raw_set(name, globals.raw_get::<Value>(name)?)?;
    }
    let serialize = SerializeOptions::new()
        .serialize_none_to_null(false)
        .serialize_unit_to_null(false);
    let read_cancel = cancel.clone();
    let list_db = db.clone();
    let list_conversation = conversation.clone();
    env.raw_set(
        "list_results",
        lua.create_function(
            move |lua, (turn, offset): (Option<LuaString>, Option<usize>)| {
                let turn = turn.map(readback_id).transpose()?;
                let page = list_db
                    .context_tool_result_refs(
                        &list_conversation,
                        turn.as_deref(),
                        offset.unwrap_or(0),
                    )
                    .map_err(mlua::Error::external)?;
                lua.to_value_with(&page, serialize)
            },
        )?,
    )?;
    env.raw_set(
        "read_result",
        lua.create_function(
            move |lua,
                  (turn_id, tool_call_id, result_id, offset, expected_digest): (
                LuaString,
                LuaString,
                Option<LuaString>,
                Option<usize>,
                Option<LuaString>,
            )| {
                if read_cancel.is_cancelled() {
                    return Err(mlua::Error::runtime("cancelled"));
                }
                let request = ReadResult {
                    turn_id: Some(readback_id(turn_id)?),
                    tool_call_id: Some(readback_id(tool_call_id)?),
                    result_id: result_id.map(readback_id).transpose()?,
                    offset,
                    expected_digest: expected_digest.map(readback_id).transpose()?,
                    max_chars: Some(16_000),
                };
                let page = request
                    .page(&db, &conversation)
                    .map_err(mlua::Error::external)?;
                lua.to_value_with(&page, serialize)
            },
        )?,
    )?;
    env.raw_set(
        "decode_json",
        lua.create_function(move |lua, text: LuaString| {
            if text.as_bytes().len() > VALUE_BYTES {
                return Err(mlua::Error::runtime("decode_json input exceeds 1 MiB"));
            }
            let value: Json =
                serde_json::from_slice(&text.as_bytes()).map_err(mlua::Error::external)?;
            lua.to_value(&value)
        })?,
    )?;
    env.raw_set("json_null", lua.null())?;
    lua.set_interrupt(move |_| {
        if cancel.is_cancelled() {
            Err(mlua::Error::runtime("cancelled"))
        } else if Instant::now() >= deadline {
            Err(mlua::Error::runtime(
                "cell time budget exhausted; resume with smaller pages or a narrower calculation",
            ))
        } else {
            Ok(VmState::Continue)
        }
    });
    let result = lua
        .load(code)
        .set_mode(ChunkMode::Text)
        .set_name("Code Mode")
        .set_environment(env)
        .eval::<Value>()?;
    // Returning scalars avoids unbounded serialization of cyclic/shared tables.
    match result {
        Value::Nil => Ok(Json::Null),
        Value::Boolean(v) => Ok(json!(v)),
        Value::Integer(v) => Ok(json!(v)),
        Value::Number(v) if v.is_finite() => Ok(json!(v)),
        Value::String(s) if s.as_bytes().len() <= VALUE_BYTES => Ok(json!(s.to_str()?.as_ref())),
        Value::String(_) => Err(mlua::Error::runtime(
            "return text exceeds 1 MiB; select a smaller result",
        )),
        _ => Err(mlua::Error::runtime(
            "return a string, finite number, boolean or nil",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_db() -> Database {
        let db = Database::open_memory().unwrap();
        let mut config = db.load_app_config().unwrap();
        config.experimental_code_mode = true;
        db.save_app_config(&config).unwrap();
        db
    }

    #[tokio::test]
    async fn code_mode_has_only_readback_and_pure_bindings() {
        let db = enabled_db();
        let eval = |code: &str| run(db.clone(), "chat".into(), code.into(), None);
        assert_eq!(eval("return require == nil and os == nil and io == nil and loadstring == nil and print == nil and getfenv == nil and _G == nil and tools == nil").await.unwrap(), json!(true));
        assert_eq!(
            eval("leak = 12; return type(read_result)").await.unwrap(),
            json!("function")
        );
        assert_eq!(eval("return leak").await.unwrap(), Json::Null);
        assert!(eval("local a = {}; a.self = a; return a").await.is_err());
        assert!(eval("local a = {}; for i=1,60 do a = {a,a} end; return a")
            .await
            .is_err());
        assert!(eval("return string.rep('x', 1048577)").await.is_err());
        let oversized_id =
            eval("local id = string.rep('x', 1048577); return read_result(id, id, id, 0, id)")
                .await
                .unwrap_err();
        assert!(oversized_id.to_string().contains("identifier exceeds"));
        assert!(eval("return string.rep('x', 67108864)").await.is_err());
        assert!(eval(&" ".repeat(MAX_SOURCE_BYTES + 1)).await.is_err());
    }

    #[tokio::test]
    async fn code_mode_interrupts_a_running_loop() {
        let db = enabled_db();
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let worker = tokio::spawn(async move {
            run(
                db,
                "chat".into(),
                "while true do end".into(),
                Some(&task_cancel),
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(CoreError::Cancelled(_))));
    }

    #[test]
    fn code_mode_cell_deadline_ends_an_unattended_loop() {
        let err = evaluate(
            enabled_db(),
            "chat".into(),
            "while true do end",
            CancellationToken::new(),
            Instant::now() + Duration::from_millis(25),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("time budget"), "{err}");
    }
}
