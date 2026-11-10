//! Reusable read-only database handle for UI background queries.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use vrc_photon_core::db::{Db, DbProfile};
use vrc_photon_core::error::Result;

/// Keeps one read-only connection open for background queries so the UI does
/// not open and close a file handle for every search or suggestion keystroke.
pub(crate) struct BackgroundReader {
    path: PathBuf,
    db: Mutex<Option<Db>>,
    #[cfg(test)]
    successful_opens: AtomicUsize,
}

impl BackgroundReader {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            db: Mutex::new(None),
            #[cfg(test)]
            successful_opens: AtomicUsize::new(0),
        }
    }

    /// Returns `None` for in-memory demo databases, which keep the
    /// synchronous UI-thread path.
    pub(crate) fn for_database(db: &Db) -> Option<Arc<Self>> {
        db.path()
            .map(|path| Arc::new(Self::new(path.to_path_buf())))
    }

    /// Runs `query` on the cached read-only connection, opening it on first
    /// use. A failed open leaves the slot empty so the next call retries.
    pub(crate) fn query<T>(&self, query: impl FnOnce(&Db) -> Result<T>) -> Result<T> {
        let mut slot = self
            .db
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            let db = Db::open_read_only_with_profile(&self.path, DbProfile::Ui)?;
            #[cfg(test)]
            self.successful_opens.fetch_add(1, Ordering::Relaxed);
            *slot = Some(db);
        }
        query(slot.as_ref().expect("read connection is initialized"))
    }

    #[cfg(test)]
    pub(crate) fn successful_open_count(&self) -> usize {
        self.successful_opens.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::BackgroundReader;
    use std::sync::Arc;
    use vrc_photon_core::db::Db;

    fn database_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join("library.db");
        let _db = Db::open(&path).unwrap();
        path
    }

    #[test]
    fn repeated_queries_reuse_one_connection() {
        let dir = tempfile::tempdir().unwrap();
        let reader = BackgroundReader::new(database_file(&dir));

        for _ in 0..3 {
            assert_eq!(reader.query(Db::count_photos).unwrap(), 0);
        }

        assert_eq!(reader.successful_open_count(), 1);
    }

    #[test]
    fn concurrent_queries_reuse_one_connection() {
        let dir = tempfile::tempdir().unwrap();
        let reader = Arc::new(BackgroundReader::new(database_file(&dir)));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let reader = Arc::clone(&reader);
                std::thread::spawn(move || {
                    for _ in 0..8 {
                        assert_eq!(reader.query(Db::count_photos).unwrap(), 0);
                    }
                })
            })
            .collect();

        for thread in threads {
            thread.join().unwrap();
        }

        assert_eq!(reader.successful_open_count(), 1);
    }

    #[test]
    fn cached_connection_sees_committed_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = database_file(&dir);
        let writer = Db::open(&path).unwrap();
        let reader = BackgroundReader::new(path);

        assert_eq!(reader.query(Db::library_revision).unwrap(), 0);
        writer.bump_library_revision().unwrap();

        assert_eq!(reader.query(Db::library_revision).unwrap(), 1);
        assert_eq!(reader.successful_open_count(), 1);
    }

    #[test]
    fn failed_open_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let reader = BackgroundReader::new(path);

        assert!(reader.query(Db::count_photos).is_err());
        assert_eq!(reader.successful_open_count(), 0);

        database_file(&dir);
        assert_eq!(reader.query(Db::count_photos).unwrap(), 0);
        assert_eq!(reader.successful_open_count(), 1);
    }

    #[test]
    fn in_memory_database_has_no_background_reader() {
        let db = Db::open_in_memory().unwrap();

        assert!(BackgroundReader::for_database(&db).is_none());
    }
}
