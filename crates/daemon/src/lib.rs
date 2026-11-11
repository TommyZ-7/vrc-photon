use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use image::DynamicImage;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use vrc_photon_ai_protocol::TaggingJobStatus;
use vrc_photon_core::compressor;
use vrc_photon_core::db::Db;
use vrc_photon_core::hashing;
use vrc_photon_core::linker;
use vrc_photon_core::log_parser::LogIngestor;
use vrc_photon_core::metadata::{self, XmpMetadata};
use vrc_photon_core::model::{
    CompressionJob, CompressionProfile, DaemonCommand, DaemonOperation, DeletionReason,
    DeletionRequest, Derivative, Photo, PhotoStatus, QueueStatus,
};
use vrc_photon_core::photo_filename::is_source_photo;
use vrc_photon_core::scanner::{
    ScanFailure, ScanPhase as CoreScanPhase, ScanProgress, ScanReport, scan_and_index_shared,
    scan_one,
};
use vrc_photon_core::settings::{AppSettings, detect_vrchat_paths};
use vrc_photon_core::thumbnail;
use vrc_photon_ipc::{Endpoint, Event, JobStatus, Operation, ScanPhase as IpcScanPhase};

#[cfg(test)]
mod ai_e2e;
mod compression;
mod deletion;
mod discord;
mod discord_http;
mod hash;
mod ipc_server;
mod lifecycle;
mod lock;
mod logs;
mod memory_budget;
pub(crate) mod plugin;
mod scan;
mod signals;
mod status;
mod supervisor;
pub(crate) mod tagging;
#[cfg(test)]
mod test_support;
mod thumbnails;
mod tray;
mod vrchat_process;
mod world_name_http;
mod world_names;

use discord_http::UreqTransport;
use ipc_server::{EventBus, IpcControl, IpcServer, status_snapshot, tagging_status};
#[cfg(test)]
pub(crate) use lifecycle::LocalizedError;
pub use lifecycle::{Daemon, DaemonStatus};
use lock::{lock_db, lock_status};
use memory_budget::{COMPRESSION_MEMORY_BUDGET_BYTES, MemoryBudget, compression_peak_estimate};
use plugin::{AI_PLUGIN_ID, PluginService};
use signals::{JobSignal, WakeSignal};
use tagging::{TaggingControl, TaggingWorker, TaggingWorkerConfig};
use tray::{Tray, TrayAction};
use vrc_photon_ai_host::{ModelHttpClient, UreqModelClient};
use world_name_http::UreqWorldNameTransport;

const PARSER_VERSION: &str = "1";
/// Number of skipped-line sentences kept in one log error detail.
const MAX_SKIPPED_DETAILS: usize = 5;
const SCAN_INTERVAL: Duration = Duration::from_millis(500);
/// How often a paused worker rechecks its gate at a dequeue point. Resume also
/// notifies the worker signals, so this is only a fallback.
const PAUSED_WORKER_POLL: Duration = Duration::from_millis(500);
const LOG_INTERVAL: Duration = Duration::from_millis(750);
/// Complete log lines one poll may return. The next poll continues from the
/// last committed offset.
const LOG_READ_MAX_LINES: usize = 5_000;
/// Bytes of log data one poll may consume. A line that crosses this budget is
/// left partial for the next poll instead of growing memory without bound.
const LOG_READ_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Complete lines applied per database transaction. This bounds how long log
/// ingestion holds the shared database mutex.
const LOG_INGEST_CHUNK_LINES: usize = 500;
/// Consecutive log-poll misses required before a missing VRChat process closes
/// the active session. This tolerates one-off process-table read failures and
/// short restart gaps.
const VRCHAT_MISSING_POLLS_BEFORE_CLOSE: u8 = 3;
/// How often the daemon re-reads the compression settings and thumbnail
/// cache limit the UI saved. Runs on every loop iteration, paused or not,
/// so a change is applied within roughly one interval plus one loop wait.
const SETTINGS_INTERVAL: Duration = Duration::from_millis(500);
const THUMBNAIL_PRUNE_BATCH: usize = 128;
const THUMBNAIL_PRUNE_IDLE: Duration = Duration::from_millis(500);

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
