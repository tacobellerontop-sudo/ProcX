//! Data models describing system state.
//!
//! Optional fields use `None` to mean "the platform could not provide this
//! value", which the UI must render differently from a measured zero.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// Primary navigation sections of the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    #[default]
    Overview,
    Processes,
    Performance,
    Services,
    Startup,
    Details,
    History,
    Settings,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Overview,
        Section::Processes,
        Section::Performance,
        Section::Services,
        Section::Startup,
        Section::Details,
        Section::History,
        Section::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Processes => "Processes",
            Self::Performance => "Performance",
            Self::Services => "Services",
            Self::Startup => "Startup",
            Self::Details => "Details",
            Self::History => "History",
            Self::Settings => "Settings",
        }
    }
}

/// Identifies one process *instance*.
///
/// Windows reuses PIDs, so a PID alone is not a stable identity over time.
/// Combining it with the process creation time distinguishes a new process
/// that happens to receive a recycled PID.
///
/// `start_time` is a platform-specific monotonic stamp: on Windows the raw
/// creation `FILETIME` (100 ns units since 1601), elsewhere seconds since the
/// Unix epoch. It is 0 when the creation time could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProcessKey {
    pub pid: u32,
    pub start_time: u64,
}

/// CPU architecture of a process image.
// Only the Windows backend can determine the architecture.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Architecture {
    X64,
    X86,
    Arm64,
    Other(u16),
}

impl Architecture {
    pub fn label(self) -> String {
        match self {
            Self::X64 => "x64".into(),
            Self::X86 => "x86".into(),
            Self::Arm64 => "ARM64".into(),
            Self::Other(machine) => format!("0x{machine:04x}"),
        }
    }
}

// Windows only reports `Running`; other states come from the fallback backend.
#[cfg_attr(windows, allow(dead_code))]
/// Coarse process state. Windows does not expose a single "state" for a
/// process, so live processes report `Running` unless a platform can say more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProcessStatus {
    Running,
    Sleeping,
    Stopped,
    Zombie,
    Unknown,
}

impl ProcessStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Sleeping => "Sleeping",
            Self::Stopped => "Stopped",
            Self::Zombie => "Zombie",
            Self::Unknown => "Unknown",
        }
    }
}

/// How much ProcX could read about a process with its current privileges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessLevel {
    /// The process could be opened for (limited) queries.
    Queryable,
    /// The process could not be opened; only enumeration data is available.
    Denied,
}

/// One process as seen in a single sample. Every optional field is `None`
/// when the platform could not provide it (commonly: access denied).
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessInfo {
    pub key: ProcessKey,
    pub parent_pid: Option<u32>,
    pub name: Arc<str>,
    pub exe_path: Option<Arc<str>>,
    pub user: Option<Arc<str>>,
    pub architecture: Option<Architecture>,
    pub session_id: Option<u32>,
    pub started_at: Option<SystemTime>,
    /// CPU utilization normalized to all logical processors, 0..=100.
    /// `None` until two samples exist or if CPU times are unreadable.
    pub cpu_percent: Option<f32>,
    /// Total CPU time (kernel + user) consumed so far.
    pub cpu_time: Option<Duration>,
    /// Working set (resident memory) in bytes.
    pub working_set: Option<u64>,
    pub peak_working_set: Option<u64>,
    /// Private committed memory in bytes.
    pub private_bytes: Option<u64>,
    pub threads: Option<u32>,
    pub handles: Option<u32>,
    pub status: ProcessStatus,
    pub access: AccessLevel,
    /// Whether Windows marks the process critical (terminating it bugchecks the system).
    pub critical: Option<bool>,
}

impl ProcessInfo {
    /// A process with only identity fields set.
    pub fn new(key: ProcessKey, name: impl Into<Arc<str>>) -> Self {
        Self {
            key,
            parent_pid: None,
            name: name.into(),
            exe_path: None,
            user: None,
            architecture: None,
            session_id: None,
            started_at: None,
            cpu_percent: None,
            cpu_time: None,
            working_set: None,
            peak_working_set: None,
            private_bytes: None,
            threads: None,
            handles: None,
            status: ProcessStatus::Unknown,
            access: AccessLevel::Denied,
            critical: None,
        }
    }
}

