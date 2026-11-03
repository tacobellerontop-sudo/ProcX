//! Non-Windows fallback used for development and CI on other hosts.
//! Windows-only information is reported as unavailable, and actions that
//! would affect the host are refused.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users};

use super::KernelCounters;
use crate::core::models::{AccessLevel, ProcessInfo, ProcessKey, ProcessStatus};
use crate::process::details::ProcessDetails;

const UNSUPPORTED: &str = "Not available on this platform";

pub fn kernel_counters() -> Option<KernelCounters> {
    None
}

pub fn is_elevated() -> Option<bool> {
    None
}

pub fn inspect_process(_key: ProcessKey) -> ProcessDetails {
    ProcessDetails {
        command_line: Err(UNSUPPORTED.into()),
        threads: Err(UNSUPPORTED.into()),
        modules: Err(UNSUPPORTED.into()),
    }
}

pub fn terminate_process(_key: ProcessKey) -> Result<(), String> {
    Err("Ending processes is only supported on Windows".into())
}

pub fn open_file_location(_path: &str) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

pub struct ProcessEnumerator {
    sys: Option<System>,
    users: Option<Users>,
    names: HashMap<String, Arc<str>>,
}

impl ProcessEnumerator {
    pub fn new() -> Self {
        Self {
            sys: System::new().ok(),
            users: Users::new_with_refreshed_list().ok(),
            names: HashMap::new(),
        }
    }

    pub fn enumerate(&mut self, out: &mut Vec<ProcessInfo>) -> Result<(), String> {
        let sys = self.sys.as_mut().ok_or("process information is unavailable")?;
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_user(UpdateKind::OnlyIfNotSet),
        );
        for (pid, p) in sys.processes() {
            // sysinfo may list threads as processes on Linux.
            if p.thread_kind().is_some() {
                continue;
            }
            let mut info = ProcessInfo::new(
                ProcessKey {
                    pid: pid.as_u32(),
                    start_time: p.start_time(),
                },
                p.name().to_string_lossy(),
            );
            info.parent_pid = p.parent().map(|pp| pp.as_u32());
            info.exe_path = p.exe().map(|e| Arc::from(e.to_string_lossy()));
            info.user = p.user_id().and_then(|uid| {
                let users = self.users.as_ref()?;
                let name = users.get_user_by_id(uid)?.name();
                Some(
                    self.names
                        .entry(name.to_owned())
                        .or_insert_with(|| Arc::from(name))
                        .clone(),
                )
            });
            info.started_at = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(p.start_time()));
            info.cpu_time = Some(Duration::from_millis(p.accumulated_cpu_time()));
            info.working_set = Some(p.memory());
            info.status = match p.status() {
                sysinfo::ProcessStatus::Run => ProcessStatus::Running,
                sysinfo::ProcessStatus::Sleep | sysinfo::ProcessStatus::Idle => ProcessStatus::Sleeping,
                sysinfo::ProcessStatus::Stop => ProcessStatus::Stopped,
                sysinfo::ProcessStatus::Zombie => ProcessStatus::Zombie,
                _ => ProcessStatus::Unknown,
            };
            info.access = AccessLevel::Queryable;
            out.push(info);
        }
        Ok(())
    }
}
