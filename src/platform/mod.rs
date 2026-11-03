//! Platform abstraction layer.
//!
//! All operating-system specific calls live below this module. The rest of
//! the application consumes the plain data types defined here and never calls
//! OS APIs directly (in particular, UI rendering code must not).
//!
//! On non-Windows hosts a fallback implementation reports every
//! Windows-specific value as unavailable. That build exists only so the
//! platform-independent parts can be developed and tested anywhere.

#[cfg(not(windows))]
mod fallback;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
use fallback as imp;
#[cfg(windows)]
use windows as imp;

pub use imp::ProcessEnumerator;

use crate::core::models::ProcessKey;
use crate::process::details::ProcessDetails;

/// Reads command line, threads and modules of one process instance.
/// Potentially slow; call from a worker thread.
pub fn inspect_process(key: ProcessKey) -> ProcessDetails {
    imp::inspect_process(key)
}

/// Ends a process instance. The caller is responsible for user confirmation
/// and for [`crate::process::safety::check_terminate`]; the platform layer
/// additionally re-verifies identity and the critical flag.
pub fn terminate_process(key: ProcessKey) -> Result<(), String> {
    imp::terminate_process(key)
}

/// Shows the file in the system file manager.
pub fn open_file_location(path: &str) -> Result<(), String> {
    imp::open_file_location(path)
}

/// System-wide counters that the cross-platform crates do not expose.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KernelCounters {
    pub handle_count: u64,
    pub thread_count: u64,
    pub process_count: u64,
    /// Commit charge in bytes.
    pub commit_total: u64,
    /// Commit limit in bytes.
    pub commit_limit: u64,
    /// System file cache in bytes.
    pub system_cache: u64,
    pub kernel_paged: u64,
    pub kernel_nonpaged: u64,
}

/// Cumulative counters of one physical disk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhysicalDiskCounters {
    pub index: u32,
    pub bytes_read: u64,
    pub bytes_written: u64,
    /// Cumulative idle time, 100 ns units.
    pub idle_time: u64,
    /// Timestamp of the reading, 100 ns units.
    pub query_time: u64,
}

/// Reads counters of `\\.\PhysicalDrive{index}`, or `None` if it does not
/// exist or cannot be queried.
pub fn physical_disk_counters(index: u32) -> Option<PhysicalDiskCounters> {
    imp::physical_disk_counters(index)
}

/// Reads kernel-wide counters, or `None` if the platform cannot provide them.
pub fn kernel_counters() -> Option<KernelCounters> {
    imp::kernel_counters()
}

/// Whether the current process runs with an elevated (administrator) token.
pub fn is_elevated() -> Option<bool> {
    imp::is_elevated()
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn kernel_counters_are_plausible() {
        let k = super::kernel_counters().expect("GetPerformanceInfo should succeed");
        assert!(k.process_count > 0);
        assert!(k.thread_count >= k.process_count);
        assert!(k.commit_limit >= k.commit_total);
    }

    #[test]
    fn enumerates_and_inspects_own_process() {
        let mut e = super::ProcessEnumerator::new();
        let mut list = Vec::new();
        e.enumerate(&mut list).unwrap();
        let me = list
            .iter()
            .find(|p| p.key.pid == std::process::id())
            .expect("own process listed");
        assert!(me.key.start_time > 0);
        assert!(me.exe_path.as_deref().is_some_and(|p| p.ends_with(".exe")));
        assert!(me.threads.unwrap_or(0) >= 1);
        assert!(me.working_set.unwrap_or(0) > 0);
        assert!(me.architecture.is_some());

        let details = super::inspect_process(me.key);
        assert!(details.command_line.is_ok(), "{:?}", details.command_line);
        assert!(!details.threads.unwrap().is_empty());

        // A stale instance key (wrong creation time) must never be terminated.
        let stale = crate::core::models::ProcessKey {
            pid: me.key.pid,
            start_time: me.key.start_time + 1,
        };
        assert!(super::terminate_process(stale).is_err());
    }

    #[test]
    fn first_physical_disk_reports_counters() {
        let c = super::physical_disk_counters(0).expect("PhysicalDrive0 should be queryable");
        assert!(c.query_time > 0);
    }

    #[test]
    fn elevation_is_known() {
        assert!(super::is_elevated().is_some());
    }
}
