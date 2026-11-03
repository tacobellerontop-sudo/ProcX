//! UI-side view model fed by collector events.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::core::events::CollectorEvent;
use crate::core::models::{ActivityEvent, StaticSystemInfo, SystemSnapshot};
use crate::core::series::TimeSeries;

/// Maximum number of activity entries kept in memory.
pub const ACTIVITY_CAPACITY: usize = 200;
/// Longest time range the live charts can show.
pub const HISTORY_SECS: u32 = 3_600;
/// Hard cap on samples per series, bounding memory at short intervals.
pub const MAX_SAMPLES: usize = 3_600;

pub struct DiskSeries {
    pub read: TimeSeries,
    pub write: TimeSeries,
    pub active: TimeSeries,
}

pub struct InterfaceSeries {
    pub rx: TimeSeries,
    pub tx: TimeSeries,
}

/// Live chart series.
pub struct MetricSeries {
    capacity: usize,
    pub cpu: TimeSeries,
    pub per_core: Vec<TimeSeries>,
    pub memory: TimeSeries,
    pub commit: TimeSeries,
    pub disk_read: TimeSeries,
    pub disk_write: TimeSeries,
    pub physical: BTreeMap<u32, DiskSeries>,
    pub net_rx: TimeSeries,
    pub net_tx: TimeSeries,
    pub interfaces: BTreeMap<String, InterfaceSeries>,
}

impl MetricSeries {
    fn new(capacity: usize) -> Self {
        let ts = || TimeSeries::new(capacity);
        Self {
            capacity,
            cpu: ts(),
            per_core: Vec::new(),
            memory: ts(),
            commit: ts(),
            disk_read: ts(),
            disk_write: ts(),
            physical: BTreeMap::new(),
            net_rx: ts(),
            net_tx: ts(),
            interfaces: BTreeMap::new(),
        }
    }

    fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        let fixed = [
            &mut self.cpu,
            &mut self.memory,
            &mut self.commit,
            &mut self.disk_read,
            &mut self.disk_write,
            &mut self.net_rx,
            &mut self.net_tx,
        ];
        for s in fixed {
            s.set_capacity(capacity);
        }
        for s in &mut self.per_core {
            s.set_capacity(capacity);
        }
        for d in self.physical.values_mut() {
            d.read.set_capacity(capacity);
            d.write.set_capacity(capacity);
            d.active.set_capacity(capacity);
        }
        for i in self.interfaces.values_mut() {
            i.rx.set_capacity(capacity);
            i.tx.set_capacity(capacity);
        }
    }

    fn push(&mut self, t: f64, s: &SystemSnapshot) {
        let cap = self.capacity;
        self.cpu.push(t, f64::from(s.cpu.total_usage));
        if self.per_core.len() != s.cpu.per_core.len() {
            self.per_core = (0..s.cpu.per_core.len()).map(|_| TimeSeries::new(cap)).collect();
        }
        for (series, &v) in self.per_core.iter_mut().zip(&s.cpu.per_core) {
            series.push(t, f64::from(v));
        }
        self.memory.push(t, f64::from(s.memory.used_percent()));
        if let Some(c) = s.memory.commit_total {
            self.commit.push(t, c as f64);
        }
        self.disk_read.push(t, s.disk_read_per_sec());
        self.disk_write.push(t, s.disk_write_per_sec());
        self.physical
            .retain(|i, _| s.physical_disks.iter().any(|d| d.index == *i));
        for d in &s.physical_disks {
            let e = self.physical.entry(d.index).or_insert_with(|| DiskSeries {
                read: TimeSeries::new(cap),
                write: TimeSeries::new(cap),
                active: TimeSeries::new(cap),
            });
            e.read.push(t, d.read_bytes_per_sec);
            e.write.push(t, d.write_bytes_per_sec);
            if let Some(a) = d.active_percent {
                e.active.push(t, f64::from(a));
            }
        }
        self.net_rx.push(t, s.network.received_per_sec);
        self.net_tx.push(t, s.network.transmitted_per_sec);
        self.interfaces
            .retain(|name, _| s.network.interfaces.iter().any(|i| &i.name == name));
        for i in &s.network.interfaces {
            let e = self
                .interfaces
                .entry(i.name.clone())
                .or_insert_with(|| InterfaceSeries {
                    rx: TimeSeries::new(cap),
                    tx: TimeSeries::new(cap),
                });
            e.rx.push(t, i.received_per_sec);
            e.tx.push(t, i.transmitted_per_sec);
        }
    }
}

