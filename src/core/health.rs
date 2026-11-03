//! Derives simple health indicators from measured data.
//!
//! The thresholds are intentionally conservative; they flag sustained
//! pressure, not momentary spikes.

use super::models::SystemSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Ok,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HealthIndicator {
    pub name: &'static str,
    pub severity: Severity,
    pub detail: String,
}

/// Window used for the sustained CPU check.
pub const CPU_SUSTAIN_SECS: f64 = 30.0;

fn grade(value: f32, warn: f32, crit: f32) -> Severity {
    if value >= crit {
        Severity::Critical
    } else if value >= warn {
        Severity::Warning
    } else {
        Severity::Ok
    }
}

/// Evaluates health indicators. `sustained_cpu` is the mean CPU usage over
/// [`CPU_SUSTAIN_SECS`], if enough samples exist.
pub fn evaluate(snapshot: &SystemSnapshot, sustained_cpu: Option<f64>) -> Vec<HealthIndicator> {
    let mut out = Vec::with_capacity(4);

    let cpu = sustained_cpu.map_or(snapshot.cpu.total_usage, |v| v as f32);
    out.push(HealthIndicator {
        name: "CPU load",
        severity: grade(cpu, 80.0, 95.0),
        detail: format!("{cpu:.0}% average over the last {CPU_SUSTAIN_SECS:.0}s"),
    });

    let mem = snapshot.memory.used_percent();
    out.push(HealthIndicator {
        name: "Physical memory",
        severity: grade(mem, 85.0, 95.0),
        detail: format!("{mem:.0}% in use"),
    });

    if let Some(commit) = snapshot.memory.commit_percent() {
        out.push(HealthIndicator {
            name: "Commit charge",
            severity: grade(commit, 85.0, 95.0),
            detail: format!("{commit:.0}% of the commit limit"),
        });
    }

    let fullest = snapshot
        .disks
        .iter()
        .filter(|d| !d.removable && d.total_space > 0)
        .max_by(|a, b| a.used_percent().total_cmp(&b.used_percent()));
    if let Some(disk) = fullest {
        let used = disk.used_percent();
        out.push(HealthIndicator {
            name: "Disk space",
            severity: grade(used, 90.0, 97.0),
            detail: format!("{} is {used:.0}% full", disk.mount_point),
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::*;
    use std::time::{Duration, SystemTime};

    fn snapshot() -> SystemSnapshot {
        SystemSnapshot {
            captured_at: SystemTime::now(),
            collection_time: Duration::ZERO,
            uptime: None,
            cpu: CpuSnapshot::default(),
            memory: MemorySnapshot {
                total: 100,
                used: 90,
                ..Default::default()
            },
            disks: vec![DiskSnapshot {
                mount_point: "C:\\".into(),
                total_space: 100,
                available_space: 2,
                ..Default::default()
            }],
            network: NetworkSnapshot::default(),
            counts: SystemCounts::default(),
            top_cpu: vec![],
            top_memory: vec![],
            activity: vec![],
            activity_overflow: 0,
        }
    }

    #[test]
    fn grades_thresholds() {
        let ind = evaluate(&snapshot(), Some(10.0));
        assert_eq!(ind[0].severity, Severity::Ok);
        assert_eq!(ind[1].severity, Severity::Warning);
        // No commit data -> indicator omitted rather than reported as 0.
        assert_eq!(ind[2].name, "Disk space");
        assert_eq!(ind[2].severity, Severity::Critical);
    }

    #[test]
    fn falls_back_to_instant_cpu() {
        let mut s = snapshot();
        s.cpu.total_usage = 99.0;
        assert_eq!(evaluate(&s, None)[0].severity, Severity::Critical);
    }
}
