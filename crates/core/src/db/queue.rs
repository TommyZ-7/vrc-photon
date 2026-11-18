use super::*;
use crate::queue::{QueueCounts, QueueEntry, QueueFeature, QueueProgress};

impl Db {
    /// Active rows for one feature, oldest first.
    pub fn active_queue_entries(
        &self,
        feature: QueueFeature,
        limit: i64,
    ) -> Result<Vec<QueueEntry>> {
        match feature {
            QueueFeature::Compression => self.active_compression_queue_entries(limit),
            QueueFeature::Discord => self.active_send_queue_entries(limit),
            QueueFeature::Tagging => self.active_tagging_queue_entries(AI_PLUGIN_ID, limit),
        }
    }

    /// The dashboard's mixed queue: all three features, oldest first.
    pub fn mixed_active_queue_entries(&self, limit: i64) -> Result<Vec<QueueEntry>> {
        let limit = clamp_limit(limit);
        let mut entries = Vec::new();
        entries.extend(self.active_compression_queue_entries(limit)?);
        entries.extend(self.active_send_queue_entries(limit)?);
        entries.extend(self.active_tagging_queue_entries(AI_PLUGIN_ID, limit)?);
        entries.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.job_id.cmp(&b.job_id))
        });
        entries.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(entries)
    }

    pub fn queue_counts(&self, feature: QueueFeature) -> Result<QueueCounts> {
        match feature {
            QueueFeature::Compression => self.compression_queue_counts(),
            QueueFeature::Discord => self.send_queue_counts(),
            QueueFeature::Tagging => self.tagging_queue_counts(AI_PLUGIN_ID),
        }
    }

    /// Completion progress for the feature's current queue run.
    pub fn queue_progress(&self, feature: QueueFeature) -> Result<QueueProgress> {
        let completed = self
            .conn
            .query_row(
                "SELECT completed FROM queue_progress WHERE feature = ?1",
                params![feature.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok(QueueProgress { completed })
    }

    /// Deletes one waiting row. Pending and processing rows are never removed.
    pub fn delete_waiting_entry(&self, feature: QueueFeature, job_id: &str) -> Result<bool> {
        match feature {
            QueueFeature::Compression => self.delete_waiting_compression_job(job_id),
            QueueFeature::Discord => match job_id.parse::<i64>() {
                Ok(id) => self.delete_waiting_send(id),
                Err(_) => Ok(false),
            },
            QueueFeature::Tagging => self.delete_waiting_tagging_job(job_id, AI_PLUGIN_ID),
        }
    }

    /// Deletes every waiting row for one feature. Pending and processing rows
    /// are left untouched.
    pub fn delete_all_waiting_entries(&self, feature: QueueFeature) -> Result<usize> {
        match feature {
            QueueFeature::Compression => self.delete_all_waiting_compression_jobs(),
            QueueFeature::Discord => self.delete_all_waiting_sends(),
            QueueFeature::Tagging => self.delete_all_waiting_tagging_jobs(AI_PLUGIN_ID),
        }
    }
}
