//! Read-only library payloads shared by the daemon and MCP clients.
//!
//! These types intentionally carry paths and metadata only. Image bytes are
//! read by the MCP process after it receives an effective path, so the IPC
//! message stays well under the 1 MiB framing limit.

use serde::{Deserialize, Serialize};

/// A player linked to a photo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSummary {
    pub user_id: String,
    pub display_name: String,
}

/// One row in an MCP photo search result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoSummary {
    pub id: i64,
    /// File the library displays: the display derivative when one exists,
    /// otherwise the original.
    pub path: String,
    pub captured_at_utc: i64,
    pub captured_at_local: String,
    pub world_name: Option<String>,
    pub world_id: Option<String>,
    pub players: Vec<PlayerSummary>,
}

/// One AI tag attached to a photo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiTagSummary {
    pub tag_key: String,
    pub name: String,
    pub name_en: Option<String>,
    pub probability: f32,
    pub applied: bool,
}

/// Full metadata for one photo, without image bytes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhotoDetail {
    pub id: i64,
    /// File the library displays: the display derivative when one exists,
    /// otherwise the original.
    pub path: String,
    /// Original file path, even when the library displays a derivative.
    pub original_path: String,
    pub file_name: String,
    pub captured_at_utc: i64,
    pub captured_at_local: String,
    pub width: u32,
    pub height: u32,
    /// Size of the displayed file.
    pub file_size: u64,
    pub world_name: Option<String>,
    pub world_id: Option<String>,
    pub instance_id: Option<String>,
    pub players: Vec<PlayerSummary>,
    pub tags: Vec<String>,
    pub ai_tags: Vec<AiTagSummary>,
    pub people_min: Option<u32>,
    pub people_max: Option<u32>,
    pub people_median: Option<u32>,
    pub is_favorite: bool,
    pub note: Option<String>,
    pub status: String,
}

/// One world represented in the library.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldSummary {
    /// External VRChat world ID, when the photo was linked to a stored world.
    pub world_id: Option<String>,
    pub name: String,
    pub photo_count: i64,
}

/// Library-wide counts exposed to MCP.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryStats {
    pub photo_count: i64,
    pub total_bytes: u64,
    pub world_count: i64,
    pub compressed_count: i64,
    pub derivative_bytes: u64,
}

/// One page of search results.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoSearch {
    pub photos: Vec<PhotoSummary>,
    pub total: i64,
    pub limit: u32,
    pub offset: u32,
}

/// A random sample of photos matching a filter. `total` is the number of
/// matches before sampling.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomPhotoSearch {
    pub photos: Vec<PhotoSummary>,
    pub total: i64,
    pub limit: u32,
}
