//! Worker thread for on-demand process inspection and user-initiated actions.
//!
//! Inspection (threads, modules, command line) and termination can be slow
//! or block on the OS, so they never run on the UI thread. Requests are
//! processed in order; results come back over a channel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};

use super::details::ProcessDetails;
use crate::core::errors::{ProcxError, Result};
use crate::core::models::ProcessKey;
use crate::core::services::{ServiceControl, ServiceInfo, StartType};
use crate::core::startup::{StartupEntry, StartupSource, StartupStatus};
use crate::monitoring::collector::Notifier;
use crate::platform;

static RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessRequest {
    Inspect(ProcessKey),
    /// Terminate processes the user already confirmed.
    Terminate(Vec<(ProcessKey, String)>),
    OpenLocation(String),
    ListServices,
    /// A service start/stop the user already confirmed.
    ControlService {
        name: String,
        control: ServiceControl,
    },
    /// A start-type change the user already confirmed.
    SetServiceStartType {
        name: String,
        start: StartType,
    },
    ListStartup,
    /// Start an elevated instance (UAC prompt); the user asked for it.
    RelaunchElevated,
    /// An enable/disable the user already confirmed.
    SetStartupStatus {
        source: StartupSource,
        name: String,
        status: StartupStatus,
    },
}

#[derive(Debug, Clone)]
pub enum ProcessResponse {
    Details(ProcessKey, Box<ProcessDetails>),
    Terminated {
        key: ProcessKey,
        name: String,
        result: std::result::Result<(), String>,
    },
    OpenLocationFailed(String),
    Services(std::result::Result<Vec<ServiceInfo>, String>),
    /// Outcome of a service change; `action` describes it for the user.
    ServiceChanged {
        action: String,
        result: std::result::Result<(), String>,
    },
    Startup(std::result::Result<Vec<StartupEntry>, String>),
    Relaunched(std::result::Result<(), String>),
    StartupChanged {
        action: String,
        result: std::result::Result<(), String>,
    },
}

pub struct ProcessWorker {
    requests: Option<Sender<ProcessRequest>>,
    responses: Receiver<ProcessResponse>,
    thread: Option<JoinHandle<()>>,
}

impl ProcessWorker {
    pub fn spawn(notify: Notifier) -> Result<Self> {
        if RUNNING.swap(true, Ordering::SeqCst) {
            return Err(ProcxError::WorkerAlreadyRunning("process"));
        }
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<ProcessRequest>();
        let (resp_tx, resp_rx) = crossbeam_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("procx-process-worker".into())
            .spawn(move || {
                struct Guard;
                impl Drop for Guard {
                    fn drop(&mut self) {
                        RUNNING.store(false, Ordering::SeqCst);
                    }
                }
                let _guard = Guard;
                // Ends when the UI drops the request sender.
                for request in req_rx {
                    for response in handle(request) {
                        if resp_tx.send(response).is_err() {
                            return;
                        }
                        notify();
                    }
                }
            });
        match spawned {
            Ok(thread) => Ok(Self {
                requests: Some(req_tx),
                responses: resp_rx,
                thread: Some(thread),
            }),
            Err(err) => {
                RUNNING.store(false, Ordering::SeqCst);
                Err(ProcxError::WorkerSpawn(err))
            }
        }
    }

    pub fn send(&self, request: ProcessRequest) {
        if let Some(tx) = &self.requests {
            let _ = tx.send(request);
        }
    }

    pub fn drain(&self) -> impl Iterator<Item = ProcessResponse> + '_ {
        self.responses.try_iter()
    }

    pub fn shutdown(&mut self) {
        self.requests = None;
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("process worker panicked");
        }
    }
}

impl Drop for ProcessWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn handle(request: ProcessRequest) -> Vec<ProcessResponse> {
    match request {
        ProcessRequest::Inspect(key) => {
            vec![ProcessResponse::Details(
                key,
                Box::new(platform::inspect_process(key)),
            )]
        }
        ProcessRequest::Terminate(targets) => targets
            .into_iter()
            .map(|(key, name)| {
                let result = platform::terminate_process(key);
                match &result {
                    Ok(()) => tracing::info!(pid = key.pid, %name, "process terminated by user"),
                    Err(e) => tracing::warn!(pid = key.pid, %name, "terminate failed: {e}"),
                }
                ProcessResponse::Terminated { key, name, result }
            })
            .collect(),
        ProcessRequest::OpenLocation(path) => match platform::open_file_location(&path) {
            Ok(()) => Vec::new(),
            Err(e) => vec![ProcessResponse::OpenLocationFailed(e)],
        },
        ProcessRequest::ListServices => vec![ProcessResponse::Services(platform::list_services())],
        ProcessRequest::ControlService { name, control } => {
            let result = platform::control_service(&name, control);
            log_change("service control", &name, control.label(), &result);
            vec![
                ProcessResponse::ServiceChanged {
                    action: format!("{} service {name}", control.label()),
                    result,
                },
                // Re-list so the page shows the state Windows reports, not an assumed one.
                ProcessResponse::Services(platform::list_services()),
            ]
        }
        ProcessRequest::SetServiceStartType { name, start } => {
            let result = platform::set_service_start_type(&name, start);
            log_change("service start type", &name, start.label(), &result);
            vec![
                ProcessResponse::ServiceChanged {
                    action: format!("Set start type of {name} to {}", start.label()),
                    result,
                },
                ProcessResponse::Services(platform::list_services()),
            ]
        }
        ProcessRequest::ListStartup => vec![ProcessResponse::Startup(platform::list_startup())],
        // Blocks while the UAC prompt is open, which is why it runs here.
        ProcessRequest::RelaunchElevated => vec![ProcessResponse::Relaunched(platform::relaunch_elevated())],
        ProcessRequest::SetStartupStatus { source, name, status } => {
            let verb = match status {
                StartupStatus::Enabled => "Enable",
                StartupStatus::Disabled => "Disable",
            };
            let result = platform::set_startup_status(source, &name, status);
            log_change("startup entry", &name, verb, &result);
            vec![
                ProcessResponse::StartupChanged {
                    action: format!("{verb} startup entry {name}"),
                    result,
                },
                ProcessResponse::Startup(platform::list_startup()),
            ]
        }
    }
}

/// Every configuration change is logged, whether it succeeded or not.
fn log_change(kind: &str, name: &str, change: &str, result: &std::result::Result<(), String>) {
    match result {
        Ok(()) => tracing::info!(%kind, %name, %change, "changed by user"),
        Err(e) => tracing::warn!(%kind, %name, %change, "change failed: {e}"),
    }
}
