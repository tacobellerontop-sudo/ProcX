//! On-demand process inspection and actions (run on a worker thread).

use std::mem::size_of;

use ::windows::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use ::windows::Win32::Foundation::{HANDLE, STATUS_INFO_LENGTH_MISMATCH, UNICODE_STRING};
use ::windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, TH32CS_SNAPMODULE,
    TH32CS_SNAPMODULE32, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use ::windows::Win32::System::Threading::{
    GetThreadPriority, OpenThread, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    THREAD_QUERY_LIMITED_INFORMATION, TerminateProcess,
};

use super::handle::OwnedHandle;
use super::processes::{is_critical, open_process, process_times};
use super::util::{describe, is_access_denied, wide_to_string};
use crate::core::models::ProcessKey;
use crate::process::details::{ModuleInfo, ProcessDetails, ThreadInfo};

/// `GetThreadPriority` failure value (winbase.h `THREAD_PRIORITY_ERROR_RETURN` = MAXLONG).
const THREAD_PRIORITY_ERROR_RETURN: i32 = i32::MAX;

const EXITED: &str = "The process has exited";

/// Opens `key` and confirms it is still the same instance (not a reused PID).
fn open_verified(
    key: ProcessKey,
    access: ::windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS,
) -> Result<OwnedHandle, String> {
    let handle = open_process(key.pid, access | PROCESS_QUERY_LIMITED_INFORMATION).map_err(|e| {
        if is_access_denied(&e) {
            "Access denied".to_owned()
        } else {
            EXITED.to_owned()
        }
    })?;
    let times = process_times(handle.raw()).ok_or("Could not read the process creation time")?;
    if key.start_time != 0 && times.creation != key.start_time {
        return Err(EXITED.to_owned());
    }
    Ok(handle)
}

/// Reads the command line with `NtQueryInformationProcess(ProcessCommandLineInformation)`.
///
/// This information class (Windows 8.1+) works with only
/// `PROCESS_QUERY_LIMITED_INFORMATION`, unlike reading the PEB, which needs
/// `PROCESS_VM_READ`. The result buffer starts with a `UNICODE_STRING` whose
/// `Buffer` points into the same allocation.
/// <https://learn.microsoft.com/windows/win32/api/winternl/nf-winternl-ntqueryinformationprocess>
fn command_line(handle: HANDLE) -> Result<String, String> {
    let mut needed = 0u32;
    // SAFETY: size query with an empty buffer.
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            ProcessCommandLineInformation,
            std::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if status != STATUS_INFO_LENGTH_MISMATCH || needed == 0 {
        return Err(format!("Command line unavailable (NTSTATUS 0x{:08x})", status.0));
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: `buf` holds at least `needed` bytes, 8-byte aligned.
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            ProcessCommandLineInformation,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if status.is_err() {
        return Err(format!("Command line unavailable (NTSTATUS 0x{:08x})", status.0));
    }
    // SAFETY: on success the buffer begins with a UNICODE_STRING.
    let us = unsafe { &*buf.as_ptr().cast::<UNICODE_STRING>() };
    if us.Buffer.is_null() || us.Length == 0 {
        return Ok(String::new());
    }
    let start = buf.as_ptr() as usize;
    let end = start + buf.len() * 8;
    let text = us.Buffer.0 as usize;
    let bytes = usize::from(us.Length);
    // Defensive: only read if the string lies inside our buffer.
    if text < start || text + bytes > end {
        return Err("Command line buffer was malformed".into());
    }
    // SAFETY: bounds checked above; Length is in bytes.
    let units = unsafe { std::slice::from_raw_parts(us.Buffer.0, bytes / 2) };
    Ok(String::from_utf16_lossy(units))
}

fn threads(pid: u32) -> Result<Vec<ThreadInfo>, String> {
    // SAFETY: no preconditions; the handle is owned.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.map_err(|e| describe(&e))?;
    let snap = OwnedHandle::new(snap).ok_or("invalid thread snapshot")?;
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut out = Vec::new();
    // SAFETY: dwSize initialized.
    let mut more = unsafe { Thread32First(snap.raw(), &mut entry) }.is_ok();
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: OpenThread has no memory preconditions; handle owned.
            let priority = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, entry.th32ThreadID) }
                .ok()
                .and_then(OwnedHandle::new)
                .map(|h| unsafe { GetThreadPriority(h.raw()) })
                .filter(|&p| p != THREAD_PRIORITY_ERROR_RETURN);
            out.push(ThreadInfo {
                tid: entry.th32ThreadID,
                base_priority: entry.tpBasePri,
                priority,
            });
        }
        // SAFETY: as above.
        more = unsafe { Thread32Next(snap.raw(), &mut entry) }.is_ok();
    }
    Ok(out)
}

