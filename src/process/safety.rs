//! Safeguards for terminating processes.

use crate::core::models::ProcessInfo;

/// Processes whose termination destabilizes or crashes Windows. Matching is
/// case-insensitive on the image name and is a second line of defence after
/// the `IsProcessCritical` check performed by the platform layer.
const PROTECTED_NAMES: [&str; 10] = [
    "system",
    "registry",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "lsaiso.exe",
    "memory compression",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminateBlock {
    SystemProcess,
    Critical,
    SelfProcess,
}

impl TerminateBlock {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::SystemProcess => {
                "This is a core Windows process. Ending it would crash or destabilize the system."
            }
            Self::Critical => "Windows marks this process as critical. Ending it would crash the system.",
            Self::SelfProcess => "This is ProcX itself. Close the window instead.",
        }
    }
}

/// Decides whether ProcX allows ending `process` at all.
pub fn check_terminate(process: &ProcessInfo, own_pid: u32) -> Result<(), TerminateBlock> {
    let pid = process.key.pid;
    if pid == own_pid {
        return Err(TerminateBlock::SelfProcess);
    }
    if pid == 0
        || pid == 4
        || PROTECTED_NAMES
            .iter()
            .any(|n| process.name.eq_ignore_ascii_case(n))
    {
        return Err(TerminateBlock::SystemProcess);
    }
    if process.critical == Some(true) {
        return Err(TerminateBlock::Critical);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::ProcessKey;

    fn proc(pid: u32, name: &str) -> ProcessInfo {
        ProcessInfo::new(ProcessKey { pid, start_time: 1 }, name)
    }

    #[test]
    fn blocks_system_and_self() {
        assert_eq!(
            check_terminate(&proc(4, "System"), 99),
            Err(TerminateBlock::SystemProcess)
        );
        assert_eq!(
            check_terminate(&proc(700, "LSASS.EXE"), 99),
            Err(TerminateBlock::SystemProcess)
        );
        assert_eq!(
            check_terminate(&proc(99, "procx.exe"), 99),
            Err(TerminateBlock::SelfProcess)
        );
    }

    #[test]
    fn blocks_critical_flag() {
        let mut p = proc(1234, "something.exe");
        p.critical = Some(true);
        assert_eq!(check_terminate(&p, 99), Err(TerminateBlock::Critical));
        p.critical = None;
        assert_eq!(check_terminate(&p, 99), Ok(()));
    }
}
