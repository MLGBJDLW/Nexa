//! Activity-free Android JobService entry. All work is admitted by the same
//! LocalRuntime and persisted scheduler used by the foreground composer.
use crate::{
    local::{host::RunHost, LocalRuntime},
    PHONE_RUNTIME,
};
use jni::{
    objects::{GlobalRef, JClass, JObject, JString, JValue},
    sys::jstring,
    JNIEnv, JavaVM,
};
use nexa_core::{agent::CancellationToken, connection_sync::SharedModelConnection};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
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

#[derive(Clone)]
struct WorkerAccounts {
    vm: Arc<JavaVM>,
    store: GlobalRef,
}

impl WorkerAccounts {
    fn read_account(&self, id: &str) -> Option<SharedModelConnection> {
        let mut env = self.vm.attach_current_thread().ok()?;
        let lookup = env.with_local_frame(4, |env| -> jni::errors::Result<_> {
            let id = env.new_string(id)?;
            let record = env
                .call_method(
                    self.store.as_obj(),
                    "readConnection",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(&id)],
                )?
                .l()?;
            if record.is_null() {
                return Ok(None);
            }
            let raw: String = env.get_string(&JString::from(record))?.into();
            Ok(serde_json::from_str(&raw).ok())
        });
        lookup.unwrap_or_else(|_| {
            let _ = env.exception_clear();
            None
        })
    }
}

#[async_trait::async_trait]
impl nexa_core::web_search::providers::deepseek::AccountKeys for WorkerAccounts {
    async fn deepseek_key(&self, id: &str) -> Option<String> {
        let accounts = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let source = accounts.read_account(&id)?;
            (source.id == id
                && nexa_core::web_search::providers::deepseek::supports_connection(
                    &source.provider,
                    source.base_url.as_deref(),
                ))
            .then_some(source.api_key)
        })
        .await
        .ok()
        .flatten()
    }
}

async fn tick(
    directory: PathBuf,
    key: String,
    connection: String,
    web_keys: WorkerAccounts,
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
        web_keys: Arc::new(web_keys),
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
    secrets: JObject,
    id: JString,
) -> jstring {
    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let id = string(&mut env, &id)?;
        let directory = string(&mut env, &directory)?;
        let key = string(&mut env, &key)?;
        let connection = string(&mut env, &connection)?;
        let web_keys = WorkerAccounts {
            vm: Arc::new(env.get_java_vm().ok()?),
            store: env.new_global_ref(secrets).ok()?,
        };
        let cancel = worker(&id);
        let result = tauri::async_runtime::block_on(tick(
            directory.into(),
            key,
            connection,
            web_keys,
            cancel,
        ));
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
