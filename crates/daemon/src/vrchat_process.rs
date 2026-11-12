//! Detects whether the VRChat client process is currently running.
//!
//! Detection is intentionally one-way: the daemon uses a missing process as a
//! reason to close the active session, but never uses a live process to open or
//! keep one open. `None` means the process table could not be read and must not
//! count as a miss.

use std::io;

fn is_vrchat_comm(comm: &str) -> bool {
    comm.trim() == "VRChat.exe"
}

/// Minimal process-table access used by [`VrchatProcessProbe`].
trait ProcessTable {
    fn list(&self) -> io::Result<Vec<(u32, String)>>;
    fn comm(&self, pid: u32) -> io::Result<Option<String>>;
}

/// Caches the VRChat PID so a steady-state poll only reads one `comm` file.
///
/// A cached PID that cannot be read is not treated as "not running": the
/// probe falls back to a full process-table scan. Only a failed full scan
/// returns `None`.
pub(crate) struct VrchatProcessProbe {
    cached_pid: Option<u32>,
}

impl VrchatProcessProbe {
    pub(crate) fn new() -> Self {
        Self { cached_pid: None }
    }

    pub(crate) fn poll(&mut self) -> Option<bool> {
        #[cfg(target_os = "linux")]
        {
            self.poll_with(&LinuxProcessTable)
        }
        #[cfg(target_os = "windows")]
        {
            self.poll_with(&WindowsProcessTable)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            None
        }
    }

    fn poll_with(&mut self, table: &impl ProcessTable) -> Option<bool> {
        if let Some(pid) = self.cached_pid {
            match table.comm(pid) {
                Ok(Some(comm)) if is_vrchat_comm(&comm) => return Some(true),
                // The cached PID is gone or no longer VRChat. A full scan is
                // the only way to tell whether VRChat restarted under a new PID.
                Ok(_) => self.cached_pid = None,
                // Access can be denied for an elevated process on Windows. The
                // process table still reports its name, so scan it instead of
                // turning this into a "not running" verdict.
                Err(_) => self.cached_pid = None,
            }
        }

        match table.list() {
            Ok(processes) => match processes.iter().find(|(_, comm)| is_vrchat_comm(comm)) {
                Some((pid, _)) => {
                    self.cached_pid = Some(*pid);
                    Some(true)
                }
                None => {
                    self.cached_pid = None;
                    Some(false)
                }
            },
            Err(_) => None,
        }
    }
}

/// Returns `Some(true)` when `VRChat.exe` is running, `Some(false)` when the
/// process list was read and it is absent, and `None` when detection failed.
pub fn is_running() -> Option<bool> {
    let mut probe = VrchatProcessProbe::new();
    probe.poll()
}

#[cfg(target_os = "linux")]
struct LinuxProcessTable;

#[cfg(target_os = "linux")]
impl ProcessTable for LinuxProcessTable {
    fn list(&self) -> io::Result<Vec<(u32, String)>> {
        let entries = std::fs::read_dir("/proc")?;
        let mut processes = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(pid) = name.to_str().and_then(|pid| pid.parse::<u32>().ok()) else {
                continue;
            };
            match std::fs::read_to_string(entry.path().join("comm")) {
                Ok(comm) => processes.push((pid, comm)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                // A process we cannot inspect may be VRChat. Never turn a
                // partial process-table read into a "not running" verdict.
                Err(error) => return Err(error),
            }
        }
        Ok(processes)
    }

