use super::*;
use nexa_core::connection_sync::SharedModelConnection;
use std::sync::RwLock;

// A JobService may initialize the executor before Android creates an Activity.
// Attach the UI later without replacing its live outbox or recovering it twice.
static UI: RwLock<Option<AppHandle>> = RwLock::new(None);

pub(crate) fn bind_ui(app: AppHandle) {
    *UI.write().unwrap_or_else(|e| e.into_inner()) = Some(app);
}

pub(super) fn ui() -> Option<AppHandle> {
    UI.read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub(crate) enum RunHost {
    Foreground(AppHandle),
    #[cfg(target_os = "android")]
    Worker {
        connection: Option<Box<SharedModelConnection>>,
        cancel: CancellationToken,
        finished: oneshot::Sender<Value>,
    },
}

impl RunHost {
    pub(super) fn app(&self) -> Option<&AppHandle> {
        match self {
            Self::Foreground(app) => Some(app),
            #[cfg(target_os = "android")]
            Self::Worker { .. } => None,
        }
    }

    pub(super) async fn connection(
        &self,
        id: Option<&str>,
    ) -> Result<SharedModelConnection, String> {
        match self {
            Self::Foreground(app) => connections::get(app, id).await,
            #[cfg(target_os = "android")]
            Self::Worker { connection, .. } => connection
                .as_deref()
                .filter(|record| id.is_none_or(|id| record.id == id))
                .cloned()
                .ok_or_else(|| "Configure a model connection on this phone.".into()),
        }
    }

    pub(super) fn cancel_token(&self) -> CancellationToken {
        match self {
            Self::Foreground(_) => CancellationToken::new(),
            #[cfg(target_os = "android")]
            Self::Worker { cancel, .. } => cancel.clone(),
        }
    }

    pub(super) fn check_worker(&self) -> Result<(), String> {
        #[cfg(target_os = "android")]
        if let Self::Worker {
            cancel, connection, ..
        } = self
        {
            if cancel.is_cancelled() {
                return Err("Phone job was stopped.".into());
            }
            if connection.is_none() {
                return Err("Configure a model connection on this phone.".into());
            }
        }
        Ok(())
    }

    // Only interactive tasks own the Activity's notification and Stop channel.
    pub(super) async fn begin(
        &self,
        runtime: &Arc<LocalRuntime>,
        token: &CancellationToken,
    ) -> Result<Option<u32>, String> {
        if token.is_cancelled() {
            return Err("Phone job was stopped.".into());
        }
        let Some(app) = self.app() else {
            return Ok(None);
        };
        let weak = Arc::downgrade(runtime);
        let stopped = token.clone();
        let channel = tauri::ipc::Channel::<Value>::new(move |_| {
            stopped.cancel();
            if let Some(runtime) = weak.upgrade() {
                tauri::async_runtime::spawn(async move {
                    runtime.pending.lock().await.clear();
                });
            }
            Ok(())
        });
        platform::call(
            app,
            "registerListener",
            json!({"event":"taskStop","handler":channel}),
        )
        .await?;
        if let Err(error) = platform::call(app, "taskService", json!({"active":true})).await {
            let _ = platform::call(
                app,
                "removeListener",
                json!({"event":"taskStop","channelId":channel.id()}),
            )
            .await;
            return Err(error);
        }
        Ok(Some(channel.id()))
    }

    pub(super) async fn end(&self, channel: Option<u32>) {
        if let (Some(app), Some(id)) = (self.app(), channel) {
            let _ = platform::call(app, "taskService", json!({"active":false})).await;
            let _ = platform::call(
                app,
                "removeListener",
                json!({"event":"taskStop","channelId":id}),
            )
            .await;
        }
    }

    pub(super) async fn finish(self, result: Value) {
        match self {
            Self::Foreground(app) => {
                crate::overlay::finish(&app, &result).await;
                let _ = platform::call(&app, "taskResult", result).await;
            }
            #[cfg(target_os = "android")]
            Self::Worker { finished, .. } => {
                let _ = finished.send(result);
            }
        }
    }
}
