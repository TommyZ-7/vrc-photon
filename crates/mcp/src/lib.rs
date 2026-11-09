//! Read-only MCP server for the VRC PhotoN library.

pub mod client;
pub mod server;

pub use client::DaemonClient;
pub use server::PhotoNMcp;
