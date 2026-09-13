//! User-started, conversation-scoped screen sharing. Only the latest bounded
//! frame is held in memory; screenshots are never written to conversation data.
use crate::tools::ToolOutputAttachment;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const MAX_BASE64: usize = 1_400_000;
const FRAME_TTL: Duration = Duration::from_secs(8);
const LEASE_TTL: Duration = Duration::from_secs(30);

struct Share {
    lease: String,
    updated: Instant,
    sequence: u64,
    source: String,
    frame_id: String,
    frame: Option<ToolOutputAttachment>,
}
impl Share {
    fn context_name(&self) -> String {
        format!(
            "nexa_screen_{}_{}",
            self.lease.replace('-', ""),
            self.frame_id
        )
    }
}
#[derive(Default)]
pub struct SharedDesktopStore {
    shares: Mutex<HashMap<String, Share>>,
}
pub fn store() -> &'static SharedDesktopStore {
    static STORE: OnceLock<SharedDesktopStore> = OnceLock::new();
    STORE.get_or_init(SharedDesktopStore::default)
}
impl SharedDesktopStore {
    pub fn begin(&self, conversation: &str, source: &str) -> Result<String, String> {
        if conversation.is_empty() || conversation.len() > 200 {
            return Err("A conversation is required for screen sharing".into());
        }
        let mut shares = self
            .shares
            .lock()
            .map_err(|_| "Screen sharing is unavailable")?;
        shares.retain(|_, share| share.updated.elapsed() < LEASE_TTL);
        if shares.len() >= 8 && !shares.contains_key(conversation) {
            return Err("Too many shared screens".into());
        }
        let lease = uuid::Uuid::new_v4().to_string();
        shares.insert(
            conversation.into(),
            Share {
                lease: lease.clone(),
                updated: Instant::now(),
                sequence: 0,
                source: source.chars().take(200).collect(),
                frame_id: String::new(),
                frame: None,
            },
        );
        Ok(lease)
    }
    pub fn update(
        &self,
        conversation: &str,
        lease: &str,
        sequence: u64,
        base64: String,
    ) -> Result<(), String> {
        if base64.is_empty() || base64.len() > MAX_BASE64 {
            return Err("Shared screen frame exceeds its size limit".into());
        }
        let bytes = STANDARD
            .decode(&base64)
            .map_err(|_| "Invalid shared screen encoding")?;
        let reader =
            image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Jpeg);
        let (width, height) = reader
            .into_dimensions()
            .map_err(|_| "Invalid shared screen JPEG")?;
        if width == 0
            || height == 0
            || width > crate::media::MAX_LLM_IMAGE_DIMENSION
            || height > crate::media::MAX_LLM_IMAGE_DIMENSION
        {
            return Err("Shared screen dimensions exceed their limit".into());
        }
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
            .map_err(|_| "Invalid shared screen JPEG data")?;
        let mut shares = self
            .shares
            .lock()
            .map_err(|_| "Screen sharing is unavailable")?;
        let share = shares
            .get_mut(conversation)
            .filter(|share| share.lease == lease && share.updated.elapsed() < LEASE_TTL)
            .ok_or("Screen sharing has ended; start it again")?;
        if sequence <= share.sequence {
            return Ok(());
        }
        share.sequence = sequence;
        share.updated = Instant::now();
        share.frame_id = blake3::hash(&bytes).to_hex()[..16].to_string();
        share.frame = Some(ToolOutputAttachment {
            name: "shared-screen.jpg".into(),
            mime_type: "image/jpeg".into(),
            data: serde_json::json!({"base64": base64}),
        });
        Ok(())
    }
    pub fn end(&self, conversation: &str, lease: &str) {
        if let Ok(mut shares) = self.shares.lock() {
            if shares
                .get(conversation)
                .is_some_and(|share| share.lease == lease)
            {
                shares.remove(conversation);
            }
        }
    }
    pub fn latest(&self, conversation: &str) -> Option<(String, ToolOutputAttachment)> {
        self.latest_context(conversation)
            .map(|(source, frame, _)| (source, frame))
    }
    /// Return pixels and their identity under the same lock, including during uploads.
    pub(crate) fn latest_context(
        &self,
        conversation: &str,
    ) -> Option<(String, ToolOutputAttachment, String)> {
        let mut shares = self.shares.lock().ok()?;
        shares.retain(|_, share| share.updated.elapsed() < LEASE_TTL);
        let share = shares.get(conversation)?;
        if share.updated.elapsed() > FRAME_TTL {
            return None;
        }
        Some((
            share.source.clone(),
            share.frame.clone()?,
            share.context_name(),
        ))
    }
    pub fn context_name(&self, conversation: &str) -> Option<String> {
        let shares = self.shares.lock().ok()?;
        let share = shares.get(conversation)?;
        share.frame.as_ref()?;
        Some(share.context_name())
    }
    /// Re-check revocation at each physical provider invocation, including retries.
    pub fn remove_revoked_context(&self, messages: &mut Vec<crate::llm::Message>) {
        let Ok(shares) = self.shares.lock() else {
            messages.retain(|message| {
                !message
                    .name
                    .as_deref()
                    .is_some_and(|name| name.starts_with("nexa_screen_"))
            });
            return;
        };
        messages.retain(|message| {
            let Some(name) = message
                .name
                .as_deref()
                .filter(|name| name.starts_with("nexa_screen_"))
            else {
                return true;
            };
            shares
                .values()
                .any(|share| share.updated.elapsed() < FRAME_TTL && name == share.context_name())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(color: u8) -> String {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([color, 0, 0]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .unwrap();
        STANDARD.encode(bytes.into_inner())
    }
    #[test]
    fn sharing_is_scoped_ordered_revocable_and_expires() {
        let store = SharedDesktopStore::default();
        let lease = store.begin("a", "Screen 1").unwrap();
        assert!(store.latest("a").is_none());
        store.update("a", &lease, 2, frame(220)).unwrap();
        store.update("a", &lease, 1, frame(0)).unwrap();
        assert_eq!(store.latest("a").unwrap().1.data["base64"], frame(220));
        assert!(store.latest("b").is_none());
        assert!(store.update("b", &lease, 3, frame(0)).is_err());
        let replacement = store.begin("a", "Screen 2").unwrap();
        store.end("a", &lease);
        assert!(store.update("a", &lease, 3, frame(0)).is_err());
        store.update("a", &replacement, 1, frame(0)).unwrap();
        store.shares.lock().unwrap().get_mut("a").unwrap().updated =
            Instant::now() - FRAME_TTL - Duration::from_millis(1);
        assert!(store.latest("a").is_none());
        store.end("a", &replacement);
        assert!(store.latest("a").is_none());
        assert!(store.update("a", &replacement, 2, frame(0)).is_err());
    }

    #[test]
    fn retry_context_tracks_pixels_not_only_the_active_lease() {
        let store = SharedDesktopStore::default();
        let lease = store.begin("a", "Screen").unwrap();
        store.update("a", &lease, 1, frame(10)).unwrap();
        let mut old = crate::llm::Message::text(crate::llm::Role::User, "old screen");
        old.name = store.context_name("a");
        assert!(old.name.as_ref().unwrap().len() <= 64);
        store.update("a", &lease, 2, frame(10)).unwrap();
        let mut unchanged = vec![old.clone()];
        store.remove_revoked_context(&mut unchanged);
        assert_eq!(unchanged.len(), 1, "unchanged pixels remain valid");
        store.update("a", &lease, 3, frame(220)).unwrap();
        let mut outdated = vec![old];
        store.remove_revoked_context(&mut outdated);
        assert!(
            outdated.is_empty(),
            "a live lease must not retain superseded pixels"
        );
        let mut latest = crate::llm::Message::text(crate::llm::Role::User, "new screen");
        latest.name = store.context_name("a");
        let mut current = vec![latest];
        store.remove_revoked_context(&mut current);
        assert_eq!(current.len(), 1);
        store.end("a", &lease);
        store.remove_revoked_context(&mut current);
        assert!(current.is_empty());
    }
}