/// Module snapshots need PROCESS_QUERY_INFORMATION | PROCESS_VM_READ on the
/// target, so they fail for elevated, other-user and protected processes.
fn modules(pid: u32) -> Result<Vec<ModuleInfo>, String> {
    // ERROR_BAD_LENGTH can occur transiently while the target loads modules.
    let mut last_err = String::new();
    for _ in 0..3 {
        // SAFETY: no preconditions; handle owned.
        match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) } {
            Ok(raw) => {
                let snap = OwnedHandle::new(raw).ok_or("invalid module snapshot")?;
                let mut entry = MODULEENTRY32W {
                    dwSize: size_of::<MODULEENTRY32W>() as u32,
                    ..Default::default()
                };
                let mut out = Vec::new();
                // SAFETY: dwSize initialized.
                let mut more = unsafe { Module32FirstW(snap.raw(), &mut entry) }.is_ok();
                while more {
                    out.push(ModuleInfo {
                        name: wide_to_string(&entry.szModule),
                        path: wide_to_string(&entry.szExePath),
                        base_address: entry.modBaseAddr as usize,
                        size: entry.modBaseSize,
                    });
                    // SAFETY: as above.
                    more = unsafe { Module32NextW(snap.raw(), &mut entry) }.is_ok();
                }
                return Ok(out);
            }
            Err(e) if is_access_denied(&e) => {
                return Err(
                    "Access denied. Modules of elevated, protected or other users' processes \
                            require administrator rights."
                        .into(),
                );
            }
            Err(e) => last_err = describe(&e),
        }
    }
    Err(last_err)
}

pub fn inspect_process(key: ProcessKey) -> ProcessDetails {
    let handle = open_verified(key, PROCESS_QUERY_LIMITED_INFORMATION);
    let file_info = handle
        .as_ref()
        .ok()
        .and_then(|h| super::processes::image_path(h.raw()))
        .and_then(|path| super::files::file_version_info(&path));
    ProcessDetails {
        file_info,
        command_line: handle
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|h| command_line(h.raw())),
        threads: threads(key.pid),
        modules: modules(key.pid),
    }
}

/// Terminates a process after re-verifying its identity and that Windows does
/// not mark it critical. Callers must already have obtained user confirmation.
pub fn terminate_process(key: ProcessKey) -> Result<(), String> {
    let handle = open_verified(key, PROCESS_TERMINATE).map_err(|e| {
        if e == "Access denied" {
            "Access denied. Ending this process requires administrator rights, \
             or it is protected by Windows."
                .to_owned()
        } else {
            e
        }
    })?;
    if is_critical(handle.raw()) == Some(true) {
        return Err("Windows marks this process as critical; ProcX will not end it.".into());
    }
    // SAFETY: valid handle opened with PROCESS_TERMINATE.
    unsafe { TerminateProcess(handle.raw(), 1) }.map_err(|e| describe(&e))
}

/// Opens Explorer with the file selected. Runs only on explicit user action.
pub fn open_file_location(path: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    if path.contains('"') {
        return Err("Invalid path".into());
    }
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{path}\""))
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Starts a new elevated instance of ProcX through the standard UAC prompt.
/// Fails (without side effects) if the user declines the prompt.
pub fn relaunch_elevated() -> Result<(), String> {
    use ::windows::Win32::UI::Shell::ShellExecuteW;
    use ::windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use ::windows::core::{HSTRING, PCWSTR, w};

    let exe = std::env::current_exe().map_err(|e| format!("cannot locate ProcX: {e}"))?;
    let file = HSTRING::from(exe.as_os_str());
    // SAFETY: all strings are valid and NUL-terminated for the call's duration.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            &file,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns a value greater than 32 on success.
    let code = result.0 as isize;
    if code > 32 {
        Ok(())
    } else if code == 5 {
        // SE_ERR_ACCESSDENIED: the UAC prompt was declined.
        Err("The administrator prompt was declined".into())
    } else {
        Err(format!(
            "Windows could not start an elevated instance (code {code})"
        ))
    }
}
