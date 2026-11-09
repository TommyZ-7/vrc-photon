//! Stable process-instance names and locks for cross-process single-instance
//! coordination.
//!
//! A name combines the operating-system user identity, the process kind, and a
//! hash of the absolute database path. Two processes that work on the same
//! library therefore share a lock, while another library (or another OS user)
//! gets its own. Names are restricted to lowercase ASCII and a fixed-width
//! hash so they are valid for `single_instance::SingleInstance` on every
//! supported platform and stay well below the 108-byte Linux socket-path
//! limit.
//!
//! On Linux the lock is an abstract Unix socket created with `SOCK_CLOEXEC`,
//! so helper and daemon processes spawned by the UI cannot keep the lock after
//! the UI exits (#481). A socket inherited from an older, non-CLOEXEC version
//! is adopted instead of being treated as another running UI, which lets the
//! first update away from those versions pass its health check.

use std::path::{Component, Path, PathBuf};

/// Hex characters kept from the BLAKE3 digest.
const HASH_HEX_LEN: usize = 32;
/// Longest sanitized `<kind>` component. Together with the prefix, separator,
/// and hash this keeps every name under 70 bytes.
const MAX_KIND_LEN: usize = 24;

/// Returns the single-instance name for `kind` scoped to `db_path`.
///
/// The same user, kind, and database path always produce the same name. Paths
/// are made absolute and lexically normalized first, so `./library.db` and
/// `/home/user/library.db` match when the working directory agrees.
pub fn instance_name(kind: &str, db_path: &Path) -> String {
    instance_name_with_user(kind, db_path, &user_identity())
}

fn instance_name_with_user(kind: &str, db_path: &Path, user: &str) -> String {
    let kind = sanitize_kind(kind);
    let path = normalized_path(db_path);
    let mut hasher = blake3::Hasher::new();
    hasher.update(user.as_bytes());
    hasher.update(&[0]);
    hasher.update(path.as_bytes());
    let digest = hasher.finalize();
    let hex = digest.to_hex();
    format!("vrc-photon-{kind}-{}", &hex.as_str()[..HASH_HEX_LEN])
}

/// Restricts a kind to lowercase ASCII letters, digits, and single hyphens.
fn sanitize_kind(kind: &str) -> String {
    let mut out = String::new();
    for ch in kind.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= MAX_KIND_LEN {
            break;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "app".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Converts a path to an absolute, lexically normalized string without
/// touching the filesystem, so the name is also stable before the database
/// file exists.
fn normalized_path(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized.to_string_lossy().into_owned()
}

#[cfg(unix)]
fn user_identity() -> String {
    // SAFETY: `getuid` has no preconditions and cannot fail.
    format!("uid-{}", unsafe { libc::getuid() })
}

#[cfg(windows)]
fn user_identity() -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| "unknown".to_string());
    let session = std::env::var("SESSIONNAME").unwrap_or_default();
    format!("{user}:{session}")
}

#[cfg(not(any(unix, windows)))]
fn user_identity() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

/// The process-wide single-instance lock for `name`.
///
/// On Linux this is backed by an abstract Unix socket; other platforms keep
/// using `single_instance::SingleInstance`.
#[cfg(target_os = "linux")]
mod guard {
    use std::io;
    use std::os::fd::RawFd;
    use std::sync::{LazyLock, Mutex};

    /// Longest name that fits in an abstract socket address.
    const MAX_ABSTRACT_NAME_LEN: usize = 107;

    /// FDs owned by guards in this process. The adoption scan must skip them
    /// so a second guard in the same process still reports `is_single() ==
    /// false`.
    static OWNED_FDS: LazyLock<Mutex<Vec<RawFd>>> = LazyLock::new(|| Mutex::new(Vec::new()));

    pub struct InstanceGuard {
        socket: Option<RawFd>,
    }

    impl InstanceGuard {
        /// Binds the abstract socket for `name`, or adopts an inherited one.
        pub fn new(name: &str) -> io::Result<Self> {
            if name.len() > MAX_ABSTRACT_NAME_LEN {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "instance name is too long for an abstract socket",
                ));
            }

