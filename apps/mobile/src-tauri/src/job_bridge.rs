//! Activity-free Android JobService entry. All work is admitted by the same
//! LocalRuntime and persisted scheduler used by the foreground composer.
use crate::{
    local::{host::RunHost, LocalRuntime},
    PHONE_RUNTIME,
};
use jni::{
    objects::{JClass, JString},
    sys::jstring,
    JNIEnv,
};
use nexa_core::{agent::CancellationToken, connection_sync::SharedModelConnection};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

// onStopJob can arrive between scheduling the Java thread and JNI entry.
// Keep its cancellation briefly so that the delayed entry cannot launch.
static WORKERS: LazyLock<Mutex<HashMap<String, (CancellationToken, Instant)>>> =
    LazyLock::new(Default::default);

fn worker(id: &str) -> CancellationToken {
    let mut workers = WORKERS.lock().unwrap_or_else(|e| e.into_inner());
    workers
        .retain(|_, (token, at)| !token.is_cancelled() || at.elapsed() < Duration::from_secs(600));
    workers
        .entry(id.to_owned())
        .or_insert_with(|| (CancellationToken::new(), Instant::now()))
        .0
        .clone()
}

fn retry() -> Value {
    json!({"status":"error","nextWakeAt":(chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339()})
}

async fn tick(
    directory: PathBuf,
    key: String,
    connection: String,
    cancel: CancellationToken,
) -> Value {
    if !directory.join("nexa-mobile.db").is_file() {
        return json!({"status":"idle","nextWakeAt":null});
    }
    let runtime = PHONE_RUNTIME
        .get_or_try_init(|| async {
            nexa_core::crypto::install_host_key(&key).map_err(|_| ())?;
            LocalRuntime::open(directory).await.map_err(|_| ())
        })
        .await;
    let Ok(runtime) = runtime else { return retry() };
    let connection: Option<Box<SharedModelConnection>> = if connection.is_empty() {
        None
    } else {
        match serde_json::from_str(&connection) {
            Ok(record) => record,
            Err(_) => return retry(),
        }
    };
    let (finished, result) = oneshot::channel();
    let host = RunHost::Worker {
        connection,
        cancel,
        finished,
    };
    let mut outcome = match runtime.tick_schedules(host).await {
        Ok(value) if value["status"] == "running" => result.await.unwrap_or_else(|_| retry()),
        Ok(value) => value,
        Err(_) => retry(),
    };
    let _guard = runtime.arm_lock.lock().await;
    match runtime.schedule_wake_snapshot() {
        Ok(Value::Object(snapshot)) => outcome.as_object_mut().unwrap().extend(snapshot),
        _ => outcome["nextWakeAt"] = retry()["nextWakeAt"].clone(),
    }
    outcome
}

fn string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Option<String> {
    env.get_string(value).ok().map(Into::into)
}

#[no_mangle]
pub extern "system" fn Java_com_nexa_mobile_PhoneScheduleBridge_tick(
    mut env: JNIEnv,
    _class: JClass,
    directory: JString,
    key: JString,
    connection: JString,
    id: JString,
) -> jstring {
    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let id = string(&mut env, &id)?;
        let directory = string(&mut env, &directory)?;
        let key = string(&mut env, &key)?;
        let connection = string(&mut env, &connection)?;
        let cancel = worker(&id);
        let result =
            tauri::async_runtime::block_on(tick(directory.into(), key, connection, cancel));
        WORKERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        Some(result)
    }))
    .ok()
    .flatten()
    .unwrap_or_else(retry);
    env.new_string(response.to_string())
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_nexa_mobile_PhoneScheduleBridge_stop(
    mut env: JNIEnv,
    _class: JClass,
    id: JString,
) {
    if let Some(id) = string(&mut env, &id) {
        worker(&id).cancel();
    }
}
