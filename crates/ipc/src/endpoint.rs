use std::fs;
use std::io;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

use interprocess::local_socket::Name;
#[cfg(unix)]
use interprocess::local_socket::{GenericFilePath, ToFsName};
#[cfg(windows)]
use interprocess::local_socket::{GenericNamespaced, ToNsName};

/// A platform-specific local socket address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    inner: EndpointInner,
    /// True when the Unix socket lives in the shared temp directory
    /// fallback, whose parent directory needs ownership checks before use.
    fallback: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EndpointInner {
    #[cfg(unix)]
    Path(PathBuf),
    #[cfg(windows)]
    Namespaced(String),
}

impl Endpoint {
    /// Resolves the per-user daemon endpoint below the data directory.
    #[cfg(unix)]
    pub fn for_data_dir(data_dir: &Path) -> io::Result<Self> {
        let path = match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(runtime) if !runtime.is_empty() => PathBuf::from(runtime)
                .join("vrc-photon")
                .join("daemon.sock"),
            _ => data_dir.join("run").join("daemon.sock"),
        };
        // Unix domain socket paths are limited to 108 bytes on Linux. The
        // fallback is a private per-user directory rather than a shared path.
        let (path, fallback) = if path.as_os_str().len() > 100 {
            (fallback_dir().join("daemon.sock"), true)
        } else {
            (path, false)
        };
        Ok(Self {
            inner: EndpointInner::Path(path),
            fallback,
        })
    }

    /// Resolves the per-user daemon endpoint on Windows.
    #[cfg(windows)]
    pub fn for_data_dir(data_dir: &Path) -> io::Result<Self> {
        Ok(Self {
            inner: EndpointInner::Namespaced(daemon_pipe_name(data_dir)?),
            fallback: false,
        })
    }

    /// Creates a unique endpoint for tests.
    #[cfg(unix)]
    pub fn for_tests(dir: &Path) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: EndpointInner::Path(
                dir.join(format!("daemon-{}-{counter}.sock", std::process::id())),
            ),
            fallback: false,
        }
    }

    /// Creates a unique endpoint for tests.
    #[cfg(windows)]
    pub fn for_tests(_dir: &Path) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: EndpointInner::Namespaced(format!(
                "vrc-photon-test-{}-{counter}",
                std::process::id()
            )),
            fallback: false,
        }
    }

    /// Whether this endpoint uses the shared temp directory fallback.
    #[cfg(unix)]
    pub(crate) fn is_fallback(&self) -> bool {
        self.fallback
    }

    /// Builds a fallback endpoint at an explicit path, for tests.
    #[cfg(all(unix, test))]
    pub(crate) fn fallback_for_tests(path: PathBuf) -> Self {
        Self {
            inner: EndpointInner::Path(path),
            fallback: true,
        }
    }

    /// Human-readable endpoint used in errors and logs.
    pub fn display(&self) -> String {
        match &self.inner {
            #[cfg(unix)]
            EndpointInner::Path(path) => path.display().to_string(),
            #[cfg(windows)]
            EndpointInner::Namespaced(name) => name.clone(),
        }
    }

    /// Filesystem path for Unix cleanup and permission handling.
    #[cfg(unix)]
    pub fn path(&self) -> Option<&Path> {
        match &self.inner {
            EndpointInner::Path(path) => Some(path),
        }
    }

    /// Named pipes do not have a filesystem path.
    #[cfg(windows)]
    pub fn path(&self) -> Option<&Path> {
        None
    }

    pub(crate) fn name(&self) -> io::Result<Name<'_>> {
        match &self.inner {
            #[cfg(unix)]
            EndpointInner::Path(path) => path.as_path().to_fs_name::<GenericFilePath>(),
            #[cfg(windows)]
            EndpointInner::Namespaced(name) => name.as_str().to_ns_name::<GenericNamespaced>(),
        }
    }
}

/// The current user's uid, read from `/proc/self` (Linux). `None` when it
/// cannot be determined, in which case ownership checks are skipped.
#[cfg(unix)]
fn current_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata("/proc/self")
        .map(|metadata| metadata.uid())
        .ok()
}

/// The per-user fallback directory inside the shared temp directory. The
/// uid form keeps the name from being predictable across users even when
/// `USER` is spoofed or shared.
#[cfg(unix)]
fn fallback_dir() -> PathBuf {
    let name = match current_uid() {
        Some(uid) => format!("vrc-photon-{uid}"),
        None => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
            format!("vrc-photon-{user}")
        }
    };
    std::env::temp_dir().join(name)
}

/// Rejects a fallback directory that already exists in an unsafe state:
/// not a directory, a symlink, or owned by another user. A missing
/// directory is safe by definition (nothing can be spoofed through it yet).
#[cfg(unix)]
pub(crate) fn verify_fallback_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "IPC fallback directory is not a plain directory: {}",
                dir.display()
            ),
        ));
    }
    if let Some(uid) = current_uid()
        && metadata.uid() != uid
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "IPC fallback directory is owned by another user: {}",
                dir.display()
            ),
        ));
    }
    Ok(())
}

