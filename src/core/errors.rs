//! Application error type.

use std::fmt;
use std::path::PathBuf;

/// Errors produced by ProcX subsystems.
///
/// UI code should never panic on these; they are surfaced as status messages
/// or "unavailable" markers instead.
#[derive(Debug)]
pub enum ProcxError {
    /// Filesystem error with the path that was being accessed.
    Io { path: PathBuf, source: std::io::Error },
    /// The configuration file could not be parsed.
    ConfigParse { path: PathBuf, message: String },
    /// The configuration could not be serialized.
    ConfigSerialize(String),
    /// A background worker of this kind is already running.
    WorkerAlreadyRunning(&'static str),
    /// Spawning a background worker failed.
    WorkerSpawn(std::io::Error),
}

impl fmt::Display for ProcxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::ConfigParse { path, message } => {
                write!(f, "invalid configuration in {}: {message}", path.display())
            }
            Self::ConfigSerialize(message) => write!(f, "failed to serialize configuration: {message}"),
            Self::WorkerAlreadyRunning(name) => write!(f, "the {name} worker is already running"),
            Self::WorkerSpawn(err) => write!(f, "failed to start background worker: {err}"),
        }
    }
}

impl std::error::Error for ProcxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::WorkerSpawn(err) => Some(err),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, ProcxError>;
