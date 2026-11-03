//! System-wide information from psapi and the security APIs.

use std::mem::size_of;

use ::windows::Win32::Foundation::HANDLE;
use ::windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use ::windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use ::windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::handle::OwnedHandle;
use crate::platform::KernelCounters;

/// Reads `GetPerformanceInfo`.
///
/// The commit fields of `PERFORMANCE_INFORMATION` are expressed in pages,
/// so they are multiplied by `PageSize`. The handle/process/thread counts are
/// plain counts. The call needs no special privileges.
/// <https://learn.microsoft.com/windows/win32/api/psapi/nf-psapi-getperformanceinfo>
pub fn kernel_counters() -> Option<KernelCounters> {
    let mut info = PERFORMANCE_INFORMATION {
        cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a properly sized, writable PERFORMANCE_INFORMATION and
    // `cb` matches its size.
    if let Err(err) = unsafe { GetPerformanceInfo(&mut info, info.cb) } {
        tracing::debug!("GetPerformanceInfo failed: {err}");
        return None;
    }
    let page = info.PageSize as u64;
    Some(KernelCounters {
        handle_count: u64::from(info.HandleCount),
        thread_count: u64::from(info.ThreadCount),
        process_count: u64::from(info.ProcessCount),
        commit_total: (info.CommitTotal as u64).saturating_mul(page),
        commit_limit: (info.CommitLimit as u64).saturating_mul(page),
    })
}

/// Queries `TokenElevation` on the current process token.
/// <https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-token_elevation>
pub fn is_elevated() -> Option<bool> {
    let mut raw = HANDLE::default();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing;
    // `raw` receives a real token handle that OwnedHandle closes.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) }.ok()?;
    let token = OwnedHandle::new(raw)?;

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    // SAFETY: the buffer is a TOKEN_ELEVATION of the size we pass.
    unsafe {
        GetTokenInformation(
            token.raw(),
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    }
    .ok()?;
    Some(elevation.TokenIsElevated != 0)
}
