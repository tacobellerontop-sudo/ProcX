//! Per-volume capacity and I/O throughput.

use sysinfo::{DiskKind, DiskRefreshKind, Disks};

use super::per_second;
use crate::core::models::{DiskKindLabel, DiskSnapshot, PhysicalDiskSnapshot};
use crate::platform::{self, PhysicalDiskCounters};

/// Detecting the drive kind (SSD/HDD) opens each volume, so it is refreshed
/// only every this many samples.
const FULL_REFRESH_EVERY: u32 = 30;

pub struct DiskSampler {
    disks: Disks,
    ticks: u32,
}

impl DiskSampler {
    pub fn new() -> Option<Self> {
        let disks = Disks::new_with_refreshed_list().ok()?;
        Some(Self { disks, ticks: 0 })
    }

    pub fn sample(&mut self, elapsed_secs: f64) -> Vec<DiskSnapshot> {
        let kind = if self.ticks.is_multiple_of(FULL_REFRESH_EVERY) {
            DiskRefreshKind::everything()
        } else {
            DiskRefreshKind::nothing().with_storage().with_io_usage()
        };
        self.ticks = self.ticks.wrapping_add(1);
        self.disks.refresh_specifics(true, kind);

        self.disks
            .iter()
            .map(|d| {
                let usage = d.usage();
                DiskSnapshot {
                    name: d.name().to_string_lossy().into_owned(),
                    mount_point: d.mount_point().to_string_lossy().into_owned(),
                    kind: match d.kind() {
                        DiskKind::SSD => DiskKindLabel::Ssd,
                        DiskKind::HDD => DiskKindLabel::Hdd,
                        DiskKind::Unknown(_) => DiskKindLabel::Unknown,
                    },
                    removable: d.is_removable(),
                    total_space: d.total_space(),
                    available_space: d.available_space(),
                    read_bytes_per_sec: per_second(usage.read_bytes, elapsed_secs),
                    write_bytes_per_sec: per_second(usage.written_bytes, elapsed_secs),
                }
            })
            .collect()
    }
}

/// Delta-based statistics for physical disks.
pub struct PhysicalDiskSampler {
    previous: Vec<PhysicalDiskCounters>,
    /// Indices known to exist; re-probed periodically for hot-plugged disks.
    present: Vec<u32>,
    ticks: u32,
}

/// Highest `PhysicalDriveN` index probed.
const MAX_PHYSICAL_DISKS: u32 = 32;

impl PhysicalDiskSampler {
    pub fn new() -> Self {
        Self {
            previous: Vec::new(),
            present: Vec::new(),
            ticks: 0,
        }
    }

    pub fn sample(&mut self) -> Vec<PhysicalDiskSnapshot> {
        let probe = self.ticks.is_multiple_of(FULL_REFRESH_EVERY);
        self.ticks = self.ticks.wrapping_add(1);
        let current: Vec<PhysicalDiskCounters> = if probe {
            (0..MAX_PHYSICAL_DISKS)
                .filter_map(platform::physical_disk_counters)
                .collect()
        } else {
            self.present
                .iter()
                .filter_map(|&i| platform::physical_disk_counters(i))
                .collect()
        };
        self.present = current.iter().map(|c| c.index).collect();

        let out = current
            .iter()
            .map(|now| {
                let prev = self.previous.iter().find(|p| p.index == now.index);
                physical_rates(prev, now)
            })
            .collect();
        self.previous = current;
        out
    }
}

/// Converts two counter readings into rates and active time.
pub fn physical_rates(
    prev: Option<&PhysicalDiskCounters>,
    now: &PhysicalDiskCounters,
) -> PhysicalDiskSnapshot {
    let mut snap = PhysicalDiskSnapshot {
        index: now.index,
        ..Default::default()
    };
    let Some(prev) = prev else { return snap };
    let elapsed_100ns = now.query_time.saturating_sub(prev.query_time);
    if elapsed_100ns == 0 {
        return snap;
    }
    let secs = elapsed_100ns as f64 / 1e7;
    snap.read_bytes_per_sec = per_second(now.bytes_read.saturating_sub(prev.bytes_read), secs);
    snap.write_bytes_per_sec = per_second(now.bytes_written.saturating_sub(prev.bytes_written), secs);
    let idle = now.idle_time.saturating_sub(prev.idle_time) as f64;
    snap.active_percent = Some(((1.0 - idle / elapsed_100ns as f64) * 100.0).clamp(0.0, 100.0) as f32);
    snap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_rates_and_active_time() {
        let a = PhysicalDiskCounters {
            index: 0,
            bytes_read: 0,
            bytes_written: 0,
            idle_time: 0,
            query_time: 0,
        };
        let b = PhysicalDiskCounters {
            index: 0,
            bytes_read: 2_000_000,
            bytes_written: 1_000_000,
            idle_time: 7_500_000,
            query_time: 10_000_000, // 1 s
        };
        assert_eq!(physical_rates(None, &b).active_percent, None);
        let r = physical_rates(Some(&a), &b);
        assert_eq!(r.read_bytes_per_sec, 2_000_000.0);
        assert_eq!(r.write_bytes_per_sec, 1_000_000.0);
        assert_eq!(r.active_percent, Some(25.0));
    }
}
