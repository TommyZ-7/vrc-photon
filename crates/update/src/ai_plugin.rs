//! Cache of the AI plugin release advertised by the last successful feed check.

use crate::error::{Result, UpdateError};
use crate::helper::write_atomic;
use crate::install::UpdatePaths;
use crate::manifest::AiPluginRelease;

/// Reads the cached release. Missing, unreadable, or malformed cache files are
/// ignored so a stale local file can never block the UI.
pub fn load(paths: &UpdatePaths) -> Option<AiPluginRelease> {
    let bytes = std::fs::read(paths.ai_plugin_file()).ok()?;
    let release: AiPluginRelease = serde_json::from_slice(&bytes).ok()?;
    release.versions()?;
    Some(release)
}

/// Replaces the cache with the release from the latest check, or removes it
/// when the feed no longer advertises a plugin.
pub fn store(paths: &UpdatePaths, release: Option<&AiPluginRelease>) -> Result<()> {
    let path = paths.ai_plugin_file();
    match release {
        Some(release) => {
            let bytes = serde_json::to_vec_pretty(release).map_err(|error| {
                UpdateError::Other(format!("cannot serialize ai_plugin: {error}"))
            })?;
            write_atomic(&path, &bytes, 0o600)
        }
        None => {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(UpdateError::io(&path, error)),
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::UpdatePaths;

    fn release(version: &str, host_min: &str) -> AiPluginRelease {
        AiPluginRelease {
            version: version.to_string(),
            host_version_min: host_min.to_string(),
        }
    }

    #[test]
    fn roundtrips_the_ai_plugin_release() {
        let dir = tempfile::tempdir().unwrap();
        let paths = UpdatePaths::new(dir.path());
        let value = release("0.2.0", "0.15.0");
        store(&paths, Some(&value)).unwrap();
        assert_eq!(load(&paths), Some(value));
    }

    #[test]
    fn removes_the_cache_when_the_feed_has_no_release() {
        let dir = tempfile::tempdir().unwrap();
        let paths = UpdatePaths::new(dir.path());
        store(&paths, Some(&release("0.2.0", "0.15.0"))).unwrap();
        store(&paths, None).unwrap();
        assert_eq!(load(&paths), None);
    }

    #[test]
    fn ignores_a_malformed_cache_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = UpdatePaths::new(dir.path());
        std::fs::create_dir_all(paths.root.clone()).unwrap();
        std::fs::write(paths.ai_plugin_file(), b"not json").unwrap();
        assert_eq!(load(&paths), None);
    }

    #[test]
    fn ignores_a_cache_file_with_a_non_semver_version() {
        let dir = tempfile::tempdir().unwrap();
        let paths = UpdatePaths::new(dir.path());
        std::fs::create_dir_all(paths.root.clone()).unwrap();
        std::fs::write(
            paths.ai_plugin_file(),
            br#"{"version":"latest","host_version_min":"0.15.0"}"#,
        )
        .unwrap();
        assert_eq!(load(&paths), None);
    }
}