/// Number of samples needed to cover the full history at `interval_ms`.
pub fn series_capacity(interval_ms: u64) -> usize {
    let interval_ms = interval_ms.max(1);
    ((u64::from(HISTORY_SECS) * 1000).div_ceil(interval_ms) as usize + 1).min(MAX_SAMPLES)
}

/// Seconds of history actually retained at `interval_ms`.
pub fn retained_secs(interval_ms: u64) -> f64 {
    (series_capacity(interval_ms) - 1) as f64 * interval_ms as f64 / 1000.0
}

pub struct LiveState {
    pub info: Option<StaticSystemInfo>,
    pub latest: Option<Box<SystemSnapshot>>,
    pub series: MetricSeries,
    /// Newest first.
    pub activity: VecDeque<ActivityEvent>,
    /// Activity events not shown because too many occurred at once.
    pub activity_dropped: usize,
    pub warnings: Vec<String>,
    last_update: Option<Instant>,
    last_sample_t: f64,
    epoch: Instant,
}

impl LiveState {
    pub fn new(interval_ms: u64) -> Self {
        Self {
            info: None,
            latest: None,
            series: MetricSeries::new(series_capacity(interval_ms)),
            activity: VecDeque::with_capacity(ACTIVITY_CAPACITY),
            activity_dropped: 0,
            warnings: Vec::new(),
            last_update: None,
            last_sample_t: 0.0,
            epoch: Instant::now(),
        }
    }

    pub fn resize(&mut self, interval_ms: u64) {
        self.series.set_capacity(series_capacity(interval_ms));
    }

    /// Applies an event. Returns true if it was a new snapshot.
    pub fn ingest(&mut self, event: CollectorEvent) -> bool {
        match event {
            CollectorEvent::StaticInfo(info) => {
                self.info = Some(*info);
                false
            }
            CollectorEvent::Warning(w) => {
                tracing::warn!("{w}");
                self.warnings.push(w);
                false
            }
            CollectorEvent::Snapshot(mut snapshot) => {
                let t = self.epoch.elapsed().as_secs_f64();
                self.last_sample_t = t;
                self.last_update = Some(Instant::now());
                self.series.push(t, &snapshot);
                crate::utils::time::set_utc_offset(snapshot.utc_offset_secs);
                for event in snapshot.activity.drain(..) {
                    if self.activity.len() == ACTIVITY_CAPACITY {
                        self.activity.pop_back();
                    }
                    self.activity.push_front(event);
                }
                self.activity_dropped += snapshot.activity_overflow;
                self.latest = Some(snapshot);
                true
            }
        }
    }

    /// Time coordinate of the newest sample; charts place it at x = 0.
    pub fn chart_now(&self) -> f64 {
        self.last_sample_t
    }

    /// True when no sample arrived for several intervals.
    pub fn is_stale(&self, interval: Duration) -> bool {
        self.last_update
            .is_none_or(|t| t.elapsed() > interval * 3 + Duration::from_secs(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_covers_history_with_cap() {
        assert_eq!(series_capacity(1000), 3_600);
        assert_eq!(series_capacity(2000), 1_801);
        assert_eq!(series_capacity(250), MAX_SAMPLES);
        assert_eq!(retained_secs(2000), 3_600.0);
        assert_eq!(retained_secs(250), 899.75);
    }
}
