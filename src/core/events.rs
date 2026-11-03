//! Messages exchanged between background workers and the UI thread.

use std::time::Duration;

use super::models::{StaticSystemInfo, SystemSnapshot};

/// Sent from the UI to the collector worker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CollectorCommand {
    /// Change the sampling interval. The value is clamped by the collector.
    SetInterval(Duration),
    /// Take a sample as soon as possible instead of waiting for the next tick.
    RefreshNow,
    /// Stop the worker thread.
    Shutdown,
}

/// Sent from the collector worker to the UI.
#[derive(Debug, Clone)]
pub enum CollectorEvent {
    /// Information that does not change while the app runs. Sent once at start-up.
    StaticInfo(Box<StaticSystemInfo>),
    /// A new periodic sample.
    Snapshot(Box<SystemSnapshot>),
    /// A non-fatal problem the user may want to know about.
    Warning(String),
}
