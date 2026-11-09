//! MCP tool surface for the read-only VRC PhotoN library API.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::json;
use crate::client::{DaemonClient, DaemonError, SearchPhotosQuery};

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

/// Read-only MCP server backed by the VRC PhotoN daemon.
#[derive(Debug, Clone)]
pub struct PhotoNMcp {
    client: DaemonClient,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchPhotosParams {
    /// World name, world ID, or instance ID substring.
    #[serde(default)]
    pub world: Option<String>,
    /// Player display name or user ID substring.
    #[serde(default)]
    pub player: Option<String>,
    /// Manual tag substring.
    #[serde(default)]
    pub tag: Option<String>,
    /// Applied AI tag keys, matched with OR.
    #[serde(default)]
    pub ai_tags: Vec<String>,
    /// Inclusive lower capture bound as an RFC 3339 timestamp.
    #[serde(default)]
    pub captured_from: Option<String>,
    /// Inclusive upper capture bound as an RFC 3339 timestamp.
    #[serde(default)]
    pub captured_to: Option<String>,
    /// Restrict the result to favorites when true.
    #[serde(default)]
    pub favorite: Option<bool>,
    /// Page size. Defaults to 50 and is clamped to 1..=200.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Page offset. Defaults to 0.
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetPhotoParams {
    /// Photo ID returned by search_photos.
    pub id: i64,
}

#[tool_router]
impl PhotoNMcp {
    pub fn new(client: DaemonClient) -> Self {
        Self {
            client,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Search the VRC PhotoN library by world, player, manual tag, AI tag, capture time, or favorite. Returns IDs, display paths, capture times, worlds, and players."
    )]
    async fn search_photos(
        &self,
        Parameters(params): Parameters<SearchPhotosParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let captured_from = parse_timestamp(params.captured_from.as_deref(), "captured_from")?;
        let captured_to = parse_timestamp(params.captured_to.as_deref(), "captured_to")?;
        let query = SearchPhotosQuery {
            world: params.world,
            player: params.player,
            tag: params.tag,
            ai_tags: params.ai_tags,
            from_utc: captured_from,
            to_utc: captured_to,
            favorite: params.favorite.unwrap_or(false),
            limit: params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
            offset: params.offset.unwrap_or(0),
        };
        let client = self.client.clone();
        match call_daemon(move || client.search_photos(query)).await {
            Ok(result) => structured(result),
            Err(error) => Ok(tool_error(error)),
        }
    }

    #[tool(
        description = "Get full metadata for one photo: resolution, file size, world, players, manual tags, AI tags, people interval, and favorite."
    )]
    async fn get_photo(
        &self,
        Parameters(params): Parameters<GetPhotoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let client = self.client.clone();
        match call_daemon(move || client.get_photo(params.id)).await {
            Ok(photo) => structured(photo),
            Err(error) => Ok(tool_error(error)),
        }
    }

    #[tool(description = "List the worlds represented in the library with their photo counts.")]
    async fn list_worlds(&self) -> Result<CallToolResult, ErrorData> {
        let client = self.client.clone();
        match call_daemon(move || client.list_worlds()).await {
            Ok(worlds) => structured(json!({ "worlds": worlds })),
            Err(error) => Ok(tool_error(error)),
        }
    }

    #[tool(
        description = "Return library totals: photo count, total bytes, world count, compressed count, and derivative bytes."
    )]
    async fn library_stats(&self) -> Result<CallToolResult, ErrorData> {
        let client = self.client.clone();
        match call_daemon(move || client.library_stats()).await {
            Ok(stats) => structured(stats),
            Err(error) => Ok(tool_error(error)),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PhotoNMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "vrc-photon-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Read-only access to the VRC PhotoN library. The VRC PhotoN daemon must be running.",
            )
    }
}

async fn call_daemon<T, F>(operation: F) -> Result<T, DaemonError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, DaemonError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| DaemonError::Message(format!("MCP worker failed: {error}")))?
}

fn structured(value: impl serde::Serialize) -> Result<CallToolResult, ErrorData> {
    let value = serde_json::to_value(value).map_err(|error| {
        ErrorData::internal_error(format!("cannot serialize tool result: {error}"), None)
    })?;
    Ok(CallToolResult::structured(value))
}

fn tool_error(error: DaemonError) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "error": error.code(),
        "message": error.to_string(),
    }))
}

