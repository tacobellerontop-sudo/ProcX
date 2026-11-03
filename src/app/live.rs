//! UI-side view model fed by collector events.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::core::events::CollectorEvent;
use crate::core::models::{ActivityEvent, StaticSystemInfo, SystemSnapshot};
use crate::core::series::TimeSeries;

/// Maximum number of activity entries kept in memory.
pub const ACTIVITY_CAPACITY: usize = 200;

/// Live chart series.
pub struct MetricSeries {
    pub cpu: TimeSeries,
    pub memory: TimeSeries,
    pub disk_read: TimeSeries,
    pub disk_write: TimeSeries,
    pub net_rx: TimeSeries,
    pub net_tx: TimeSeries,
}

impl MetricSeries {
    fn new(capacity: usize) -> Self {
        Self {
            cpu: TimeSeries::new(capacity),
            memory: TimeSeries::new(capacity),
            disk_read: TimeSeries::new(capacity),
            disk_write: TimeSeries::new(capacity),
            net_rx: TimeSeries::new(capacity),
            net_tx: TimeSeries::new(capacity),
        }
    }

    fn all_mut(&mut self) -> [&mut TimeSeries; 6] {
        [
            &mut self.cpu,
            &mut self.memory,
            &mut self.disk_read,
            &mut self.disk_write,
            &mut self.net_rx,
            &mut self.net_tx,
        ]
    }
}

/// Number of samples needed to cover `window_secs` at `interval_ms`.
pub fn series_capacity(window_secs: u32, interval_ms: u64) -> usize {
    let interval_ms = interval_ms.max(1);
    (u64::from(window_secs) * 1000).div_ceil(interval_ms) as usize + 1
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
    pub fn new(window_secs: u32, interval_ms: u64) -> Self {
        Self {
            info: None,
            latest: None,
            series: MetricSeries::new(series_capacity(window_secs, interval_ms)),
            activity: VecDeque::with_capacity(ACTIVITY_CAPACITY),
            activity_dropped: 0,
            warnings: Vec::new(),
            last_update: None,
            last_sample_t: 0.0,
            epoch: Instant::now(),
        }
    }

    pub fn resize(&mut self, window_secs: u32, interval_ms: u64) {
        let capacity = series_capacity(window_secs, interval_ms);
        for s in self.series.all_mut() {
            s.set_capacity(capacity);
        }
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
                let s = &mut self.series;
                s.cpu.push(t, f64::from(snapshot.cpu.total_usage));
                s.memory.push(t, f64::from(snapshot.memory.used_percent()));
                s.disk_read.push(t, snapshot.disk_read_per_sec());
                s.disk_write.push(t, snapshot.disk_write_per_sec());
                s.net_rx.push(t, snapshot.network.received_per_sec);
                s.net_tx.push(t, snapshot.network.transmitted_per_sec);

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
    fn capacity_covers_window() {
        assert_eq!(series_capacity(120, 1000), 121);
        assert_eq!(series_capacity(60, 250), 241);
        assert_eq!(series_capacity(30, 0), 30_001);
    }
}
