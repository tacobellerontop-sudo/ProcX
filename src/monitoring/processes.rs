//! Process-level summaries for the dashboard: top consumers and
//! start/exit activity.
//!
//! The full process explorer (Phase 2) builds on the same refresh but needs
//! richer per-process metadata from the platform layer.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::core::models::{ActivityEvent, ActivityKind, ProcessKey, ProcessSummary};

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

fn key_of(pid: Pid, process: &Process) -> ProcessKey {
    ProcessKey {
        pid: pid.as_u32(),
        start_time: process.start_time(),
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
    pub count: usize,
    pub top_cpu: Vec<ProcessSummary>,
    pub top_memory: Vec<ProcessSummary>,
    pub activity: Vec<ActivityEvent>,
    pub activity_overflow: usize,
}

pub struct ProcessSampler {
    activity: ActivityTracker,
    logical_cpus: f32,
}

impl ProcessSampler {
    pub fn new(logical_cpus: usize) -> Self {
        Self {
            activity: ActivityTracker::default(),
            logical_cpus: logical_cpus.max(1) as f32,
        }
    }

    pub fn sample(&mut self, sys: &mut System, now: SystemTime) -> ProcessSample {
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        // On Linux, sysinfo may list threads alongside processes; Windows never does.
        let processes = || sys.processes().iter().filter(|(_, p)| p.thread_kind().is_none());

        let (activity, activity_overflow) = self.activity.observe(
            processes().map(|(&pid, p)| (key_of(pid, p), p.name().to_str().unwrap_or("?"))),
            now,
        );

        let summarize = |(pid, p): (&Pid, &Process)| ProcessSummary {
            key: key_of(*pid, p),
            name: p.name().to_string_lossy().into_owned(),
            cpu_percent: (p.cpu_usage() / self.logical_cpus).clamp(0.0, 100.0),
            memory_bytes: p.memory(),
        };

        // PID 0 is the "System Idle Process" on Windows; its "CPU usage" is idle time.
        let mut by_cpu: Vec<_> = processes().filter(|(pid, _)| pid.as_u32() != 0).collect();
        top_by(&mut by_cpu, TOP_N, |(_, p)| f64::from(p.cpu_usage()));
        let mut by_mem: Vec<_> = processes().collect();
        let count = by_mem.len();
        top_by(&mut by_mem, TOP_N, |(_, p)| p.memory() as f64);

        ProcessSample {
            count,
            top_cpu: by_cpu.into_iter().map(summarize).collect(),
            top_memory: by_mem.into_iter().map(summarize).collect(),
            activity,
            activity_overflow,
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
    fn top_by_orders_descending() {
        let mut v = vec![3.0, 9.0, 1.0, 7.0];
        top_by(&mut v, 2, |x| *x);
        assert_eq!(v, vec![9.0, 7.0]);
    }
}
