//! Versioned local-socket protocol shared by the UI and the daemon.

// #762 staged rollout: core casts are enforced first. This crate keeps
// the lint allowed with a reason so the exception is visible and greppable.
#![allow(
    clippy::as_conversions,
    reason = "Staged #762 rollout: vrc-photon-core casts are enforced first; this crate follows"
)]

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

pub const PROTOCOL_VERSION: u32 = 1;
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::PROTOCOL_VERSION;

    #[test]
    fn protocol_version_is_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }
}
