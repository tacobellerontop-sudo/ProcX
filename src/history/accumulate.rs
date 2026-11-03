//! Folds live collector ticks into one history record per recording interval.
//!
//! System values are averaged over the interval (CPU also keeps its peak).
//! Per-process CPU is averaged over the ticks the process was present;
//! memory keeps the peak. Every process seen is recorded for lifetime
//! tracking, but per-process samples are kept only for the busiest
//! processes, which bounds database growth.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use crate::core::models::{ProcessInfo, ProcessKey, SystemSnapshot};

/// Processes sampled per record: the top N by CPU plus the top N by memory.
pub const TOP_PER_RECORD: usize = 15;

/// The part of a collector snapshot that history needs. Cheap to build:
/// the process list is shared, not copied.
#[derive(Debug, Clone)]
pub struct HistoryTick {
    pub at: SystemTime,
    pub cpu: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub commit: Option<u64>,
    pub disk_read: f64,
    pub disk_write: f64,
    pub net_rx: f64,
    pub net_tx: f64,
    pub processes: Arc<[ProcessInfo]>,
}

impl HistoryTick {
    pub fn from_snapshot(s: &SystemSnapshot) -> Self {
        // Physical disk counters where available (Windows), else per-volume rates.
        let (disk_read, disk_write) = if s.physical_disks.is_empty() {
            s.disks.iter().fold((0.0, 0.0), |(r, w), d| {
                (r + d.read_bytes_per_sec, w + d.write_bytes_per_sec)
            })
        } else {
            s.physical_disks.iter().fold((0.0, 0.0), |(r, w), d| {
                (r + d.read_bytes_per_sec, w + d.write_bytes_per_sec)
            })
        };
        Self {
            at: s.captured_at,
            cpu: s.cpu.total_usage,
            mem_used: s.memory.used,
            mem_total: s.memory.total,
            commit: s.memory.commit_total,
            disk_read,
            disk_write,
            net_rx: s.network.received_per_sec,
            net_tx: s.network.transmitted_per_sec,
            processes: s.processes.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SystemRecord {
    pub ts_ms: i64,
    pub cpu_avg: f32,
    pub cpu_max: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub commit: Option<u64>,
    pub disk_read: f64,
    pub disk_write: f64,
    pub net_rx: f64,
    pub net_tx: f64,
    pub process_count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessRecord {
    pub key: ProcessKey,
    pub name: String,
    pub exe_path: Option<String>,
    pub user: Option<String>,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    /// `None` when CPU usage was never measurable (e.g. access denied).
    pub cpu_avg: Option<f32>,
    pub working_set_max: Option<u64>,
    pub private_max: Option<u64>,
    /// Whether a per-process sample row is stored (top consumers only).
    pub sampled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub system: SystemRecord,
    pub processes: Vec<ProcessRecord>,
}

pub fn unix_ms(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

struct ProcAcc {
    name: String,
    exe_path: Option<String>,
    user: Option<String>,
    first_seen_ms: i64,
    last_seen_ms: i64,
    cpu_sum: f64,
    cpu_n: u32,
    ws_max: Option<u64>,
    private_max: Option<u64>,
}

#[derive(Default)]
pub struct Accumulator {
    ticks: u32,
    cpu_sum: f64,
    cpu_max: f32,
    mem_sum: f64,
    disk_read_sum: f64,
    disk_write_sum: f64,
    net_rx_sum: f64,
    net_tx_sum: f64,
    last_at: Option<SystemTime>,
    last_mem_total: u64,
    last_commit: Option<u64>,
    last_count: u32,
    procs: HashMap<ProcessKey, ProcAcc>,
}

fn max_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

impl Accumulator {
    pub fn is_empty(&self) -> bool {
        self.ticks == 0
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn add(&mut self, tick: &HistoryTick) {
        self.ticks += 1;
        self.cpu_sum += f64::from(tick.cpu);
        self.cpu_max = self.cpu_max.max(tick.cpu);
        self.mem_sum += tick.mem_used as f64;
        self.disk_read_sum += tick.disk_read;
        self.disk_write_sum += tick.disk_write;
        self.net_rx_sum += tick.net_rx;
        self.net_tx_sum += tick.net_tx;
        self.last_at = Some(tick.at);
        self.last_mem_total = tick.mem_total;
        self.last_commit = tick.commit;
        self.last_count = u32::try_from(tick.processes.len()).unwrap_or(u32::MAX);
        let now_ms = unix_ms(tick.at);
        for p in tick.processes.iter() {
            let acc = self.procs.entry(p.key).or_insert_with(|| ProcAcc {
                name: p.name.to_string(),
                exe_path: p.exe_path.as_deref().map(str::to_owned),
                user: p.user.as_deref().map(str::to_owned),
                first_seen_ms: now_ms,
                last_seen_ms: now_ms,
                cpu_sum: 0.0,
                cpu_n: 0,
                ws_max: None,
                private_max: None,
            });
            acc.last_seen_ms = now_ms;
            if acc.exe_path.is_none() {
                acc.exe_path = p.exe_path.as_deref().map(str::to_owned);
            }
            if acc.user.is_none() {
                acc.user = p.user.as_deref().map(str::to_owned);
            }
            if let Some(c) = p.cpu_percent {
                acc.cpu_sum += f64::from(c);
                acc.cpu_n += 1;
            }
            acc.ws_max = max_opt(acc.ws_max, p.working_set);
            acc.private_max = max_opt(acc.private_max, p.private_bytes);
        }
    }

    /// Produces the record for the interval and resets. `None` if no tick arrived.
    pub fn take(&mut self, top_n: usize) -> Option<Record> {
        let at = self.last_at?;
        if self.ticks == 0 {
            return None;
        }
        let n = f64::from(self.ticks);
        let system = SystemRecord {
            ts_ms: unix_ms(at),
            cpu_avg: (self.cpu_sum / n) as f32,
            cpu_max: self.cpu_max,
            mem_used: (self.mem_sum / n) as u64,
            mem_total: self.last_mem_total,
            commit: self.last_commit,
            disk_read: self.disk_read_sum / n,
            disk_write: self.disk_write_sum / n,
            net_rx: self.net_rx_sum / n,
            net_tx: self.net_tx_sum / n,
            process_count: self.last_count,
        };
        let mut processes: Vec<ProcessRecord> = self
            .procs
            .drain()
            .map(|(key, a)| ProcessRecord {
                key,
                name: a.name,
                exe_path: a.exe_path,
                user: a.user,
                first_seen_ms: a.first_seen_ms,
                last_seen_ms: a.last_seen_ms,
                cpu_avg: (a.cpu_n > 0).then(|| (a.cpu_sum / f64::from(a.cpu_n)) as f32),
                working_set_max: a.ws_max,
                private_max: a.private_max,
                sampled: false,
            })
            .collect();
        mark_top(&mut processes, top_n, |p| p.cpu_avg.map(f64::from));
        mark_top(&mut processes, top_n, |p| p.working_set_max.map(|v| v as f64));
        self.clear();
        Some(Record { system, processes })
    }
}

/// Marks the `n` records with the largest metric as sampled. Records without
/// the metric, or with a zero value, are never chosen by it.
fn mark_top(records: &mut [ProcessRecord], n: usize, metric: impl Fn(&ProcessRecord) -> Option<f64>) {
    let mut order: Vec<(usize, f64)> = records
        .iter()
        .enumerate()
        .filter_map(|(i, r)| metric(r).filter(|v| *v > 0.0).map(|v| (i, v)))
        .collect();
    order.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (i, _) in order.into_iter().take(n) {
        records[i].sampled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn proc(pid: u32, cpu: Option<f32>, ws: Option<u64>) -> ProcessInfo {
        let mut p = ProcessInfo::new(ProcessKey { pid, start_time: 100 }, format!("p{pid}").as_str());
        p.cpu_percent = cpu;
        p.working_set = ws;
        p
    }

    fn tick(secs: u64, cpu: f32, procs: Vec<ProcessInfo>) -> HistoryTick {
        HistoryTick {
            at: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
            cpu,
            mem_used: 1000,
            mem_total: 4000,
            commit: None,
            disk_read: 10.0,
            disk_write: 0.0,
            net_rx: 0.0,
            net_tx: 0.0,
            processes: procs.into(),
        }
    }

    #[test]
    fn averages_and_peaks() {
        let mut acc = Accumulator::default();
        assert!(acc.take(5).is_none());
        acc.add(&tick(
            10,
            20.0,
            vec![proc(1, Some(10.0), Some(100)), proc(2, None, None)],
        ));
        acc.add(&tick(
            11,
            40.0,
            vec![proc(1, Some(30.0), Some(300)), proc(3, Some(1.0), Some(5))],
        ));
        let r = acc.take(5).unwrap();
        assert_eq!(r.system.ts_ms, 11_000);
        assert_eq!(r.system.cpu_avg, 30.0);
        assert_eq!(r.system.cpu_max, 40.0);
        assert_eq!(r.system.process_count, 2);
        let p1 = r.processes.iter().find(|p| p.key.pid == 1).unwrap();
        assert_eq!(p1.cpu_avg, Some(20.0));
        assert_eq!(p1.working_set_max, Some(300));
        assert_eq!((p1.first_seen_ms, p1.last_seen_ms), (10_000, 11_000));
        let p2 = r.processes.iter().find(|p| p.key.pid == 2).unwrap();
        assert_eq!(p2.cpu_avg, None, "unmeasured CPU must stay unknown, not zero");
        assert!(!p2.sampled);
        assert!(acc.is_empty());
    }

    #[test]
    fn samples_only_top_consumers() {
        let mut acc = Accumulator::default();
        let procs = (1..=10)
            .map(|i| proc(i, Some(i as f32), Some(u64::from(100 - i))))
            .collect();
        acc.add(&tick(1, 1.0, procs));
        let r = acc.take(2).unwrap();
        let mut sampled: Vec<u32> = r
            .processes
            .iter()
            .filter(|p| p.sampled)
            .map(|p| p.key.pid)
            .collect();
        sampled.sort_unstable();
        // Top 2 by CPU (9, 10) and top 2 by memory (1, 2).
        assert_eq!(sampled, vec![1, 2, 9, 10]);
        assert_eq!(r.processes.len(), 10);
    }
}
