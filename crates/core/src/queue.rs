//! Unified view of the three job queues.
//!
//! The compression, Discord, and tagging queues keep their own tables and
//! states. This module only provides the shared shape the UI renders.

use vrc_photon_ai_protocol::TaggingJobStatus;

use crate::model::{QueueStatus, SendStatus};

/// One of the three queues shown in 連携・自動 and the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueFeature {
    Compression,
    Discord,
    Tagging,
}

impl QueueFeature {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compression => "compression",
            Self::Discord => "discord",
            Self::Tagging => "tagging",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "compression" => Some(Self::Compression),
            "discord" => Some(Self::Discord),
            "tagging" => Some(Self::Tagging),
            _ => None,
        }
    }
}

/// The three active states shared by every queue. Terminal rows are never
/// included in a queue entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueState {
    Waiting,
    Pending,
    Processing,
}

impl QueueState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Pending => "pending",
            Self::Processing => "processing",
        }
    }

    pub fn from_queue_status(status: QueueStatus) -> Option<Self> {
        match status {
            QueueStatus::Waiting => Some(Self::Waiting),
            QueueStatus::Pending => Some(Self::Pending),
            QueueStatus::Processing => Some(Self::Processing),
            _ => None,
        }
    }

    pub fn from_send_status(status: SendStatus) -> Option<Self> {
        match status {
            SendStatus::Waiting => Some(Self::Waiting),
            SendStatus::Pending => Some(Self::Pending),
            SendStatus::Processing => Some(Self::Processing),
            _ => None,
        }
    }

    pub fn from_tagging_status(status: TaggingJobStatus) -> Option<Self> {
        match status {
            TaggingJobStatus::Waiting => Some(Self::Waiting),
            TaggingJobStatus::Pending => Some(Self::Pending),
            TaggingJobStatus::Processing => Some(Self::Processing),
            _ => None,
        }
    }
}

/// One active row in a feature queue or the dashboard's mixed queue.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueEntry {
    pub feature: QueueFeature,
    pub job_id: String,
    pub photo_id: Option<i64>,
    pub photo_name: Option<String>,
    pub state: QueueState,
    pub progress: f32,
    /// Feature-specific context: `auto`/profile name for compression,
    /// destination or `test` for Discord, and the stage for tagging.
    pub detail: String,
    pub created_at: String,
}

/// Active row counts for one feature. Terminal rows are not counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueCounts {
    pub waiting: i64,
    pub pending: i64,
    pub processing: i64,
}

impl QueueCounts {
    pub fn total(self) -> i64 {
        self.waiting + self.pending + self.processing
    }

    pub fn add(&mut self, state: QueueState) {
        match state {
            QueueState::Waiting => self.waiting += 1,
            QueueState::Pending => self.pending += 1,
            QueueState::Processing => self.processing += 1,
        }
    }
}

/// Completion progress for the current run of one feature queue.
///
/// The database also stores the run start time; the UI contract only needs
/// the number of successful completions since that start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueProgress {
    /// Completed jobs in the current run. Failed and cancelled jobs are not
    /// counted.
    pub completed: i64,
}

/// One failed row shown in the notification modal. Failed rows are not part
/// of the active queue.
#[derive(Debug, Clone, PartialEq)]
pub struct FailedJobEntry {
    pub feature: QueueFeature,
    pub job_id: String,
    pub photo_id: Option<i64>,
    pub photo_name: Option<String>,
    pub error: String,
    pub created_at: String,
}
