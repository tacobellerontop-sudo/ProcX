//! History thread: receives collector ticks, writes one batched record per
//! interval, applies retention, and answers queries. SQLite never runs on
//! the UI thread.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, Sender, TrySendError, select};

use super::accumulate::{Accumulator, HistoryTick, TOP_PER_RECORD, unix_ms};
use super::export;
use super::store::{ProcessPoint, ProcessRow, Store, StoreStats, SystemPoint};
use crate::core::config::HistoryConfig;
use crate::core::errors::{ProcxError, Result};
use crate::monitoring::collector::Notifier;
use crate::utils::paths;

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Ticks buffered between the collector and the history thread.
const TICK_BUFFER: usize = 64;
const CLEANUP_EVERY: Duration = Duration::from_secs(3600);
/// Most process rows returned for one range.
pub const PROCESS_LIMIT: usize = 2000;
/// Chart resolution for system series.
pub const MAX_POINTS: usize = 800;

#[derive(Debug, Clone, PartialEq)]
pub enum HistoryRequest {
    Configure(HistoryConfig),
    /// System series and process list for a time range (Unix ms).
    Range {
        from_ms: i64,
        to_ms: i64,
    },
    ProcessSeries {
        id: i64,
        from_ms: i64,
        to_ms: i64,
    },
    ExportSystem {
        from_ms: i64,
        to_ms: i64,
    },
    ExportProcess {
        row: Box<ProcessRow>,
        from_ms: i64,
        to_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryStatus {
    pub enabled: bool,
    pub path: PathBuf,
    pub stats: Option<StoreStats>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum HistoryResponse {
    Status(HistoryStatus),
    Range {
        from_ms: i64,
        to_ms: i64,
        system: std::result::Result<Vec<SystemPoint>, String>,
        processes: std::result::Result<Vec<ProcessRow>, String>,
    },
    ProcessSeries {
        id: i64,
        points: std::result::Result<Vec<ProcessPoint>, String>,
    },
    Exported(std::result::Result<PathBuf, String>),
}

/// Feeds the history thread from the collector without ever blocking it.
#[derive(Clone)]
pub struct TickSender(Sender<HistoryTick>);

impl TickSender {
    pub fn send(&self, tick: HistoryTick) {
        match self.0.try_send(tick) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => tracing::trace!("history thread is behind; dropping a tick"),
        }
    }
}

pub struct HistoryWorker {
    requests: Option<Sender<HistoryRequest>>,
    responses: Receiver<HistoryResponse>,
    ticks: TickSender,
    thread: Option<JoinHandle<()>>,
}

impl HistoryWorker {
    pub fn spawn(config: HistoryConfig, notify: Notifier) -> Result<Self> {
        if RUNNING.swap(true, Ordering::SeqCst) {
            return Err(ProcxError::WorkerAlreadyRunning("history"));
        }
        let (req_tx, req_rx) = crossbeam_channel::unbounded();
        let (resp_tx, resp_rx) = crossbeam_channel::unbounded();
        let (tick_tx, tick_rx) = crossbeam_channel::bounded(TICK_BUFFER);
        let spawned = std::thread::Builder::new()
            .name("procx-history".into())
            .spawn(move || {
                struct Guard;
                impl Drop for Guard {
                    fn drop(&mut self) {
                        RUNNING.store(false, Ordering::SeqCst);
                    }
                }
                let _guard = Guard;
                let mut state = State::new(config);
                let reply = |r: HistoryResponse| {
                    if resp_tx.send(r).is_ok() {
                        notify();
                    }
                };
                reply(HistoryResponse::Status(state.status()));
                state.run(&req_rx, &tick_rx, &reply);
                state.flush();
                tracing::info!("history stopped");
            });
        match spawned {
            Ok(thread) => Ok(Self {
                requests: Some(req_tx),
                responses: resp_rx,
                ticks: TickSender(tick_tx),
                thread: Some(thread),
            }),
            Err(e) => {
                RUNNING.store(false, Ordering::SeqCst);
                Err(ProcxError::WorkerSpawn(e))
            }
        }
    }

    pub fn tick_sender(&self) -> TickSender {
        self.ticks.clone()
    }

    pub fn send(&self, request: HistoryRequest) {
        if let Some(tx) = &self.requests {
            let _ = tx.send(request);
        }
    }

    pub fn drain(&self) -> impl Iterator<Item = HistoryResponse> + '_ {
        self.responses.try_iter()
    }

    /// Stops the thread after it writes the pending partial interval.
    pub fn shutdown(&mut self) {
        self.requests = None;
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("history thread panicked");
        }
    }
}

