//! Windows service models.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceState {
    Stopped,
    StartPending,
    StopPending,
    Running,
    ContinuePending,
    PausePending,
    Paused,
    Unknown(u32),
}

impl ServiceState {
    pub fn from_raw(v: u32) -> Self {
        match v {
            1 => Self::Stopped,
            2 => Self::StartPending,
            3 => Self::StopPending,
            4 => Self::Running,
            5 => Self::ContinuePending,
            6 => Self::PausePending,
            7 => Self::Paused,
            other => Self::Unknown(other),
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Stopped => "Stopped".into(),
            Self::StartPending => "Starting".into(),
            Self::StopPending => "Stopping".into(),
            Self::Running => "Running".into(),
            Self::ContinuePending => "Resuming".into(),
            Self::PausePending => "Pausing".into(),
            Self::Paused => "Paused".into(),
            Self::Unknown(v) => format!("Unknown ({v})"),
        }
    }

    pub fn is_running(self) -> bool {
        matches!(self, Self::Running | Self::StartPending | Self::ContinuePending)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartType {
    Boot,
    System,
    Automatic,
    AutomaticDelayed,
    Manual,
    Disabled,
}

impl StartType {
    /// Start types a user may choose in the UI. Boot/System apply to drivers only.
    pub const CHOOSABLE: [StartType; 4] = [
        Self::Automatic,
        Self::AutomaticDelayed,
        Self::Manual,
        Self::Disabled,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Boot => "Boot",
            Self::System => "System",
            Self::Automatic => "Automatic",
            Self::AutomaticDelayed => "Automatic (delayed)",
            Self::Manual => "Manual",
            Self::Disabled => "Disabled",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceInfo {
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub state: ServiceState,
    /// `None` when the configuration could not be read.
    pub start_type: Option<StartType>,
    pub pid: Option<u32>,
    pub binary_path: Option<String>,
    pub account: Option<String>,
    /// Services (and `+`-prefixed load-order groups) this service depends on.
    pub dependencies: Vec<String>,
    /// Shares a process with other services (svchost).
    pub shared_process: bool,
    /// Whether the service accepts a stop request right now.
    pub can_stop: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceControl {
    Start,
    Stop,
}

impl ServiceControl {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
        }
    }
}

/// Names of services that depend on `name`, derived from all configurations.
pub fn dependents<'a>(services: &'a [ServiceInfo], name: &str) -> Vec<&'a ServiceInfo> {
    services
        .iter()
        .filter(|s| s.dependencies.iter().any(|d| d.eq_ignore_ascii_case(name)))
        .collect()
}

/// Case-insensitive match on display name, service name, description or PID.
/// `query` must already be lowercase.
pub fn matches(service: &ServiceInfo, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let contains = |s: &str| s.to_lowercase().contains(query);
    contains(&service.name)
        || contains(&service.display_name)
        || service.description.as_deref().is_some_and(contains)
        || service.pid.is_some_and(|p| p.to_string() == query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str, deps: &[&str]) -> ServiceInfo {
        ServiceInfo {
            name: name.into(),
            display_name: name.into(),
            description: None,
            state: ServiceState::Stopped,
            start_type: None,
            pid: None,
            binary_path: None,
            account: None,
            dependencies: deps.iter().map(|d| d.to_string()).collect(),
            shared_process: false,
            can_stop: false,
        }
    }

    #[test]
    fn finds_dependents_case_insensitively() {
        let all = [svc("RpcSs", &[]), svc("A", &["rpcss"]), svc("B", &["Other"])];
        let d = dependents(&all, "RpcSs");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "A");
    }

    #[test]
    fn search_matches() {
        let mut s = svc("Spooler", &[]);
        s.display_name = "Print Spooler".into();
        s.pid = Some(42);
        assert!(matches(&s, ""));
        assert!(matches(&s, "print"));
        assert!(matches(&s, "spool"));
        assert!(matches(&s, "42"));
        assert!(!matches(&s, "4"));
        assert!(!matches(&s, "audio"));
    }

    #[test]
    fn state_mapping() {
        assert_eq!(ServiceState::from_raw(4), ServiceState::Running);
        assert!(ServiceState::from_raw(2).is_running());
        assert_eq!(ServiceState::from_raw(99).label(), "Unknown (99)");
    }
}
