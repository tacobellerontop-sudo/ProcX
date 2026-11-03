//! Process-level summaries for the dashboard: top consumers and
//! start/exit activity.
//!
//! The process list itself comes from the platform layer; this module adds
//! CPU utilization (from CPU-time deltas) and derived dashboards data.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::core::models::{ActivityEvent, ActivityKind, ProcessInfo, ProcessKey, ProcessSummary};
use crate::platform::ProcessEnumerator;

/// Number of processes listed in each top-N table.
pub const TOP_N: usize = 8;
/// At most this many activity events are reported per sample; the rest are counted.
pub const MAX_ACTIVITY_PER_SAMPLE: usize = 64;

/// Detects process starts and exits by diffing process instance keys
/// between consecutive samples.
#[derive(Default)]
pub struct ActivityTracker {
    known: HashMap<ProcessKey, String>,
    seen: HashSet<ProcessKey>,
    primed: bool,
}

impl ActivityTracker {
    /// Records the current process set and returns what changed since the
    /// previous call, plus the number of events dropped by the per-sample cap.
    /// The first call only establishes a baseline.
    pub fn observe<'a>(
        &mut self,
        current: impl Iterator<Item = (ProcessKey, &'a str)>,
        now: SystemTime,
    ) -> (Vec<ActivityEvent>, usize) {
        let mut events = Vec::new();
        let mut overflow = 0usize;
        let mut push = |event: ActivityEvent| {
            if events.len() < MAX_ACTIVITY_PER_SAMPLE {
                events.push(event);
            } else {
                overflow += 1;
            }
        };

        self.seen.clear();
        for (key, name) in current {
            self.seen.insert(key);
            if let Entry::Vacant(slot) = self.known.entry(key) {
                slot.insert(name.to_owned());
                if self.primed {
                    push(ActivityEvent {
                        at: now,
                        kind: ActivityKind::ProcessStarted,
                        key,
                        name: name.to_owned(),
                    });
                }
            }
        }

        let seen = &self.seen;
        let primed = self.primed;
        self.known.retain(|key, name| {
            let alive = seen.contains(key);
            if !alive && primed {
                push(ActivityEvent {
                    at: now,
                    kind: ActivityKind::ProcessExited,
                    key: *key,
                    name: std::mem::take(name),
                });
            }
            alive
        });

        self.primed = true;
        (events, overflow)
    }
}

/// Converts cumulative CPU time into utilization by diffing consecutive samples.
#[derive(Default)]
pub struct CpuTracker {
    previous: HashMap<ProcessKey, Duration>,
    current: HashMap<ProcessKey, Duration>,
}

impl CpuTracker {
    /// Returns utilization (0..=100, normalized to all logical CPUs) for a
    /// process given its cumulative CPU time. `None` on the first sighting.
    pub fn observe(
        &mut self,
        key: ProcessKey,
        cpu_time: Duration,
        elapsed: Duration,
        logical_cpus: f32,
    ) -> Option<f32> {
        self.current.insert(key, cpu_time);
        let prev = *self.previous.get(&key)?;
        let wall = elapsed.as_secs_f64() * f64::from(logical_cpus);
        if wall <= 0.0 {
            return None;
        }
        let used = cpu_time.saturating_sub(prev).as_secs_f64();
        Some(((used / wall) * 100.0).clamp(0.0, 100.0) as f32)
    }

    /// Ends a sample; processes not observed are forgotten.
    pub fn finish(&mut self) {
        std::mem::swap(&mut self.previous, &mut self.current);
        self.current.clear();
    }
}

/// Selects the `n` largest entries by `metric` and returns them in descending order.
pub fn top_by<T, F>(items: &mut Vec<T>, n: usize, metric: F)
where
    F: Fn(&T) -> f64,
{
    items.sort_unstable_by(|a, b| metric(b).total_cmp(&metric(a)));
    items.truncate(n);
}

pub struct ProcessSample {
    pub processes: Arc<[ProcessInfo]>,
    pub top_cpu: Vec<ProcessSummary>,
    pub top_memory: Vec<ProcessSummary>,
    pub activity: Vec<ActivityEvent>,
    pub activity_overflow: usize,
    pub error: Option<String>,
}

pub struct ProcessSampler {
    enumerator: ProcessEnumerator,
    activity: ActivityTracker,
    cpu: CpuTracker,
    logical_cpus: f32,
    last: Option<Instant>,
    capacity_hint: usize,
}

