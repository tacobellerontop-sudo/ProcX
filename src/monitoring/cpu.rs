//! CPU utilization sampling.

use sysinfo::System;

use crate::core::models::CpuSnapshot;

/// Refreshes CPU usage and frequency and builds a snapshot.
///
/// `sysinfo` computes usage from the delta since the previous refresh, so the
/// very first call after start-up reports 0%; the collector performs a
/// priming refresh before publishing its first sample.
pub fn sample(sys: &mut System) -> CpuSnapshot {
    sys.refresh_cpu_usage();
    sys.refresh_cpu_frequency();
    let cpus = sys.cpus();
    CpuSnapshot {
        total_usage: sys.global_cpu_usage().clamp(0.0, 100.0),
        per_core: cpus.iter().map(|c| c.usage().clamp(0.0, 100.0)).collect(),
        frequency_mhz: cpus.first().map(|c| c.frequency()).filter(|&f| f > 0),
    }
}