    fn comm(&self, pid: u32) -> io::Result<Option<String>> {
        match std::fs::read_to_string(format!("/proc/{pid}/comm")) {
            Ok(comm) => Ok(Some(comm)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(target_os = "windows")]
struct WindowsProcessTable;

#[cfg(target_os = "windows")]
impl ProcessTable for WindowsProcessTable {
    fn list(&self) -> io::Result<Vec<(u32, String)>> {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        };

        // SAFETY: `CreateToolhelp32Snapshot` returns an owned snapshot handle or
        // an error. The handle is closed exactly once below and no pointer is
        // stored.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|error| io::Error::other(error.to_string()))?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        debug_assert_eq!(
            entry.dwSize as usize,
            std::mem::size_of::<PROCESSENTRY32W>(),
            "PROCESSENTRY32W.dwSize must match the struct layout"
        );
        let mut processes = Vec::new();
        // SAFETY: `snapshot` is a valid process snapshot, `entry.dwSize` is set
        // to the struct size, and `entry` is a distinct stack buffer that
        // Windows initializes before returning.
        let result = match unsafe { Process32FirstW(snapshot, &mut entry) } {
            Ok(()) => loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&unit| unit == 0)
                    .unwrap_or(entry.szExeFile.len());
                processes.push((
                    entry.th32ProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..len]),
                ));
                // SAFETY: same snapshot and entry buffer as Process32FirstW; the
                // buffer stays alive and is not aliased for the duration of the
                // call.
                match unsafe { Process32NextW(snapshot, &mut entry) } {
                    Ok(()) => {}
                    Err(error) if is_end_of_process_list(&error) => break Ok(()),
                    Err(error) => break Err(error),
                }
            },
            Err(error) if is_end_of_process_list(&error) => Ok(()),
            Err(error) => Err(error),
        };
        // SAFETY: `snapshot` is the owned handle returned by
        // CreateToolhelp32Snapshot and is closed exactly once here.
        unsafe {
            let _ = CloseHandle(snapshot);
        }
        result
            .map(|()| processes)
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn comm(&self, pid: u32) -> io::Result<Option<String>> {
        use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER};
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        };

        // SAFETY: `OpenProcess` returns an owned handle or an error; `false`
        // means the handle is not inherited. The PID is not dereferenced.
        let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
            Ok(handle) => handle,
            Err(error) => {
                let code = error.code().0 as u32 & 0xffff;
                if code == ERROR_INVALID_PARAMETER.0 {
                    return Ok(None);
                }
                return Err(io::Error::other(error.to_string()));
            }
        };
        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        // SAFETY: `handle` was returned by OpenProcess with query access and is
        // still open; `buffer` is a valid writable stack buffer and `length`
        // carries its capacity.
        let result = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        };
        // SAFETY: `handle` is owned here and is neither used nor closed anywhere
        // else before this call.
        unsafe {
            let _ = CloseHandle(handle);
        }
        match result {
            Ok(()) => {
                let length = length.min(buffer.len() as u32) as usize;
                let path = String::from_utf16_lossy(&buffer[..length]);
                let name = path
                    .rsplit(|c| c == '\\' || c == '/')
                    .next()
                    .unwrap_or(&path);
                Ok(Some(name.to_string()))
            }
            Err(error) => Err(io::Error::other(error.to_string())),
        }
    }
}