fn parse_timestamp(value: Option<&str>, field: &str) -> Result<Option<i64>, ErrorData> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| Some(timestamp.timestamp()))
        .map_err(|error| {
            ErrorData::invalid_params(
                format!("{field} must be an RFC 3339 timestamp: {error}"),
                None,
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::BufReader;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use vrc_photon_ipc::server::Listener;
    use vrc_photon_ipc::{
        PROTOCOL_VERSION, PhotoDetail, PhotoSearch, PhotoSummary, RequestEnvelope, Response,
        ServerMessage, StatusSnapshot,
    };

    struct FakeDaemon {
        _dir: tempfile::TempDir,
        endpoint: vrc_photon_ipc::Endpoint,
        stop: Arc<AtomicBool>,
        requests: Arc<Mutex<Vec<vrc_photon_ipc::Request>>>,
        handle: Option<JoinHandle<()>>,
    }

    impl FakeDaemon {
        fn start(responses: Vec<Response>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let endpoint = vrc_photon_ipc::Endpoint::for_tests(dir.path());
            let listener = Listener::bind(endpoint.clone()).unwrap();
            listener.set_nonblocking_accept(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let thread_requests = requests.clone();
            let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
            let handle = thread::spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok(stream) => {
                            let mut reader = BufReader::new(stream);
                            let Some(hello) =
                                vrc_photon_ipc::framing::read_message::<_, RequestEnvelope>(&mut reader)
                                    .unwrap()
                            else {
                                continue;
                            };
                            vrc_photon_ipc::framing::write_message(
                                reader.get_mut(),
                                &ServerMessage::Response {
                                    id: hello.id,
                                    response: Response::Hello {
                                        protocol_version: PROTOCOL_VERSION,
                                        daemon_version: "test".to_string(),
                                        pid: 1,
                                        status: StatusSnapshot::default(),
                                    },
                                },
                            )
                            .unwrap();
                            let Some(envelope) =
                                vrc_photon_ipc::framing::read_message::<_, RequestEnvelope>(&mut reader)
                                    .unwrap()
                            else {
                                continue;
                            };
                            thread_requests.lock().unwrap().push(envelope.request);
                            let response = responses
                                .lock()
                                .unwrap()
                                .pop_front()
                                .expect("fake daemon response");
                            vrc_photon_ipc::framing::write_message(
                                reader.get_mut(),
                                &ServerMessage::Response {
                                    id: envelope.id,
                                    response,
                                },
                            )
                            .unwrap();
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                _dir: dir,
                endpoint,
                stop,
                requests,
                handle: Some(handle),
            }
        }

        fn client(&self) -> DaemonClient {
            DaemonClient::new(self.endpoint.clone(), Duration::from_secs(2))
        }

        fn requests(&self) -> Vec<vrc_photon_ipc::Request> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Drop for FakeDaemon {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn photo_search() -> PhotoSearch {
        PhotoSearch {
            photos: vec![PhotoSummary {
                id: 7,
                path: "/photos/a.png".to_string(),
                captured_at_utc: 100,
                captured_at_local: "2026-11-09 12:00:00".to_string(),
                world_name: Some("World".to_string()),
                world_id: Some("wrld_1".to_string()),
                players: Vec::new(),
            }],
            total: 1,
            limit: 50,
            offset: 0,
        }
    }

    fn tool_router_names(server: &PhotoNMcp) -> Vec<String> {
        server
            .tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    #[tokio::test]
    async fn search_photos_clamps_limits_and_forwards_filters() {
        let daemon = FakeDaemon::start(vec![Response::Photos {
            result: photo_search(),
        }]);
        let server = PhotoNMcp::new(daemon.client());

        let result = server
            .search_photos(Parameters(SearchPhotosParams {
                world: Some(" World ".to_string()),
                player: None,
                tag: Some("beach".to_string()),
                ai_tags: vec!["sea".to_string()],
                captured_from: Some("2026-11-09T00:00:00Z".to_string()),
                captured_to: Some("2026-11-09T23:59:59Z".to_string()),
                favorite: Some(true),
                limit: Some(9999),
                offset: Some(0),
            }))
            .await
            .unwrap();

        assert_eq!(result.is_error, Some(false));
        let structured = result.structured_content.unwrap();
        assert_eq!(structured["total"], 1);
        assert_eq!(structured["photos"][0]["id"], 7);

        let requests = daemon.requests();
        match &requests[0] {
            vrc_photon_ipc::Request::SearchPhotos {
                world,
                tag,
                ai_tags,
                from_utc,
                to_utc,
                favorite,
                limit,
                offset,
                ..
            } => {
                assert_eq!(world.as_deref(), Some(" World "));
                assert_eq!(tag.as_deref(), Some("beach"));
                assert_eq!(ai_tags, &vec!["sea".to_string()]);
                assert_eq!(from_utc, &Some(1_794_182_400));
                assert_eq!(to_utc, &Some(1_794_268_799));
                assert!(favorite);
                assert_eq!(*limit, 200);
                assert_eq!(*offset, 0);
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_photos_rejects_invalid_timestamps() {
        let daemon = FakeDaemon::start(Vec::new());
        let server = PhotoNMcp::new(daemon.client());
        let error = server
            .search_photos(Parameters(SearchPhotosParams {
                world: None,
                player: None,
                tag: None,
                ai_tags: Vec::new(),
                captured_from: Some("yesterday".to_string()),
                captured_to: None,
                favorite: None,
                limit: None,
                offset: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn get_photo_returns_metadata_and_maps_not_found_to_a_tool_error() {
        let daemon = FakeDaemon::start(vec![Response::Photo {
            photo: PhotoDetail {
                id: 7,
                path: "/photos/a.png".to_string(),
                original_path: "/photos/a.png".to_string(),
                file_name: "a.png".to_string(),
                width: 1920,
                height: 1080,
                file_size: 10,
                tags: vec!["beach".to_string()],
                is_favorite: true,
                status: "present".to_string(),
                ..PhotoDetail::default()
            },
        }]);
        let server = PhotoNMcp::new(daemon.client());

        let result = server
            .get_photo(Parameters(GetPhotoParams { id: 7 }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.structured_content.unwrap()["file_name"], "a.png");

        let missing = FakeDaemon::start(vec![Response::Error {
            code: vrc_photon_ipc::ErrorCode::NotFound,
            message: "photo not found: 8".to_string(),
        }]);
        let missing_server = PhotoNMcp::new(missing.client());
        let result = missing_server
            .get_photo(Parameters(GetPhotoParams { id: 8 }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(
            result.structured_content.unwrap()["message"]
                .as_str()
                .unwrap()
                .contains("photo not found")
        );
    }

    #[tokio::test]
    async fn list_worlds_and_library_stats_round_trip() {
        let daemon = FakeDaemon::start(vec![
            Response::Worlds {
                worlds: vec![vrc_photon_ipc::WorldSummary {
                    world_id: Some("wrld_1".to_string()),
                    name: "World".to_string(),
                    photo_count: 3,
                }],
            },
            Response::LibraryStats {
                stats: vrc_photon_ipc::LibraryStats {
                    photo_count: 10,
                    total_bytes: 1000,
                    world_count: 2,
                    compressed_count: 4,
                    derivative_bytes: 400,
                },
            },
        ]);
        let server = PhotoNMcp::new(daemon.client());

        let worlds = server.list_worlds().await.unwrap();
        assert_eq!(
            worlds.structured_content.unwrap()["worlds"][0]["photo_count"],
            3
        );

        let stats = server.library_stats().await.unwrap();
        assert_eq!(stats.structured_content.unwrap()["world_count"], 2);
    }

    #[tokio::test]
    async fn daemon_unavailable_is_a_visible_tool_error() {
        let dir = tempfile::tempdir().unwrap();
        let server = PhotoNMcp::new(DaemonClient::new(
            vrc_photon_ipc::Endpoint::for_tests(dir.path()),
            Duration::from_secs(1),
        ));
        let result = server.list_worlds().await.unwrap();
        assert_eq!(result.is_error, Some(true));
        let error = result.structured_content.unwrap();
        assert_eq!(error["error"], "daemon_unavailable");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("daemon is not running")
        );
    }

    #[tokio::test]
    async fn tool_router_lists_the_four_read_tools() {
        let daemon = FakeDaemon::start(Vec::new());
        let server = PhotoNMcp::new(daemon.client());
        let names = tool_router_names(&server);
        for expected in ["search_photos", "get_photo", "list_worlds", "library_stats"] {
            assert!(names.contains(&expected.to_string()), "{names:?}");
        }
    }
}