/// Creates the fallback directory if needed and verifies the result, so a
/// directory planted by another user or a symlink is never used for the
/// socket.
#[cfg(unix)]
pub(crate) fn ensure_safe_fallback_dir(dir: &Path) -> io::Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(_) => verify_fallback_dir(dir),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            use std::os::unix::fs::PermissionsExt;
            fs::create_dir_all(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
            // Verify what is actually there now: a pre-planted directory or
            // symlink must not slip through the creation path.
            verify_fallback_dir(dir)
        }
        Err(error) => Err(error),
    }
}

#[cfg(any(windows, test))]
const PIPE_NONCE_FILE: &str = "daemon-pipe-nonce";

#[cfg(any(windows, test))]
fn valid_pipe_nonce(nonce: &str) -> bool {
    nonce.len() == 32
        && nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(any(windows, test))]
fn read_pipe_nonce(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(nonce) if valid_pipe_nonce(&nonce) => Ok(Some(nonce)),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Waits briefly for a concurrently created file to finish being written.
/// Another process can observe the file between `create_new` and the first
/// write, so an immediate invalid read is not proof of corruption.
#[cfg(any(windows, test))]
fn wait_for_pipe_nonce(path: &Path) -> io::Result<Option<String>> {
    for _ in 0..20 {
        if let Some(nonce) = read_pipe_nonce(path)? {
            return Ok(Some(nonce));
        }
        if !path.exists() {
            return Ok(None);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(None)
}

#[cfg(any(windows, test))]
fn create_pipe_nonce(path: &Path) -> io::Result<String> {
    use std::io::Write as _;

    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(nonce.as_bytes())?;
    file.sync_all()?;
    Ok(nonce)
}

#[cfg(any(windows, test))]
fn read_or_create_pipe_nonce(data_dir: &Path) -> io::Result<String> {
    fs::create_dir_all(data_dir)?;
    let path = data_dir.join(PIPE_NONCE_FILE);
    if let Some(nonce) = wait_for_pipe_nonce(&path)? {
        return Ok(nonce);
    }
    if path.exists() {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    match create_pipe_nonce(&path) {
        Ok(nonce) => Ok(nonce),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            // Another process won the create race. Its nonce is authoritative.
            if let Some(nonce) = wait_for_pipe_nonce(&path)? {
                return Ok(nonce);
            }
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            create_pipe_nonce(&path)
        }
        Err(error) => Err(error),
    }
}

#[cfg(any(windows, test))]
fn daemon_pipe_name(data_dir: &Path) -> io::Result<String> {
    Ok(format!(
        "vrc-photon-daemon-{}",
        read_or_create_pipe_nonce(data_dir)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoints_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        assert_ne!(
            Endpoint::for_tests(dir.path()),
            Endpoint::for_tests(dir.path())
        );
    }

    #[test]
    fn endpoint_display_is_non_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!Endpoint::for_tests(dir.path()).display().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn fallback_dir_is_named_for_the_current_user() {
        let dir = fallback_dir();
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        match current_uid() {
            Some(uid) => assert_eq!(name, format!("vrc-photon-{uid}")),
            None => assert!(name.starts_with("vrc-photon-")),
        }
        assert_eq!(dir.parent(), Some(std::env::temp_dir().as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_safe_fallback_dir_creates_a_private_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("fallback");
        ensure_safe_fallback_dir(&target).unwrap();
        let metadata = std::fs::metadata(&target).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
        // An existing, healthy directory stays accepted.
        ensure_safe_fallback_dir(&target).unwrap();
        verify_fallback_dir(&target).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn ensure_safe_fallback_dir_rejects_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let error = ensure_safe_fallback_dir(&link).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let error = verify_fallback_dir(&link).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_safe_fallback_dir_rejects_a_plain_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        let error = ensure_safe_fallback_dir(&file).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_safe_fallback_dir_rejects_another_users_directory() {
        let Some(uid) = current_uid() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("foreign");
        std::fs::create_dir(&target).unwrap();
        // Hand the directory to a different uid. Needs privileges (root);
        // skip when the environment cannot chown.
        if std::os::unix::fs::chown(&target, Some(uid + 1), None).is_err() {
            return;
        }
        let error = ensure_safe_fallback_dir(&target).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let error = verify_fallback_dir(&target).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[cfg(unix)]
    #[test]
    fn verify_fallback_dir_accepts_a_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        verify_fallback_dir(&dir.path().join("missing")).unwrap();
    }

    #[test]
    fn pipe_name_is_stable_for_one_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            daemon_pipe_name(dir.path()).unwrap(),
            daemon_pipe_name(dir.path()).unwrap()
        );
    }

    #[test]
    fn pipe_names_differ_between_data_dirs() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        assert_ne!(
            daemon_pipe_name(first.path()).unwrap(),
            daemon_pipe_name(second.path()).unwrap()
        );
    }

    #[test]
    fn invalid_pipe_nonce_is_regenerated() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("daemon-pipe-nonce"), b"broken").unwrap();
        let name = daemon_pipe_name(dir.path()).unwrap();
        let nonce = std::fs::read_to_string(dir.path().join("daemon-pipe-nonce")).unwrap();
        assert_eq!(name, format!("vrc-photon-daemon-{nonce}"));
        assert_eq!(nonce.len(), 32);
        assert!(nonce.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(nonce.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()));
    }

    #[cfg(unix)]
    #[test]
    fn pipe_nonce_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        daemon_pipe_name(dir.path()).unwrap();
        let mode = std::fs::metadata(dir.path().join("daemon-pipe-nonce"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
