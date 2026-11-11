//! Poison-aware lock helpers for the daemon.
//!
//! `std::sync::Mutex` poisons itself when a thread panics while holding the
//! guard. The daemon supervises and restarts its workers, so a poison left
//! behind would make every restarted worker panic at the same lock forever.
//! These helpers recover the inner value and leave a log line so the failure
//! stays visible.

use std::sync::{Mutex, MutexGuard};

use vrc_photon_core::db::Db;

use crate::DaemonStatus;

/// Locks the daemon's library database, recovering a poisoned mutex.
pub(crate) fn lock_db(mutex: &Mutex<Db>) -> MutexGuard<'_, Db> {
    lock_unpoisoned(mutex, "daemon.db")
}

/// Locks the daemon status, recovering a poisoned mutex.
pub(crate) fn lock_status(mutex: &Mutex<DaemonStatus>) -> MutexGuard<'_, DaemonStatus> {
    lock_unpoisoned(mutex, "daemon.status")
}

/// Locks any daemon mutex, recovering a poisoned mutex and logging it.
pub(crate) fn lock_unpoisoned<'a, T>(
    mutex: &'a Mutex<T>,
    label: &'static str,
) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::error!(
                mutex = label,
                "recovered poisoned mutex; the previous holder panicked"
            );
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;
    use std::sync::Arc;

    use super::*;

    #[test]
    fn poisoned_db_lock_recovers_and_keeps_workers_running() {
        let db = Arc::new(Mutex::new(Db::open_in_memory().unwrap()));
        let poisoned = Arc::clone(&db);
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = poisoned.lock().unwrap();
            panic!("worker panic while holding the db lock");
        }));
        assert!(db.is_poisoned());

        let guard = lock_db(&db);
        assert!(guard.count_photos().is_ok());
    }

    #[test]
    fn poisoned_status_lock_recovers() {
        let status = Arc::new(Mutex::new(DaemonStatus::default()));
        let poisoned = Arc::clone(&status);
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = poisoned.lock().unwrap();
            panic!("worker panic while holding the status lock");
        }));
        assert!(status.is_poisoned());

        let mut guard = lock_status(&status);
        guard.worker_restarts += 1;
        assert_eq!(guard.worker_restarts, 1);
    }
}
