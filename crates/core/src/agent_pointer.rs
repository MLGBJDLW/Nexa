//! Shared accent and reduced-motion preference for desktop interaction feedback.
use std::sync::atomic::{AtomicU32, Ordering};

// Neutral only until the main renderer projects the current CSS theme.
static APPEARANCE: AtomicU32 = AtomicU32::new(0x0080_8080);

pub fn configure(accent: [u8; 3], reduced_motion: bool) {
    APPEARANCE.store(
        u32::from_be_bytes([u8::from(reduced_motion), accent[0], accent[1], accent[2]]),
        Ordering::Release,
    );
}

pub fn accent() -> [u8; 3] {
    let bytes = APPEARANCE.load(Ordering::Acquire).to_be_bytes();
    [bytes[1], bytes[2], bytes[3]]
}

pub fn reduced_motion() -> bool {
    APPEARANCE.load(Ordering::Acquire) >> 24 != 0
}

pub fn browser_appearance() -> serde_json::Value {
    let [red, green, blue] = accent();
    serde_json::json!({ "accent": format!("rgb({red} {green} {blue})"), "reducedMotion": reduced_motion() })
}
