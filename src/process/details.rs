//! Data returned by on-demand process inspection.

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadInfo {
    pub tid: u32,
    /// Base priority from the thread snapshot.
    pub base_priority: i32,
    /// Current relative priority (GetThreadPriority), if the thread could be opened.
    pub priority: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleInfo {
    pub name: String,
    pub path: String,
    pub base_address: usize,
    pub size: u32,
}

/// Result of inspecting one process. Each part fails independently so one
/// access-denied query does not hide the others.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessDetails {
    pub command_line: Result<String, String>,
    pub threads: Result<Vec<ThreadInfo>, String>,
    pub modules: Result<Vec<ModuleInfo>, String>,
}
