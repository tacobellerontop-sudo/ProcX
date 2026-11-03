//! Persistent user configuration stored as TOML.
//!
//! Loading never fails hard: a missing file yields defaults, an unparsable
//! file yields defaults plus a warning (the broken file is left untouched
//! until the user explicitly saves), and out-of-range values are clamped
//! with a warning describing the correction.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::errors::{ProcxError, Result};
use super::models::Section;
use crate::process::explorer::ProcessColumn;

pub const MIN_REFRESH_MS: u64 = 250;
pub const MAX_REFRESH_MS: u64 = 10_000;
pub const MIN_CHART_WINDOW_SECS: u32 = 30;
pub const MAX_CHART_WINDOW_SECS: u32 = 3_600;
pub const MIN_RETENTION_DAYS: u32 = 1;
pub const MAX_RETENTION_DAYS: u32 = 365;
pub const MIN_HISTORY_SAMPLE_SECS: u32 = 5;
pub const MAX_HISTORY_SAMPLE_SECS: u32 = 3_600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub const ALL: [LogLevel; 5] = [Self::Error, Self::Warn, Self::Info, Self::Debug, Self::Trace];

    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warn => "Warning",
            Self::Info => "Info",
            Self::Debug => "Debug",
            Self::Trace => "Trace",
        }
    }

    pub fn as_tracing(self) -> tracing::Level {
        match self {
            Self::Error => tracing::Level::ERROR,
            Self::Warn => tracing::Level::WARN,
            Self::Info => tracing::Level::INFO,
            Self::Debug => tracing::Level::DEBUG,
            Self::Trace => tracing::Level::TRACE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub theme: ThemePreference,
    pub start_section: Section,
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            theme: ThemePreference::Dark,
            start_section: Section::Overview,
            start_minimized: false,
            minimize_to_tray: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitoringConfig {
    /// Interval between live samples, in milliseconds.
    pub refresh_interval_ms: u64,
    /// Time span shown by live charts, in seconds.
    pub chart_window_secs: u32,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            refresh_interval_ms: 1_000,
            chart_window_secs: 120,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessesConfig {
    /// Visible process table columns, in display order.
    pub visible_columns: Vec<ProcessColumn>,
    /// Show the parent/child tree instead of a flat list.
    pub tree_view: bool,
}

impl Default for ProcessesConfig {
    fn default() -> Self {
        Self {
            visible_columns: ProcessColumn::default_visible(),
            tree_view: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    pub enabled: bool,
    pub retention_days: u32,
    pub sample_interval_secs: u32,
    /// Custom database location. `None` uses the default data directory.
    pub database_path: Option<PathBuf>,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 7,
            sample_interval_secs: 30,
            database_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConfirmationConfig {
    pub end_process: bool,
    pub service_changes: bool,
    pub startup_changes: bool,
}

impl Default for ConfirmationConfig {
    fn default() -> Self {
        Self {
            end_process: true,
            service_changes: true,
            startup_changes: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: LogLevel,
}

/// Root configuration document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub monitoring: MonitoringConfig,
    pub processes: ProcessesConfig,
    pub history: HistoryConfig,
    pub confirmations: ConfirmationConfig,
    pub logging: LoggingConfig,
}

/// Result of loading configuration from disk.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: Config,
    /// Human-readable notes about defaults applied or values corrected.
    pub warnings: Vec<String>,
}

fn clamp_field<T: PartialOrd + Copy + std::fmt::Display>(
    value: &mut T,
    min: T,
    max: T,
    name: &str,
    warnings: &mut Vec<String>,
) {
    let original = *value;
    if *value < min {
        *value = min;
    } else if *value > max {
        *value = max;
    }
    if *value != original {
        warnings.push(format!(
            "{name} = {original} is out of range ({min}..={max}); using {}",
            *value
        ));
    }
}

impl Config {
    /// Clamps every numeric setting into its supported range.
    /// Returns a description of each correction made.
    pub fn validate(&mut self) -> Vec<String> {
        let mut w = Vec::new();
        clamp_field(
            &mut self.monitoring.refresh_interval_ms,
            MIN_REFRESH_MS,
            MAX_REFRESH_MS,
            "monitoring.refresh_interval_ms",
            &mut w,
        );
        clamp_field(
            &mut self.monitoring.chart_window_secs,
            MIN_CHART_WINDOW_SECS,
            MAX_CHART_WINDOW_SECS,
            "monitoring.chart_window_secs",
            &mut w,
        );
        clamp_field(
            &mut self.history.retention_days,
            MIN_RETENTION_DAYS,
            MAX_RETENTION_DAYS,
            "history.retention_days",
            &mut w,
        );
        clamp_field(
            &mut self.history.sample_interval_secs,
            MIN_HISTORY_SAMPLE_SECS,
            MAX_HISTORY_SAMPLE_SECS,
            "history.sample_interval_secs",
            &mut w,
        );
        let columns = &mut self.processes.visible_columns;
        let before = columns.len();
        let mut seen = std::collections::HashSet::new();
        columns.retain(|c| seen.insert(*c));
        if columns.len() != before {
            w.push("processes.visible_columns contained duplicates; removed them".to_owned());
        }
        if !columns.contains(&ProcessColumn::Name) {
            columns.insert(0, ProcessColumn::Name);
            w.push("processes.visible_columns must include \"name\"; added it".to_owned());
        }
        if let Some(path) = &self.history.database_path
            && path.as_os_str().is_empty()
        {
            self.history.database_path = None;
            w.push("history.database_path is empty; using the default location".to_owned());
        }
        w
    }

    /// Parses and validates a TOML document.
    pub fn from_toml(text: &str) -> std::result::Result<LoadedConfig, toml::de::Error> {
        let mut config: Config = toml::from_str(text)?;
        let warnings = config.validate();
        Ok(LoadedConfig { config, warnings })
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| ProcxError::ConfigSerialize(e.to_string()))
    }

    /// Loads configuration from `path`, falling back to defaults on any problem.
    pub fn load_or_default(path: &Path) -> LoadedConfig {
        match fs::read_to_string(path) {
            Ok(text) => match Self::from_toml(&text) {
                Ok(loaded) => loaded,
                Err(err) => LoadedConfig {
                    config: Config::default(),
                    warnings: vec![
                        ProcxError::ConfigParse {
                            path: path.to_owned(),
                            message: err.message().to_owned(),
                        }
                        .to_string()
                            + "; using defaults",
                    ],
                },
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => LoadedConfig {
                config: Config::default(),
                warnings: Vec::new(),
            },
            Err(source) => LoadedConfig {
                config: Config::default(),
                warnings: vec![
                    ProcxError::Io {
                        path: path.to_owned(),
                        source,
                    }
                    .to_string()
                        + "; using defaults",
                ],
            },
        }
    }

    /// Writes the configuration atomically (write to a temp file, then rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = self.to_toml()?;
        let io_err = |source| ProcxError::Io {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io_err)?;
        }
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(io_err)?;
        fs::rename(&tmp, path).map_err(io_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let config = Config::default();
        let text = config.to_toml().unwrap();
        let loaded = Config::from_toml(&text).unwrap();
        assert_eq!(loaded.config, config);
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn partial_document_fills_defaults() {
        let loaded = Config::from_toml("[general]\ntheme = \"light\"\n").unwrap();
        assert_eq!(loaded.config.general.theme, ThemePreference::Light);
        assert_eq!(loaded.config.monitoring, MonitoringConfig::default());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let loaded =
            Config::from_toml("[monitoring]\nrefresh_interval_ms = 1\nchart_window_secs = 999999\n").unwrap();
        assert_eq!(loaded.config.monitoring.refresh_interval_ms, MIN_REFRESH_MS);
        assert_eq!(loaded.config.monitoring.chart_window_secs, MAX_CHART_WINDOW_SECS);
        assert_eq!(loaded.warnings.len(), 2);
    }

    #[test]
    fn process_columns_are_normalized() {
        let loaded =
            Config::from_toml("[processes]\nvisible_columns = [\"pid\", \"cpu\", \"pid\"]\n").unwrap();
        assert_eq!(
            loaded.config.processes.visible_columns,
            vec![ProcessColumn::Name, ProcessColumn::Pid, ProcessColumn::Cpu]
        );
        assert_eq!(loaded.warnings.len(), 2);
    }

    #[test]
    fn invalid_document_is_an_error() {
        assert!(Config::from_toml("[general\ntheme = 3").is_err());
        assert!(Config::from_toml("[general]\ntheme = \"purple\"").is_err());
    }

    #[test]
    fn load_missing_and_corrupt_files() {
        let dir = std::env::temp_dir().join(format!("procx-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        let missing = Config::load_or_default(&dir.join("missing.toml"));
        assert_eq!(missing.config, Config::default());
        assert!(missing.warnings.is_empty());

        let corrupt = dir.join("corrupt.toml");
        fs::write(&corrupt, "not = [valid").unwrap();
        let loaded = Config::load_or_default(&corrupt);
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.warnings.len(), 1);

        let saved = dir.join("nested").join("procx.toml");
        let mut config = Config::default();
        config.general.theme = ThemePreference::Light;
        config.save(&saved).unwrap();
        assert_eq!(Config::load_or_default(&saved).config, config);

        fs::remove_dir_all(&dir).unwrap();
    }
}
