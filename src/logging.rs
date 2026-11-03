//! Logging setup. Logs go to a file because release builds have no console.

use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;

use crate::core::config::LogLevel;

/// Log files larger than this are truncated at start-up to bound disk use.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

fn open_log(path: &Path) -> std::io::Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let too_big = fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
    OpenOptions::new()
        .create(true)
        .append(!too_big)
        .write(true)
        .truncate(too_big)
        .open(path)
}

/// Installs the global tracing subscriber. Falls back to stderr if the log
/// file cannot be opened.
pub fn init(level: LogLevel, path: &Path) {
    let builder = tracing_subscriber::fmt()
        .with_max_level(level.as_tracing())
        .with_target(false)
        .with_thread_names(true);
    let result = match open_log(path) {
        Ok(file) => builder.with_ansi(false).with_writer(Mutex::new(file)).try_init(),
        Err(err) => {
            eprintln!("ProcX: cannot open log file {}: {err}", path.display());
            builder.with_writer(std::io::stderr).try_init()
        }
    };
    if let Err(err) = result {
        eprintln!("ProcX: logging already initialized: {err}");
    }
}