#[cfg(target_os = "windows")]
fn is_end_of_process_list(error: &windows::core::Error) -> bool {
    error.code()
        == windows::core::HRESULT::from_win32(windows::Win32::Foundation::ERROR_NO_MORE_FILES.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeTable {
        processes: Mutex<Vec<(u32, String)>>,
        list_calls: AtomicUsize,
        comm_calls: AtomicUsize,
        fail_comm: Option<std::io::ErrorKind>,
        fail_list: Option<std::io::ErrorKind>,
    }

    impl FakeTable {
        fn new(processes: &[(u32, &str)]) -> Self {
            Self {
                processes: Mutex::new(
                    processes
                        .iter()
                        .map(|(pid, comm)| (*pid, (*comm).to_string()))
                        .collect(),
                ),
                list_calls: AtomicUsize::new(0),
                comm_calls: AtomicUsize::new(0),
                fail_comm: None,
                fail_list: None,
            }
        }

        fn set_processes(&self, processes: &[(u32, &str)]) {
            *self.processes.lock().unwrap() = processes
                .iter()
                .map(|(pid, comm)| (*pid, (*comm).to_string()))
                .collect();
        }

        fn list_calls(&self) -> usize {
            self.list_calls.load(Ordering::Relaxed)
        }

        fn comm_calls(&self) -> usize {
            self.comm_calls.load(Ordering::Relaxed)
        }
    }

    impl ProcessTable for FakeTable {
        fn list(&self) -> std::io::Result<Vec<(u32, String)>> {
            self.list_calls.fetch_add(1, Ordering::Relaxed);
            if let Some(kind) = self.fail_list {
                return Err(std::io::Error::new(kind, "list failed"));
            }
            Ok(self.processes.lock().unwrap().clone())
        }

        fn comm(&self, pid: u32) -> std::io::Result<Option<String>> {
            self.comm_calls.fetch_add(1, Ordering::Relaxed);
            if let Some(kind) = self.fail_comm {
                return Err(std::io::Error::new(kind, "comm failed"));
            }
            Ok(self
                .processes
                .lock()
                .unwrap()
                .iter()
                .find(|(candidate, _)| *candidate == pid)
                .map(|(_, comm)| comm.clone()))
        }
    }

    #[test]
    fn matches_vrchat_comm_with_line_endings() {
        assert!(is_vrchat_comm("VRChat.exe\n"));
        assert!(is_vrchat_comm("VRChat.exe"));
        assert!(!is_vrchat_comm("vrchat.exe"));
        assert!(!is_vrchat_comm("VRChat.exe.old"));
        assert!(!is_vrchat_comm("wine64-preloader"));
    }

    #[test]
    fn probe_reuses_the_cached_pid_without_listing_the_table() {
        let table = FakeTable::new(&[(42, "VRChat.exe")]);
        let mut probe = VrchatProcessProbe::new();

        assert_eq!(probe.poll_with(&table), Some(true));
        assert_eq!(table.list_calls(), 1);

        assert_eq!(probe.poll_with(&table), Some(true));
        assert_eq!(
            table.list_calls(),
            1,
            "cached poll must not list /proc again"
        );
        assert_eq!(table.comm_calls(), 1);
    }

    #[test]
    fn probe_rescans_when_the_cached_pid_disappears() {
        let table = FakeTable::new(&[(42, "VRChat.exe")]);
        let mut probe = VrchatProcessProbe::new();
        assert_eq!(probe.poll_with(&table), Some(true));

        table.set_processes(&[(43, "VRChat.exe")]);
        assert_eq!(probe.poll_with(&table), Some(true));
        assert_eq!(table.list_calls(), 2);
        assert_eq!(table.comm_calls(), 1);
    }

    #[test]
    fn probe_falls_back_to_a_full_scan_when_the_cached_pid_cannot_be_read() {
        let mut table = FakeTable::new(&[(42, "VRChat.exe")]);
        let mut probe = VrchatProcessProbe::new();
        assert_eq!(probe.poll_with(&table), Some(true));

        table.set_processes(&[(43, "VRChat.exe")]);
        table.fail_comm = Some(std::io::ErrorKind::PermissionDenied);
        assert_eq!(probe.poll_with(&table), Some(true));
        assert_eq!(table.list_calls(), 2);
    }

    #[test]
    fn probe_returns_none_when_the_process_table_cannot_be_read() {
        let mut table = FakeTable::new(&[(42, "VRChat.exe")]);
        let mut probe = VrchatProcessProbe::new();
        assert_eq!(probe.poll_with(&table), Some(true));

        table.fail_comm = Some(std::io::ErrorKind::PermissionDenied);
        table.fail_list = Some(std::io::ErrorKind::PermissionDenied);
        assert_eq!(probe.poll_with(&table), None);
        assert_eq!(table.list_calls(), 2);
    }

    #[test]
    fn probe_returns_false_when_vrchat_is_absent() {
        let table = FakeTable::new(&[]);
        let mut probe = VrchatProcessProbe::new();
        assert_eq!(probe.poll_with(&table), Some(false));
        assert_eq!(table.list_calls(), 1);
    }
}
