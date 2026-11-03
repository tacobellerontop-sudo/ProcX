//! Locations of configuration, logs and data.
//!
//! On Windows everything lives under `%APPDATA%\ProcX`. Other platforms
//! (used only for development) follow the XDG convention.

use std::path::PathBuf;

const APP_DIR: &str = "ProcX";

/// Directory containing `procx.toml`, logs and the history database.
pub fn app_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));

    base.unwrap_or_else(std::env::temp_dir).join(APP_DIR)
}

pub fn config_file() -> PathBuf {
    app_dir().join("procx.toml")
}

pub fn log_file() -> PathBuf {
    app_dir().join("procx.log")
}

/// Default history database location (used from Phase 5 onward).
pub fn default_database_file() -> PathBuf {
    app_dir().join("history.sqlite3")
}
