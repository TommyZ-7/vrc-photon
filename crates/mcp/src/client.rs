//! Blocking IPC client used by the MCP tools.
//!
//! Every request opens a short-lived connection through the shared
//! `vrc-photon-ipc` client, so the MCP process never touches SQLite.

use std::io;
use std::time::Duration;

use vrc_photon_ipc::client;
use vrc_photon_ipc::{
    Endpoint, IpcError, LibraryStats, PhotoDetail, PhotoSearch, RandomPhotoSearch, Request,
    Response, WorldSummary,
};

/// A daemon-side failure surfaced to an MCP tool.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("VRC PhotoN daemon is not running")]
    Unavailable,
    #[error("{0}")]
    Message(String),
}

impl DaemonError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable => "daemon_unavailable",
            Self::Message(message) if message.contains("photo not found") => "photo_not_found",
            Self::Message(_) => "daemon_error",
        }
    }
}

/// Parameters for one read-only photo search.
#[derive(Debug, Clone, Default)]
pub struct SearchPhotosQuery {
    pub world: Option<String>,
    pub player: Option<String>,
    pub tag: Option<String>,
    pub ai_tags: Vec<String>,
    pub from_utc: Option<i64>,
    pub to_utc: Option<i64>,
    pub favorite: bool,
    pub limit: u32,
    pub offset: u32,
}

/// Parameters for one random photo search.
#[derive(Debug, Clone, Default)]
pub struct RandomSearchPhotosQuery {
    pub world: Option<String>,
    pub player: Option<String>,
    pub tag: Option<String>,
    pub ai_tags: Vec<String>,
    pub from_utc: Option<i64>,
    pub to_utc: Option<i64>,
    pub favorite: bool,
    pub limit: u32,
    pub seed: Option<u64>,
}

/// Thin client over the daemon's versioned local socket.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    endpoint: Endpoint,
    timeout: Duration,
}

impl DaemonClient {
    pub fn new(endpoint: Endpoint, timeout: Duration) -> Self {
        Self { endpoint, timeout }
    }

    pub fn search_photos(&self, query: SearchPhotosQuery) -> Result<PhotoSearch, DaemonError> {
        match self.request(Request::SearchPhotos {
            world: query.world,
            player: query.player,
            tag: query.tag,
            ai_tags: query.ai_tags,
            from_utc: query.from_utc,
            to_utc: query.to_utc,
            favorite: query.favorite,
            limit: query.limit,
            offset: query.offset,
        })? {
            Response::Photos { result } => Ok(result),
            other => Err(unexpected(&other)),
        }
    }

    pub fn random_search_photos(
        &self,
        query: RandomSearchPhotosQuery,
    ) -> Result<RandomPhotoSearch, DaemonError> {
        match self.request(Request::RandomSearchPhotos {
            world: query.world,
            player: query.player,
            tag: query.tag,
            ai_tags: query.ai_tags,
            from_utc: query.from_utc,
            to_utc: query.to_utc,
            favorite: query.favorite,
            limit: query.limit,
            seed: query.seed,
        })? {
            Response::RandomPhotos { result } => Ok(result),
            other => Err(unexpected(&other)),
        }
    }

    pub fn get_photo(&self, id: i64) -> Result<PhotoDetail, DaemonError> {
        match self.request(Request::GetPhoto { id })? {
            Response::Photo { photo } => Ok(*photo),
            other => Err(unexpected(&other)),
        }
    }

    pub fn list_worlds(&self) -> Result<Vec<WorldSummary>, DaemonError> {
        match self.request(Request::ListWorlds)? {
            Response::Worlds { worlds } => Ok(worlds),
            other => Err(unexpected(&other)),
        }
    }

    pub fn library_stats(&self) -> Result<LibraryStats, DaemonError> {
        match self.request(Request::LibraryStats)? {
            Response::LibraryStats { stats } => Ok(stats),
            other => Err(unexpected(&other)),
        }
    }

    fn request(&self, request: Request) -> Result<Response, DaemonError> {
        match client::request(&self.endpoint, request, self.timeout) {
            Ok(response) => Ok(response),
            Err(IpcError::Io(error)) if daemon_is_unreachable(&error) => {
                Err(DaemonError::Unavailable)
            }
            Err(error) => Err(DaemonError::Message(error.to_string())),
        }
    }
}

fn daemon_is_unreachable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::AddrNotAvailable
    )
}

fn unexpected(response: &Response) -> DaemonError {
    DaemonError::Message(format!("unexpected daemon response: {response:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_endpoint_reports_the_daemon_as_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::for_tests(dir.path());
        let client = DaemonClient::new(endpoint, Duration::from_secs(1));
        let error = client.list_worlds().unwrap_err();
        assert!(matches!(error, DaemonError::Unavailable), "{error:?}");
        assert_eq!(error.to_string(), "VRC PhotoN daemon is not running");
        assert_eq!(error.code(), "daemon_unavailable");
    }
}
