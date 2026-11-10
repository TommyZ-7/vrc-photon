//! Versioned local-socket protocol shared by the UI and the daemon.

pub mod client;
pub mod endpoint;
pub mod framing;
pub mod library;
pub mod protocol;
pub mod server;

pub use client::{IpcError, SubscriptionInfo};
pub use endpoint::Endpoint;
pub use library::*;
pub use protocol::*;

pub const PROTOCOL_VERSION: u32 = 8;
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::PROTOCOL_VERSION;

    #[test]
    fn protocol_version_is_eight() {
        assert_eq!(PROTOCOL_VERSION, 8);
    }
}
