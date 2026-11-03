//! Background collection worker.
//!
//! A single dedicated thread owns all samplers, takes a snapshot every
//! interval and publishes it through a bounded channel. The UI thread only
//! drains that channel, so slow system queries can never stall rendering.
//!
//! Only one collector may run at a time (enforced with a process-wide flag),
//! and dropping the [`Collector`] stops and joins the thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use super::disk::{DiskSampler, PhysicalDiskSampler};
use super::network::NetworkSampler;
use super::processes::ProcessSampler;
use super::{cpu, memory};
use crate::core::config::{MAX_REFRESH_MS, MIN_REFRESH_MS};
use crate::core::errors::{ProcxError, Result};
use crate::core::events::{CollectorCommand, CollectorEvent};
use crate::core::models::{StaticSystemInfo, SystemCounts, SystemSnapshot};
use crate::history::accumulate::HistoryTick;
use crate::history::worker::TickSender;
use crate::platform;

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Snapshots buffered for the UI. If the UI falls behind (e.g. while the
/// window is minimized), newer snapshots are dropped instead of piling up.
const EVENT_BUFFER: usize = 4;

/// Callback used to wake the UI after an event is published.
pub type Notifier = Arc<dyn Fn() + Send + Sync>;

pub struct Collector {
    commands: Sender<CollectorCommand>,
    events: Receiver<CollectorEvent>,
    thread: Option<JoinHandle<()>>,
}

/// Resets the running flag when the worker exits, even by panic.
struct RunningGuard;

impl Drop for RunningGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

pub fn clamp_interval(interval: Duration) -> Duration {
    interval.clamp(
        Duration::from_millis(MIN_REFRESH_MS),
        Duration::from_millis(MAX_REFRESH_MS),
    )
}

impl Collector {
    /// Starts the collector thread.
    ///
    /// Fails with [`ProcxError::WorkerAlreadyRunning`] if another collector
    /// is still alive, which prevents duplicate workers.
    pub fn spawn(interval: Duration, notify: Notifier, history: Option<TickSender>) -> Result<Self> {
        if RUNNING.swap(true, Ordering::SeqCst) {
            return Err(ProcxError::WorkerAlreadyRunning("collector"));
        }
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (evt_tx, evt_rx) = crossbeam_channel::bounded(EVENT_BUFFER);

        let spawned = std::thread::Builder::new()
            .name("procx-collector".into())
            .spawn(move || {
                let _guard = RunningGuard;
                Worker::run(
                    clamp_interval(interval),
                    &cmd_rx,
                    &evt_tx,
                    &notify,
                    history.as_ref(),
                );
                tracing::info!("collector stopped");
            });

        match spawned {
            Ok(thread) => Ok(Self {
                commands: cmd_tx,
                events: evt_rx,
                thread: Some(thread),
            }),
            Err(err) => {
                RUNNING.store(false, Ordering::SeqCst);
                Err(ProcxError::WorkerSpawn(err))
            }
        }
    }

    pub fn send(&self, command: CollectorCommand) {
        // The worker only disconnects when it has stopped, in which case
        // there is nothing left to command.
        let _ = self.commands.send(command);
    }

    /// Returns all events received since the last call without blocking.
    pub fn drain(&self) -> impl Iterator<Item = CollectorEvent> + '_ {
        self.events.try_iter()
    }

    pub fn is_alive(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Stops the worker and waits for it to exit.
    pub fn shutdown(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.commands.send(CollectorCommand::Shutdown);
            if thread.join().is_err() {
                tracing::error!("collector thread panicked");
            }
        }
    }
}

impl Drop for Collector {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Worker {
    sys: System,
    disks: Option<DiskSampler>,
    physical_disks: PhysicalDiskSampler,
    networks: Option<NetworkSampler>,
    processes: ProcessSampler,
    last_sample: Instant,
}

impl Worker {
    fn new() -> std::result::Result<(Self, StaticSystemInfo), sysinfo::Error> {
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        )?;
        let logical_cpus = sys.cpus().len();
        let info = StaticSystemInfo {
            host_name: System::host_name().ok(),
            os_version: System::long_os_version().ok(),
            kernel_version: System::kernel_version().ok(),
            cpu_brand: sys
                .cpus()
                .first()
                .map(|c| c.brand().trim().to_owned())
                .filter(|b| !b.is_empty()),
            cpu_arch: System::cpu_arch(),
            logical_cpus,
            physical_cores: System::physical_core_count().ok(),
            elevated: platform::is_elevated(),
        };

        let mut processes = ProcessSampler::new(logical_cpus);
        // Prime the delta-based counters (CPU, per-process CPU, disk and
        // network I/O) so that the first published sample is meaningful.
        processes.sample(SystemTime::now());
        sys.refresh_cpu_usage();

        Ok((
            Self {
                sys,
                disks: DiskSampler::new(),
                physical_disks: PhysicalDiskSampler::new(),
                networks: NetworkSampler::new(),
                processes,
                last_sample: Instant::now(),
            },
            info,
        ))
    }

