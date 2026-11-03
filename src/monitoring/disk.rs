//! Per-volume capacity and I/O throughput.

use sysinfo::{DiskKind, DiskRefreshKind, Disks};

use super::per_second;
use crate::core::models::{DiskKindLabel, DiskSnapshot};

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
