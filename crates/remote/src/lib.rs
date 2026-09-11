//! Authenticated browser access to an explicitly exposed host surface.
mod auth;
mod protocol;
pub mod public_access;
mod server;
pub mod tls;
pub mod tunnel;
pub use auth::{AuthStore, Device, PairedDevice, PairingCode};
pub use protocol::*;
pub use server::{RemoteServer, RunningListener};

pub fn pairing_qr(url: &str) -> Result<String, String> {
    qrcode::QrCode::new(url)
        .map(|code| {
            code.render::<qrcode::render::svg::Color>()
                .min_dimensions(280, 280)
                .build()
        })
        .map_err(|e| e.to_string())
}

#[async_trait::async_trait]
pub trait RemoteHost: Send + Sync + 'static {
    async fn execute(
        &self,
        owner: &str,
        command: RemoteCommand,
    ) -> Result<serde_json::Value, String>;
    async fn disconnected(&self, owner: &str);
    async fn connection_changed(&self, _owner: &str, _connected: bool) {}
    fn asset(&self, path: &str) -> Option<WebAsset>;
}
