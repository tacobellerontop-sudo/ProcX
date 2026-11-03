//! Process enumeration.
//!
//! The process list comes from a Toolhelp32 snapshot, which needs no special
//! rights and supplies PID, parent PID, thread count and image name for every
//! process. Each process is then opened with `PROCESS_QUERY_LIMITED_INFORMATION`
//! (the least privileged query right; it is granted for most processes,
//! including other users' and protected ones) to read creation time, CPU
//! times, memory counters and handle count. Opening can still fail
//! (e.g. the System Idle Process); such processes are reported with
//! `AccessLevel::Denied` and their counters left as `None`.
//!
//! Values that never change for a process instance (path, user, architecture,
//! session, critical flag) are cached per `ProcessKey`.

use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::sync::Arc;
use std::time::Duration;

use ::windows::Win32::Foundation::{FILETIME, HANDLE};
use ::windows::Win32::Security::{
    GetLengthSid, GetTokenInformation, LookupAccountSidW, SID_NAME_USE, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use ::windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use ::windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use ::windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use ::windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
    IMAGE_FILE_MACHINE_UNKNOWN,
};
use ::windows::Win32::System::Threading::{
    GetProcessHandleCount, GetProcessTimes, IsProcessCritical, IsWow64Process2, OpenProcess,
    OpenProcessToken, PROCESS_ACCESS_RIGHTS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use ::windows::core::{BOOL, PCWSTR, PWSTR};

use super::handle::OwnedHandle;
use super::util::{filetime_to_system_time, filetime_u64, wide_to_string};
use crate::core::models::{AccessLevel, Architecture, ProcessInfo, ProcessKey, ProcessStatus};

/// Opens a process, returning `None` when access is refused or it has exited.
pub(super) fn open_process(
    pid: u32,
    access: PROCESS_ACCESS_RIGHTS,
) -> Result<OwnedHandle, ::windows::core::Error> {
    // SAFETY: OpenProcess has no memory-safety preconditions; the returned
    // handle is owned by OwnedHandle.
    let raw = unsafe { OpenProcess(access, false, pid) }?;
    OwnedHandle::new(raw).ok_or_else(::windows::core::Error::from_thread)
}

pub(super) struct Times {
    pub creation: u64,
    pub cpu: Duration,
}

pub(super) fn process_times(handle: HANDLE) -> Option<Times> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: all four out-pointers reference valid FILETIMEs.
    unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;
    let ticks = filetime_u64(kernel).saturating_add(filetime_u64(user));
    Some(Times {
        creation: filetime_u64(created),
        cpu: Duration::from_nanos(ticks.saturating_mul(100)),
    })
}

fn memory_counters(handle: HANDLE) -> Option<PROCESS_MEMORY_COUNTERS_EX> {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: PROCESS_MEMORY_COUNTERS_EX starts with the PROCESS_MEMORY_COUNTERS
    // layout; `cb` tells the API which variant was supplied.
    unsafe {
        GetProcessMemoryInfo(
            handle,
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    }
    .ok()?;
    Some(counters)
}

fn handle_count(handle: HANDLE) -> Option<u32> {
    let mut count = 0u32;
    // SAFETY: valid out-pointer.
    unsafe { GetProcessHandleCount(handle, &mut count) }.ok()?;
    Some(count)
}

pub(super) fn image_path(handle: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    loop {
        let mut len = buf.len() as u32;
        // SAFETY: `buf` has `len` writable UTF-16 units; on success `len`
        // receives the number of characters written (without NUL).
        let result = unsafe {
            QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
        };
        match result {
            Ok(()) => return Some(String::from_utf16_lossy(&buf[..len as usize])),
            // Long paths: grow up to the 32K UNICODE_STRING limit.
            Err(_) if buf.len() < 32_768 => buf.resize(buf.len() * 4, 0),
            Err(_) => return None,
        }
    }
}

fn architecture(handle: HANDLE) -> Option<Architecture> {
    let mut process_machine = IMAGE_FILE_MACHINE::default();
    let mut native_machine = IMAGE_FILE_MACHINE::default();
    // SAFETY: valid out-pointers. For a non-WOW64 process `process_machine`
    // is IMAGE_FILE_MACHINE_UNKNOWN and the process runs natively.
    unsafe { IsWow64Process2(handle, &mut process_machine, Some(&mut native_machine)) }.ok()?;
    let machine = if process_machine == IMAGE_FILE_MACHINE_UNKNOWN {
        native_machine
    } else {
        process_machine
    };
    Some(match machine {
        IMAGE_FILE_MACHINE_AMD64 => Architecture::X64,
        IMAGE_FILE_MACHINE_I386 => Architecture::X86,
        IMAGE_FILE_MACHINE_ARM64 => Architecture::Arm64,
        other => Architecture::Other(other.0),
    })
}

pub(super) fn is_critical(handle: HANDLE) -> Option<bool> {
    let mut critical = BOOL::default();
    // SAFETY: valid out-pointer.
    unsafe { IsProcessCritical(handle, &mut critical) }.ok()?;
    Some(critical.as_bool())
}

fn session_id(pid: u32) -> Option<u32> {
    let mut session = 0u32;
    // SAFETY: valid out-pointer; needs no process handle.
    unsafe { ProcessIdToSessionId(pid, &mut session) }.ok()?;
    Some(session)
}

/// Reads the token's user SID as raw bytes. Fails for other users'
/// processes unless ProcX is elevated, which is expected.
fn token_user_sid(handle: HANDLE) -> Option<Vec<u8>> {
    let mut raw = HANDLE::default();
    // SAFETY: valid out-pointer; the token handle is owned below.
    unsafe { OpenProcessToken(handle, TOKEN_QUERY, &mut raw) }.ok()?;
    let token = OwnedHandle::new(raw)?;

    let mut needed = 0u32;
    // SAFETY: size query with no buffer; fails with ERROR_INSUFFICIENT_BUFFER by design.
    let _ = unsafe { GetTokenInformation(token.raw(), TokenUser, None, 0, &mut needed) };
    if needed == 0 {
        return None;
    }
    // u64 storage gives the alignment TOKEN_USER requires.
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: `buf` is at least `needed` bytes and suitably aligned.
    unsafe {
        GetTokenInformation(
            token.raw(),
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    }
    .ok()?;
    // SAFETY: the API filled a TOKEN_USER at the start of the buffer, whose
    // SID pointer points into the same buffer (alive for this scope).
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    // SAFETY: `sid` is a valid SID per the API contract.
    let len = unsafe { GetLengthSid(sid) } as usize;
    // SAFETY: a valid SID occupies `len` readable bytes.
    Some(unsafe { std::slice::from_raw_parts(sid.0.cast::<u8>(), len) }.to_vec())
}

/// Resolves a SID to `DOMAIN\name`.
fn account_name(sid: &mut [u8]) -> Option<String> {
    let psid = ::windows::Win32::Security::PSID(sid.as_mut_ptr().cast());
    let mut name = [0u16; 256];
    let mut domain = [0u16; 256];
    let (mut name_len, mut domain_len) = (name.len() as u32, domain.len() as u32);
    let mut kind = SID_NAME_USE::default();
    // SAFETY: buffers and their lengths match; `psid` points to a valid SID.
    unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            psid,
            Some(PWSTR(name.as_mut_ptr())),
            &mut name_len,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut domain_len,
            &mut kind,
        )
    }
    .ok()?;
    let name = wide_to_string(&name);
    let domain = wide_to_string(&domain);
    Some(if domain.is_empty() {
        name
    } else {
        format!("{domain}\\{name}")
    })
}

