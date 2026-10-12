//! Ephemeral generated HTML, shared by desktop custom-protocol and remote HTTP
//! adapters. The trusted outer frame restricts *navigation* of the opaque inner
//! frame as well as its subresources. Generated documents have no host bridge.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub const MAX_HTML_BYTES: usize = 512 * 1024;
pub const PREVIEW_TTL: Duration = Duration::from_secs(1800);
pub const HOST_CSP: &str = "sandbox allow-scripts; default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; connect-src 'none'; frame-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'";
pub const DOCUMENT_CSP: &str = "sandbox allow-scripts; default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; worker-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
pub const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), clipboard-read=(), clipboard-write=(), payment=(), usb=(), serial=(), bluetooth=(), fullscreen=()";

#[derive(Default)]
pub struct HtmlPreviewStore {
    entries: HashMap<String, Preview>,
}
struct Preview {
    owner: String,
    html: String,
    created: Instant,
}

impl HtmlPreviewStore {
    pub fn create(&mut self, owner: &str, html: String) -> Result<String, String> {
        if html.len() > MAX_HTML_BYTES {
            return Err("HTML preview exceeds 512 KiB".into());
        }
        self.entries
            .retain(|_, preview| preview.created.elapsed() < PREVIEW_TTL);
        if self.entries.len() >= 24 {
            return Err("Too many active HTML previews; stop an existing preview first".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.entries.insert(
            id.clone(),
            Preview {
                owner: owner.into(),
                html,
                created: Instant::now(),
            },
        );
        Ok(id)
    }
    pub fn read(&self, id: &str) -> Option<(&str, &str)> {
        self.entries
            .get(id)
            .filter(|preview| preview.created.elapsed() < PREVIEW_TTL)
            .map(|preview| (preview.owner.as_str(), preview.html.as_str()))
    }
    pub fn release(&mut self, owner: &str, id: &str) {
        if self
            .entries
            .get(id)
            .is_some_and(|preview| preview.owner == owner)
        {
            self.entries.remove(id);
        }
    }
    pub fn revoke_owner(&mut self, owner: &str) {
        self.entries.retain(|_, preview| preview.owner != owner);
    }
}

/// Only validated UUID paths reach this template. There is no generated HTML,
/// script, event handler or postMessage bridge in the trusted outer document.
pub fn host_document(id: &str) -> Option<String> {
    let id = uuid::Uuid::parse_str(id).ok()?.to_string();
    Some(format!("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>html,body,iframe{{margin:0;width:100%;height:100%;border:0;display:block}}</style></head><body><iframe title=\"Generated content\" sandbox=\"allow-scripts\" referrerpolicy=\"no-referrer\" src=\"/preview/{id}/document\"></iframe></body></html>"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_lifecycle_is_scoped_bounded_and_releases_capacity() {
        let mut store = HtmlPreviewStore::default();
        assert!(store.create("one", "a".repeat(MAX_HTML_BYTES + 1)).is_err());
        let id = store.create("one", "<h1>Hello</h1>".into()).unwrap();
        store.release("other", &id);
        assert!(store.read(&id).is_some());
        store.release("one", &id);
        assert!(store.read(&id).is_none());
        for _ in 0..24 {
            store.create("one", String::new()).unwrap();
        }
        assert!(store.create("two", String::new()).is_err());
        store.revoke_owner("one");
        assert!(store.create("two", String::new()).is_ok());
    }
    #[test]
    fn trusted_outer_frame_contains_only_fixed_markup_and_local_document_path() {
        assert!(host_document("\"><script>attack()</script>").is_none());
        let host = host_document("00000000-0000-0000-0000-000000000000").unwrap();
        assert!(!host.contains("allow-same-origin"));
        assert!(!host.contains("<script"));
        assert!(HOST_CSP.contains("frame-src 'self'"));
        assert!(DOCUMENT_CSP.contains("connect-src 'none'"));
        assert!(!DOCUMENT_CSP.contains("https:"));
    }
}