fn summarize(p: &ProcessInfo) -> ProcessSummary {
    ProcessSummary {
        key: p.key,
        name: p.name.to_string(),
        cpu_percent: p.cpu_percent.unwrap_or(0.0),
        memory_bytes: p.working_set.unwrap_or(0),
    }
}

impl ProcessSampler {
    pub fn new(logical_cpus: usize) -> Self {
        Self {
            enumerator: ProcessEnumerator::new(),
            activity: ActivityTracker::default(),
            cpu: CpuTracker::default(),
            logical_cpus: logical_cpus.max(1) as f32,
            last: None,
            capacity_hint: 256,
        }
    }

    pub fn sample(&mut self, now: SystemTime) -> ProcessSample {
        let started = Instant::now();
        let elapsed = self.last.map(|l| started.duration_since(l)).unwrap_or_default();
        self.last = Some(started);

        let mut list = Vec::with_capacity(self.capacity_hint);
        let error = self.enumerator.enumerate(&mut list).err();
        self.capacity_hint = list.len() + 32;

        for p in &mut list {
            if let Some(t) = p.cpu_time {
                p.cpu_percent = self.cpu.observe(p.key, t, elapsed, self.logical_cpus);
            }
        }
        self.cpu.finish();

        let (activity, activity_overflow) =
            self.activity.observe(list.iter().map(|p| (p.key, &*p.name)), now);

        // PID 0 is the "System Idle Process"; it never consumes real CPU.
        let mut by_cpu: Vec<&ProcessInfo> = list.iter().filter(|p| p.key.pid != 0).collect();
        top_by(&mut by_cpu, TOP_N, |p| f64::from(p.cpu_percent.unwrap_or(0.0)));
        let mut by_mem: Vec<&ProcessInfo> = list.iter().collect();
        top_by(&mut by_mem, TOP_N, |p| p.working_set.unwrap_or(0) as f64);
        let top_cpu = by_cpu.into_iter().map(summarize).collect();
        let top_memory = by_mem.into_iter().map(summarize).collect();

        ProcessSample {
            processes: Arc::from(list),
            top_cpu,
            top_memory,
            activity,
            activity_overflow,
            error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pid: u32, start: u64) -> ProcessKey {
        ProcessKey {
            pid,
            start_time: start,
        }
    }

    #[test]
    fn first_observation_is_baseline() {
        let mut t = ActivityTracker::default();
        let (events, _) = t.observe([(key(1, 1), "a")].into_iter(), SystemTime::now());
        assert!(events.is_empty());
    }

    #[test]
    fn detects_start_exit_and_pid_reuse() {
        let mut t = ActivityTracker::default();
        let now = SystemTime::now();
        t.observe([(key(1, 1), "a"), (key(2, 1), "b")].into_iter(), now);

        // PID 2 exited and was reused by a new process "c".
        let (events, overflow) = t.observe([(key(1, 1), "a"), (key(2, 5), "c")].into_iter(), now);
        assert_eq!(overflow, 0);
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .any(|e| e.kind == ActivityKind::ProcessStarted && e.name == "c")
        );
        assert!(
            events
                .iter()
                .any(|e| e.kind == ActivityKind::ProcessExited && e.name == "b")
        );
    }

    #[test]
    fn caps_events_per_sample() {
        let mut t = ActivityTracker::default();
        let now = SystemTime::now();
        t.observe(std::iter::empty(), now);
        let many: Vec<_> = (0..100).map(|i| (key(i, 0), "x")).collect();
        let (events, overflow) = t.observe(many.into_iter(), now);
        assert_eq!(events.len(), MAX_ACTIVITY_PER_SAMPLE);
        assert_eq!(overflow, 100 - MAX_ACTIVITY_PER_SAMPLE);
    }

    #[test]
    fn cpu_from_time_deltas() {
        let mut t = CpuTracker::default();
        let k = key(1, 1);
        let sec = Duration::from_secs(1);
        assert_eq!(t.observe(k, Duration::from_millis(100), sec, 2.0), None);
        t.finish();
        // 500 ms of CPU over 1 s on 2 logical CPUs = 25 %.
        assert_eq!(t.observe(k, Duration::from_millis(600), sec, 2.0), Some(25.0));
        t.finish();
        // A process not seen in a sample is forgotten.
        t.finish();
        assert_eq!(t.observe(k, Duration::from_millis(700), sec, 2.0), None);
    }

    #[test]
    fn top_by_orders_descending() {
        let mut v = vec![3.0, 9.0, 1.0, 7.0];
        top_by(&mut v, 2, |x| *x);
        assert_eq!(v, vec![9.0, 7.0]);
    }
}
