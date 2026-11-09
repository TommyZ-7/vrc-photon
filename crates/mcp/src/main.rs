use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use rmcp::ServiceExt;
use rmcp::transport::stdio;
use tracing_subscriber::EnvFilter;
use vrc_photon_ipc::Endpoint;
use vrc_photon_mcp::{DaemonClient, PhotoNMcp};

#[derive(Debug, Parser)]
#[command(
    name = "vrc-photon-mcp",
    version,
    about = "Read-only MCP server for the VRC PhotoN library"
)]
struct Args {
    /// Override the VRC PhotoN data directory used to find the daemon socket.
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let data_dir = args
        .data_dir
        .or_else(vrc_photon_core::data_dir)
        .context("cannot resolve the VRC PhotoN data directory")?;
    let endpoint = Endpoint::for_data_dir(&data_dir)
        .with_context(|| format!("cannot resolve the daemon endpoint in {}", data_dir.display()))?;
    let server = PhotoNMcp::new(DaemonClient::new(endpoint, Duration::from_secs(10)));

    let service = server
        .serve(stdio())
        .await
        .context("cannot start the MCP stdio server")?;
    service.waiting().await.context("MCP server stopped")?;
    Ok(())
}