/// Information gathered once at start-up.
#[derive(Debug, Clone, Default)]
pub struct StaticSystemInfo {
    pub host_name: Option<String>,
    pub os_version: Option<String>,
    pub kernel_version: Option<String>,
    pub cpu_brand: Option<String>,
    pub cpu_arch: String,
    pub logical_cpus: usize,
    pub physical_cores: Option<usize>,
    /// Whether ProcX runs with an elevated token. `None` if unknown.
    pub elevated: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct CpuSnapshot {
    /// Overall utilization, 0..=100.
    pub total_usage: f32,
    /// Per logical processor utilization, 0..=100.
    pub per_core: Vec<f32>,
    /// Current frequency of the first logical processor in MHz, if reported.
    pub frequency_mhz: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct MemorySnapshot {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    /// Committed virtual memory (Windows commit charge).
    pub commit_total: Option<u64>,
    /// Commit limit (physical memory + page files).
    pub commit_limit: Option<u64>,
    /// File system cache.
    pub system_cache: Option<u64>,
    pub kernel_paged: Option<u64>,
    pub kernel_nonpaged: Option<u64>,
}

impl MemorySnapshot {
    pub fn used_percent(&self) -> f32 {
        percent(self.used, self.total)
    }

    pub fn commit_percent(&self) -> Option<f32> {
        Some(percent(self.commit_total?, self.commit_limit?))
    }
}

#[derive(Debug, Clone, Default)]
pub struct DiskSnapshot {
    pub name: String,
    pub mount_point: String,
    pub kind: DiskKindLabel,
    pub removable: bool,
    pub total_space: u64,
    pub available_space: u64,
    pub read_bytes_per_sec: f64,
    pub write_bytes_per_sec: f64,
}

impl DiskSnapshot {
    pub fn used_percent(&self) -> f32 {
        percent(
            self.total_space.saturating_sub(self.available_space),
            self.total_space,
        )
    }
}

/// Throughput and activity of one physical disk.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhysicalDiskSnapshot {
    pub index: u32,
    pub read_bytes_per_sec: f64,
    pub write_bytes_per_sec: f64,
    /// Share of time the disk was busy, 0..=100. `None` on the first sample.
    pub active_percent: Option<f32>,
}

impl PhysicalDiskSnapshot {
    pub fn label(&self) -> String {
        format!("Disk {}", self.index)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DiskKindLabel {
    Ssd,
    Hdd,
    #[default]
    Unknown,
}

impl DiskKindLabel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ssd => "SSD",
            Self::Hdd => "HDD",
            Self::Unknown => "Disk",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct InterfaceSnapshot {
    pub name: String,
    pub received_per_sec: f64,
    pub transmitted_per_sec: f64,
    pub total_received: u64,
    pub total_transmitted: u64,
    pub mac_address: Option<String>,
    pub addresses: Vec<String>,
    /// Link speed in bits per second, if reported.
    pub link_speed_bps: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkSnapshot {
    pub received_per_sec: f64,
    pub transmitted_per_sec: f64,
    pub interfaces: Vec<InterfaceSnapshot>,
}

/// System-wide object counts.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemCounts {
    pub processes: usize,
    pub threads: Option<u64>,
    pub handles: Option<u64>,
}

/// A compact process entry for dashboards and top-N lists.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessSummary {
    pub key: ProcessKey,
    pub name: String,
    /// CPU utilization normalized to all logical processors, 0..=100.
    pub cpu_percent: f32,
    /// Resident memory (working set on Windows) in bytes.
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActivityKind {
    ProcessStarted,
    ProcessExited,
}

/// A notable change detected between two samples.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityEvent {
    pub at: SystemTime,
    pub kind: ActivityKind,
    pub key: ProcessKey,
    pub name: String,
}

/// One periodic sample of the whole system.
#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub captured_at: SystemTime,
    /// Time spent collecting this sample; useful for diagnosing overhead.
    pub collection_time: Duration,
    pub uptime: Option<Duration>,
    pub cpu: CpuSnapshot,
    pub memory: MemorySnapshot,
    pub disks: Vec<DiskSnapshot>,
    /// Physical disks (Windows); empty where unsupported.
    pub physical_disks: Vec<PhysicalDiskSnapshot>,
    pub network: NetworkSnapshot,
    pub counts: SystemCounts,
    pub top_cpu: Vec<ProcessSummary>,
    pub top_memory: Vec<ProcessSummary>,
    /// Every process in this sample, shared cheaply with the UI.
    pub processes: Arc<[ProcessInfo]>,
    /// Process starts/exits since the previous sample. Empty on the first sample.
    pub activity: Vec<ActivityEvent>,
    /// Number of activity events omitted because too many happened at once.
    pub activity_overflow: usize,
}

impl SystemSnapshot {
    /// The volume holding the operating system: `%SystemDrive%` on Windows,
    /// `/` elsewhere, falling back to the largest fixed volume.
    pub fn system_disk(&self) -> Option<&DiskSnapshot> {
        let system_root = std::env::var("SystemDrive")
            .map(|d| format!("{d}\\"))
            .unwrap_or_else(|_| "/".to_owned());
        self.disks
            .iter()
            .find(|d| d.mount_point.eq_ignore_ascii_case(&system_root))
            .or_else(|| {
                self.disks
                    .iter()
                    .filter(|d| !d.removable && d.total_space > 0)
                    .max_by_key(|d| d.total_space)
            })
    }

    pub fn disk_read_per_sec(&self) -> f64 {
        self.disks.iter().map(|d| d.read_bytes_per_sec).sum()
    }

    pub fn disk_write_per_sec(&self) -> f64 {
        self.disks.iter().map(|d| d.write_bytes_per_sec).sum()
    }
}

/// `part / whole` as a percentage, returning 0 when `whole` is 0.
pub fn percent(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        0.0
    } else {
        ((part as f64 / whole as f64) * 100.0).clamp(0.0, 100.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_handles_zero_and_overflow() {
        assert_eq!(percent(5, 0), 0.0);
        assert_eq!(percent(50, 200), 25.0);
        assert_eq!(percent(300, 200), 100.0);
    }

    #[test]
    fn commit_percent_requires_both_values() {
        let mut mem = MemorySnapshot {
            commit_total: Some(4),
            ..Default::default()
        };
        assert_eq!(mem.commit_percent(), None);
        mem.commit_limit = Some(8);
        assert_eq!(mem.commit_percent(), Some(50.0));
    }

    #[test]
    fn process_key_distinguishes_reused_pid() {
        let a = ProcessKey {
            pid: 42,
            start_time: 100,
        };
        let b = ProcessKey {
            pid: 42,
            start_time: 200,
        };
        assert_ne!(a, b);
    }
}