            let socket = create_socket()?;
            match bind_abstract(socket, name.as_bytes()) {
                Ok(()) => {
                    register(socket);
                    Ok(Self {
                        socket: Some(socket),
                    })
                }
                Err(error) if error.raw_os_error() == Some(libc::EADDRINUSE) => {
                    close_fd(socket);
                    // An updater helper started by a version that predates
                    // #481 passes the old, inheritable socket to the new UI.
                    // Take over that socket instead of exiting: the helper
                    // then exits after the health check and the new UI keeps
                    // the lock.
                    match adopt_inherited(name.as_bytes()) {
                        Some(inherited) => {
                            if let Err(error) = set_close_on_exec(inherited) {
                                close_fd(inherited);
                                return Err(error);
                            }
                            register(inherited);
                            Ok(Self {
                                socket: Some(inherited),
                            })
                        }
                        None => Ok(Self { socket: None }),
                    }
                }
                Err(error) => {
                    close_fd(socket);
                    Err(error)
                }
            }
        }

        pub fn is_single(&self) -> bool {
            self.socket.is_some()
        }
    }

    impl Drop for InstanceGuard {
        fn drop(&mut self) {
            if let Some(socket) = self.socket.take() {
                unregister(socket);
                close_fd(socket);
            }
        }
    }

    fn create_socket() -> io::Result<RawFd> {
        // SAFETY: `socket` has no preconditions and the returned fd is checked.
        let socket =
            unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        if socket < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(socket)
        }
    }

    fn bind_abstract(socket: RawFd, name: &[u8]) -> io::Result<()> {
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (index, byte) in name.iter().enumerate() {
            address.sun_path[index + 1] = *byte as libc::c_char;
        }
        let length = (std::mem::size_of::<libc::sa_family_t>() + 1 + name.len()) as libc::socklen_t;
        // SAFETY: `address` is initialized for the abstract name and is large
        // enough for the validated name length.
        let result = unsafe {
            libc::bind(
                socket,
                &address as *const _ as *const libc::sockaddr,
                length,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Finds a socket already bound to `name` among this process's inherited
    /// file descriptors.
    fn adopt_inherited(name: &[u8]) -> Option<RawFd> {
        let entries = std::fs::read_dir("/proc/self/fd").ok()?;
        for entry in entries.flatten() {
            let Some(fd) = entry
                .file_name()
                .to_str()
                .and_then(|value| value.parse::<RawFd>().ok())
            else {
                continue;
            };
            if fd < 3 || is_owned(fd) {
                continue;
            }
            if fd_is_abstract_socket(fd, name) {
                return Some(fd);
            }
        }
        None
    }

    fn fd_is_abstract_socket(fd: RawFd, expected: &[u8]) -> bool {
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        // SAFETY: `getsockname` writes at most `length` bytes into `address`.
        let result = unsafe {
            libc::getsockname(
                fd,
                &mut address as *mut _ as *mut libc::sockaddr,
                &mut length,
            )
        };
        if result != 0 || address.sun_family != libc::AF_UNIX as libc::sa_family_t {
            return false;
        }

        let offset = std::mem::size_of::<libc::sa_family_t>();
        let length = length as usize;
        if length <= offset
            || length > std::mem::size_of::<libc::sockaddr_un>()
            || address.sun_path[0] != 0
        {
            return false;
        }
        let path_length = length - offset - 1;
        if path_length != expected.len() {
            return false;
        }
        // SAFETY: the returned abstract path occupies `path_length` bytes
        // after the leading NUL, inside `sun_path`.
        let path = unsafe {
            std::slice::from_raw_parts(address.sun_path.as_ptr().add(1) as *const u8, path_length)
        };
        path == expected
    }

    fn set_close_on_exec(fd: RawFd) -> io::Result<()> {
        // SAFETY: `fcntl` only reads and sets flags on the owned fd.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: same as above; `FD_CLOEXEC` does not change the descriptor
        // while this process is running.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn close_fd(fd: RawFd) {
        // SAFETY: the caller owns `fd` and closes it exactly once.
        unsafe {
            libc::close(fd);
        }
    }

    fn register(fd: RawFd) {
        locked_owned_fds().push(fd);
    }

    fn unregister(fd: RawFd) {
        let mut owned = locked_owned_fds();
        if let Some(index) = owned.iter().position(|owned| *owned == fd) {
            owned.swap_remove(index);
        }
    }

    fn is_owned(fd: RawFd) -> bool {
        locked_owned_fds().contains(&fd)
    }

    fn locked_owned_fds() -> std::sync::MutexGuard<'static, Vec<RawFd>> {
        OWNED_FDS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(not(target_os = "linux"))]
mod guard {
    use std::io;

    pub struct InstanceGuard {
        inner: single_instance::SingleInstance,
    }

    impl InstanceGuard {
        pub fn new(name: &str) -> io::Result<Self> {
            let inner = single_instance::SingleInstance::new(name)
                .map_err(|error| io::Error::other(error.to_string()))?;
            Ok(Self { inner })
        }

        pub fn is_single(&self) -> bool {
            self.inner.is_single()
        }
    }
}

pub use guard::InstanceGuard;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_differs_by_database_path() {
        let first = instance_name_with_user("daemon", Path::new("/tmp/one/library.db"), "alice");
        let second = instance_name_with_user("daemon", Path::new("/tmp/two/library.db"), "alice");
        assert_ne!(first, second);
    }

    #[test]
    fn name_is_equal_for_the_same_path() {
        let first = instance_name_with_user("daemon", Path::new("/tmp/library.db"), "alice");
        let second = instance_name_with_user("daemon", Path::new("/tmp/library.db"), "alice");
        assert_eq!(first, second);
    }

    #[test]
    fn name_normalizes_relative_and_parent_components() {
        let absolute = instance_name_with_user("ui", Path::new("/tmp/photos/library.db"), "alice");
        let dotted = instance_name_with_user("ui", Path::new("/tmp/photos/./library.db"), "alice");
        let parent =
            instance_name_with_user("ui", Path::new("/tmp/photos/../photos/library.db"), "alice");
        assert_eq!(absolute, dotted);
        assert_eq!(absolute, parent);
    }

    #[test]
    fn name_is_safe_for_single_instance() {
        let name = instance_name_with_user("daemon", Path::new("/tmp/library.db"), "alice");
        assert!(name.starts_with("vrc-photon-daemon-"));
        assert!(name.len() < 100, "{name} is {} bytes", name.len());
        assert!(!name.contains('\0'));
        assert!(!name.chars().any(char::is_whitespace));
        assert!(
            name.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        );
        assert_eq!(name.rsplit('-').next().unwrap().len(), HASH_HEX_LEN);
    }

    #[test]
    fn name_includes_the_user_component() {
        let alice = instance_name_with_user("ui", Path::new("/tmp/library.db"), "alice");
        let bob = instance_name_with_user("ui", Path::new("/tmp/library.db"), "bob");
        assert_ne!(alice, bob);
        assert!(!user_identity().is_empty());
        assert_eq!(
            instance_name_with_user("ui", Path::new("/tmp/library.db"), &user_identity()),
            instance_name("ui", Path::new("/tmp/library.db"))
        );
    }

    #[test]
    fn unsafe_kind_is_sanitized() {
        let name = instance_name_with_user("daemon/worker \u{1f600}", Path::new("/tmp/db"), "u");
        assert!(name.starts_with("vrc-photon-daemon-worker-"), "{name}");
        assert!(name.len() < 100);
    }

    #[cfg(target_os = "linux")]
    fn unique_lock_name() -> String {
        format!(
            "vrc-photon-test-lock-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        )
    }

    #[cfg(target_os = "linux")]
    fn bind_without_close_on_exec(name: &str) -> std::os::fd::RawFd {
        // Mimics single-instance 0.3.3, which created the Linux socket without
        // SOCK_CLOEXEC.
        let socket = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        assert!(socket >= 0, "{}", std::io::Error::last_os_error());
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (index, byte) in name.as_bytes().iter().enumerate() {
            address.sun_path[index + 1] = *byte as libc::c_char;
        }
        let length = (std::mem::size_of::<libc::sa_family_t>() + 1 + name.len()) as libc::socklen_t;
        let result = unsafe {
            libc::bind(
                socket,
                &address as *const _ as *const libc::sockaddr,
                length,
            )
        };
        assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
        socket
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn lock_is_exclusive_until_dropped() {
        let name = unique_lock_name();
        let first = InstanceGuard::new(&name).unwrap();
        assert!(first.is_single());
        let second = InstanceGuard::new(&name).unwrap();
        assert!(!second.is_single());
        drop(first);
        let third = InstanceGuard::new(&name).unwrap();
        assert!(third.is_single());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn spawned_children_do_not_retain_the_lock() {
        let name = unique_lock_name();
        let guard = InstanceGuard::new(&name).unwrap();
        assert!(guard.is_single());

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        drop(guard);

        let after = InstanceGuard::new(&name).unwrap();
        let retained_by_child = !after.is_single();
        let _ = child.kill();
        let _ = child.wait();
        assert!(!retained_by_child, "the child inherited the lock socket");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_lock_is_adopted_during_an_update() {
        let name = unique_lock_name();
        let _inherited = bind_without_close_on_exec(&name);

        let guard = InstanceGuard::new(&name).unwrap();
        assert!(guard.is_single());
        drop(guard);

        let fresh = InstanceGuard::new(&name).unwrap();
        assert!(fresh.is_single());
    }
}