    fn run(
        mut interval: Duration,
        commands: &Receiver<CollectorCommand>,
        events: &Sender<CollectorEvent>,
        notify: &Notifier,
        history: Option<&TickSender>,
    ) {
        let publish = |event: CollectorEvent| -> bool {
            match events.try_send(event) {
                Ok(()) => {
                    notify();
                    true
                }
                Err(TrySendError::Full(_)) => {
                    tracing::trace!("UI is behind; dropping a collector event");
                    true
                }
                Err(TrySendError::Disconnected(_)) => false,
            }
        };

        let (mut worker, info) = match Worker::new() {
            Ok(v) => v,
            Err(err) => {
                tracing::error!("collector initialization failed: {err}");
                publish(CollectorEvent::Warning(format!(
                    "System monitoring could not start: {err}"
                )));
                return;
            }
        };
        if worker.disks.is_none() {
            publish(CollectorEvent::Warning("Disk information is unavailable".into()));
        }
        if worker.networks.is_none() {
            publish(CollectorEvent::Warning(
                "Network information is unavailable".into(),
            ));
        }
        if !publish(CollectorEvent::StaticInfo(Box::new(info))) {
            return;
        }
        tracing::info!(interval_ms = interval.as_millis() as u64, "collector started");

        loop {
            let due = worker.last_sample + interval;
            let wait = due.saturating_duration_since(Instant::now());
            match commands.recv_timeout(wait) {
                Ok(CollectorCommand::SetInterval(new)) => {
                    interval = clamp_interval(new);
                    tracing::debug!(interval_ms = interval.as_millis() as u64, "interval changed");
                    continue;
                }
                Ok(CollectorCommand::RefreshNow) => {
                    // Usage deltas need a minimum gap to be meaningful.
                    let min_gap = Duration::from_millis(MIN_REFRESH_MS);
                    let since = worker.last_sample.elapsed();
                    if since < min_gap {
                        std::thread::sleep(min_gap - since);
                    }
                }
                Ok(CollectorCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }

            let snapshot = worker.sample();
            if let Some(h) = history {
                h.send(HistoryTick::from_snapshot(&snapshot));
            }
            if !publish(CollectorEvent::Snapshot(Box::new(snapshot))) {
                break;
            }
        }
    }

    fn sample(&mut self) -> SystemSnapshot {
        let started = Instant::now();
        let elapsed = started.duration_since(self.last_sample).as_secs_f64();
        self.last_sample = started;
        let now = SystemTime::now();

        let kernel = platform::kernel_counters();
        let cpu = cpu::sample(&mut self.sys);
        let memory = memory::sample(&mut self.sys, kernel.as_ref());
        let processes = self.processes.sample(now);
        if let Some(err) = &processes.error {
            tracing::warn!("process enumeration failed: {err}");
        }
        let disks = self.disks.as_mut().map(|d| d.sample(elapsed)).unwrap_or_default();
        let physical_disks = self.physical_disks.sample();
        let network = self
            .networks
            .as_mut()
            .map(|n| n.sample(elapsed))
            .unwrap_or_default();

        SystemSnapshot {
            captured_at: now,
            collection_time: started.elapsed(),
            uptime: System::uptime().ok().map(Duration::from_secs),
            cpu,
            memory,
            disks,
            physical_disks,
            network,
            counts: SystemCounts {
                processes: kernel.map_or(processes.processes.len(), |k| k.process_count as usize),
                threads: kernel.map(|k| k.thread_count),
                handles: kernel.map(|k| k.handle_count),
            },
            top_cpu: processes.top_cpu,
            top_memory: processes.top_memory,
            processes: processes.processes,
            activity: processes.activity,
            activity_overflow: processes.activity_overflow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_is_clamped() {
        assert_eq!(
            clamp_interval(Duration::ZERO),
            Duration::from_millis(MIN_REFRESH_MS)
        );
        assert_eq!(
            clamp_interval(Duration::from_secs(3600)),
            Duration::from_millis(MAX_REFRESH_MS)
        );
    }

    /// Exercises the real worker end to end against the host system and
    /// checks the single-instance guard. Runs as one test because the guard
    /// is process-wide.
    #[test]
    fn collector_produces_snapshots_and_rejects_duplicates() {
        let mut collector =
            Collector::spawn(Duration::from_millis(MIN_REFRESH_MS), Arc::new(|| {}), None).unwrap();
        assert!(matches!(
            Collector::spawn(Duration::from_secs(1), Arc::new(|| {}), None),
            Err(ProcxError::WorkerAlreadyRunning(_))
        ));

        let deadline = Instant::now() + Duration::from_secs(20);
        let (mut got_info, mut got_snapshot) = (false, false);
        while Instant::now() < deadline && !(got_info && got_snapshot) {
            match collector.events.recv_timeout(Duration::from_secs(1)) {
                Ok(CollectorEvent::StaticInfo(info)) => {
                    assert!(info.logical_cpus > 0);
                    got_info = true;
                }
                Ok(CollectorEvent::Snapshot(s)) => {
                    assert!(s.memory.total > 0);
                    assert!(s.counts.processes > 0);
                    assert!(!s.top_memory.is_empty());
                    got_snapshot = true;
                }
                _ => {}
            }
        }
        assert!(got_info && got_snapshot);

        collector.shutdown();
        assert!(!collector.is_alive());
        // After shutdown a new collector may start.
        drop(Collector::spawn(Duration::from_secs(1), Arc::new(|| {}), None).unwrap());
    }
}
