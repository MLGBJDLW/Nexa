//! Compile the same store and native adapter without desktop transport dependencies.
#[path = "../../../../shared/rust/native_chat_ui_preview.rs"]
mod native;
#[path = "../../../../crates/remote/src/html_preview.rs"]
mod store;
pub use native::*;