impl Drop for HistoryWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct State {
    config: HistoryConfig,
    store: Option<Store>,
    open_error: Option<String>,
    acc: Accumulator,
    interval_started: Instant,
    last_cleanup: Option<Instant>,
}

fn db_path(config: &HistoryConfig) -> PathBuf {
    config
        .database_path
        .clone()
        .unwrap_or_else(paths::default_database_file)
}

impl State {
    fn new(config: HistoryConfig) -> Self {
        let mut s = Self {
            config,
            store: None,
            open_error: None,
            acc: Accumulator::default(),
            interval_started: Instant::now(),
            last_cleanup: None,
        };
        if s.config.enabled {
            s.ensure_open();
        }
        s
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.config.sample_interval_secs.max(1)))
    }

    /// Opens the database if needed. Also used for browsing while recording
    /// is off, so existing history stays viewable.
    fn ensure_open(&mut self) -> Option<&mut Store> {
        if self.store.is_none() {
            let path = db_path(&self.config);
            match Store::open(&path) {
                Ok(store) => {
                    tracing::info!("history database: {}", path.display());
                    self.store = Some(store);
                    self.open_error = None;
                    self.cleanup();
                }
                Err(e) => {
                    tracing::error!("cannot open history database {}: {e}", path.display());
                    self.open_error = Some(e);
                }
            }
        }
        self.store.as_mut()
    }

    fn status(&mut self) -> HistoryStatus {
        let path = db_path(&self.config);
        let stats = if path.exists() {
            self.ensure_open().and_then(|s| s.stats().ok())
        } else {
            None
        };
        HistoryStatus {
            enabled: self.config.enabled,
            path,
            stats,
            error: self.open_error.clone(),
        }
    }

    fn cleanup(&mut self) {
        let retention = Duration::from_secs(u64::from(self.config.retention_days) * 86_400);
        let cutoff = unix_ms(
            SystemTime::now()
                .checked_sub(retention)
                .unwrap_or(SystemTime::UNIX_EPOCH),
        );
        if let Some(store) = self.store.as_mut() {
            match store.cleanup(cutoff) {
                Ok(0) => {}
                Ok(n) => tracing::info!(rows = n, "history retention cleanup"),
                Err(e) => tracing::warn!("history cleanup failed: {e}"),
            }
        }
        self.last_cleanup = Some(Instant::now());
    }

    fn flush(&mut self) {
        if !self.config.enabled {
            self.acc.clear();
            return;
        }
        let Some(record) = self.acc.take(TOP_PER_RECORD) else {
            return;
        };
        if let Some(store) = self.ensure_open()
            && let Err(e) = store.insert(&record)
        {
            tracing::warn!("history write failed: {e}");
        }
    }

    fn on_tick(&mut self, tick: &HistoryTick) {
        if !self.config.enabled {
            return;
        }
        if self.acc.is_empty() {
            self.interval_started = Instant::now();
        }
        self.acc.add(tick);
        if self.interval_started.elapsed() >= self.interval() {
            self.flush();
        }
        if self.last_cleanup.is_none_or(|t| t.elapsed() >= CLEANUP_EVERY) {
            self.cleanup();
        }
    }

    fn configure(&mut self, config: HistoryConfig) {
        let reopen = db_path(&config) != db_path(&self.config);
        if reopen || !config.enabled {
            // Write what was gathered under the old settings first.
            self.flush();
        }
        let retention_changed = config.retention_days != self.config.retention_days;
        self.config = config;
        if reopen {
            self.store = None;
            self.open_error = None;
        }
        if self.config.enabled {
            self.ensure_open();
        }
        if retention_changed {
            self.cleanup();
        }
    }

    fn run(
        &mut self,
        requests: &Receiver<HistoryRequest>,
        ticks: &Receiver<HistoryTick>,
        reply: &impl Fn(HistoryResponse),
    ) {
        loop {
            select! {
                recv(ticks) -> tick => match tick {
                    Ok(tick) => self.on_tick(&tick),
                    // The collector stopped; keep serving queries.
                    Err(_) => return self.serve_only(requests, reply),
                },
                recv(requests) -> req => match req {
                    Ok(req) => self.handle(req, reply),
                    Err(_) => return,
                },
            }
        }
    }

    fn serve_only(&mut self, requests: &Receiver<HistoryRequest>, reply: &impl Fn(HistoryResponse)) {
        for req in requests {
            self.handle(req, reply);
        }
    }

    fn handle(&mut self, request: HistoryRequest, reply: &impl Fn(HistoryResponse)) {
        match request {
            HistoryRequest::Configure(config) => {
                self.configure(config);
                reply(HistoryResponse::Status(self.status()));
            }
            HistoryRequest::Range { from_ms, to_ms } => {
                let (system, processes) = match self.ensure_open() {
                    Some(store) => (
                        store.system_series(from_ms, to_ms, Some(MAX_POINTS)),
                        store.processes(from_ms, to_ms, PROCESS_LIMIT),
                    ),
                    None => {
                        let e = self.open_error.clone().unwrap_or_default();
                        (Err(e.clone()), Err(e))
                    }
                };
                reply(HistoryResponse::Range {
                    from_ms,
                    to_ms,
                    system,
                    processes,
                });
                reply(HistoryResponse::Status(self.status()));
            }
            HistoryRequest::ProcessSeries { id, from_ms, to_ms } => {
                let points = match self.ensure_open() {
                    Some(store) => store.process_series(id, from_ms, to_ms),
                    None => Err(self.open_error.clone().unwrap_or_default()),
                };
                reply(HistoryResponse::ProcessSeries { id, points });
            }
            HistoryRequest::ExportSystem { from_ms, to_ms } => {
                let result = self
                    .ensure_open()
                    .ok_or_else(|| "history database is not open".to_owned())
                    .and_then(|store| {
                        let points = store.system_series(from_ms, to_ms, None)?;
                        let path = paths::exports_dir()
                            .join(export::file_name("system", unix_ms(SystemTime::now())));
                        export::write_file(&path, |w| export::system_csv(w, &points))?;
                        Ok(path)
                    });
                log_export(&result);
                reply(HistoryResponse::Exported(result));
            }
            HistoryRequest::ExportProcess { row, from_ms, to_ms } => {
                let result = self
                    .ensure_open()
                    .ok_or_else(|| "history database is not open".to_owned())
                    .and_then(|store| {
                        let points = store.process_series(row.id, from_ms, to_ms)?;
                        let kind = format!("{}-{}", row.name, row.pid);
                        let path =
                            paths::exports_dir().join(export::file_name(&kind, unix_ms(SystemTime::now())));
                        export::write_file(&path, |w| export::process_csv(w, &row, &points))?;
                        Ok(path)
                    });
                log_export(&result);
                reply(HistoryResponse::Exported(result));
            }
        }
    }
}

fn log_export(result: &std::result::Result<PathBuf, String>) {
    match result {
        Ok(p) => tracing::info!("exported {}", p.display()),
        Err(e) => tracing::warn!("export failed: {e}"),
    }
}