struct StaticInfo {
    exe_path: Option<Arc<str>>,
    user: Option<Arc<str>>,
    architecture: Option<Architecture>,
    critical: Option<bool>,
}

#[derive(Default)]
pub struct ProcessEnumerator {
    cache: HashMap<ProcessKey, StaticInfo>,
    /// SID → account name. Lookups can hit the network for domain accounts,
    /// so each SID is resolved once.
    accounts: HashMap<Vec<u8>, Option<Arc<str>>>,
    seen: HashSet<ProcessKey>,
}

impl ProcessEnumerator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enumerate(&mut self, out: &mut Vec<ProcessInfo>) -> Result<(), String> {
        // SAFETY: no preconditions; the snapshot handle is owned below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|e| format!("CreateToolhelp32Snapshot failed: {e}"))?;
        let snapshot = OwnedHandle::new(snapshot).ok_or("invalid process snapshot")?;

        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        self.seen.clear();
        // SAFETY: `entry.dwSize` is initialized as the API requires.
        let mut more = unsafe { Process32FirstW(snapshot.raw(), &mut entry) }.is_ok();
        while more {
            let info = self.describe(&entry);
            self.seen.insert(info.key);
            out.push(info);
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot.raw(), &mut entry) }.is_ok();
        }
        let seen = &self.seen;
        self.cache.retain(|k, _| seen.contains(k));
        Ok(())
    }

    fn describe(&mut self, entry: &PROCESSENTRY32W) -> ProcessInfo {
        let pid = entry.th32ProcessID;
        let name = match pid {
            0 => "System Idle Process".to_owned(),
            _ => wide_to_string(&entry.szExeFile),
        };
        let mut info = ProcessInfo::new(ProcessKey { pid, start_time: 0 }, name);
        info.parent_pid = (pid != 0).then_some(entry.th32ParentProcessID);
        info.threads = Some(entry.cntThreads);
        info.session_id = session_id(pid);
        info.status = ProcessStatus::Running;

        if pid == 0 {
            return info;
        }
        let Ok(handle) = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
            return info;
        };
        let h = handle.raw();
        info.access = AccessLevel::Queryable;
        if let Some(times) = process_times(h) {
            info.key.start_time = times.creation;
            info.started_at = filetime_to_system_time(times.creation);
            info.cpu_time = Some(times.cpu);
        }
        if let Some(mem) = memory_counters(h) {
            info.working_set = Some(mem.WorkingSetSize as u64);
            info.peak_working_set = Some(mem.PeakWorkingSetSize as u64);
            info.private_bytes = Some(mem.PrivateUsage as u64);
        }
        info.handles = handle_count(h);

        let accounts = &mut self.accounts;
        let cached = self.cache.entry(info.key).or_insert_with(|| StaticInfo {
            exe_path: image_path(h).map(Arc::from),
            user: token_user_sid(h).and_then(|mut sid| {
                accounts
                    .entry(sid.clone())
                    .or_insert_with(|| account_name(&mut sid).map(Arc::from))
                    .clone()
            }),
            architecture: architecture(h),
            critical: is_critical(h),
        });
        info.exe_path = cached.exe_path.clone();
        info.user = cached.user.clone();
        info.architecture = cached.architecture;
        info.critical = cached.critical;
        info
    }
}
