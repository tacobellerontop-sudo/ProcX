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
use crate::monitoring::collector::Notifier;
use crate::platform;

static RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessRequest {
    Inspect(ProcessKey),
    /// Terminate processes the user already confirmed.
    Terminate(Vec<(ProcessKey, String)>),
    OpenLocation(String),
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
    }
}
