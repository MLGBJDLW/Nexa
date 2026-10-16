//! Bounded, cancellation-safe presentation coordination. These acknowledgements
//! describe decoration only; host leases and target checks still authorize input.
use super::webview_host::eval_json;
use std::time::Duration;
use tauri::Webview;

struct FeedbackScope {
    webview: Webview,
    id: String,
    cleanup: &'static str,
}

impl FeedbackScope {
    fn new(webview: &Webview, id: &str, cleanup: &'static str) -> Self {
        Self {
            webview: webview.clone(),
            id: serde_json::to_string(id).expect("string"),
            cleanup,
        }
    }

    fn expression(&self, method: &str) -> String {
        format!("window.__NEXA_BROWSER_RUNTIME__?.{method}({})", self.id)
    }

    async fn wait(&self, method: &str, ready: &str) -> Result<(), String> {
        tokio::time::timeout(Duration::from_millis(1500), async {
            loop {
                let value = eval_json(&self.webview, &self.expression(method)).await?;
                if serde_json::to_string(&value["id"]).ok().as_deref() != Some(&self.id) {
                    return Err("Browser presentation identity changed; observe again".into());
                }
                match value["status"].as_str() {
                    Some(status) if status == ready => return Ok(()),
                    Some("pending") => tokio::time::sleep(Duration::from_millis(16)).await,
                    _ => return Err("Browser presentation was interrupted; observe again".into()),
                }
            }
        })
        .await
        .map_err(|_| "Browser feedback did not settle before capture or input".to_string())?
    }
}

impl Drop for FeedbackScope {
    fn drop(&mut self) {
        let _ = self.webview.eval(&self.expression(self.cleanup));
    }
}

pub(super) struct PresentationGuard(FeedbackScope);
impl PresentationGuard {
    pub fn new(webview: &Webview, id: &str) -> Self {
        Self(FeedbackScope::new(webview, id, "cancelPresentation"))
    }
    pub async fn wait_for_arrival(&self) -> bool {
        if self.0.wait("presentationStatus", "arrived").await.is_ok() {
            return true;
        }
        // Decoration is optional. Remove an interrupted/stalled animation and
        // let the caller revalidate the real lease and target before input.
        let _ = self
            .0
            .webview
            .eval(&self.0.expression("cancelPresentation"));
        false
    }
}

pub(super) struct CaptureFeedbackGuard(FeedbackScope);
impl CaptureFeedbackGuard {
    pub async fn begin(webview: &Webview) -> Result<Self, String> {
        let scope = Self(FeedbackScope::new(
            webview,
            &uuid::Uuid::new_v4().to_string(),
            "endCapture",
        ));
        eval_json(webview, &scope.0.expression("beginCapture")).await?;
        scope.0.wait("captureStatus", "ready").await?;
        Ok(scope)
    }
}
