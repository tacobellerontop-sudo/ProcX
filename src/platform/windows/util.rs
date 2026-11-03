//! Small conversions shared by the Windows backend.

use std::time::{Duration, SystemTime};

use ::windows::Win32::Foundation::{ERROR_ACCESS_DENIED, FILETIME};

/// 100 ns intervals between 1601-01-01 and 1970-01-01.
const UNIX_EPOCH_AS_FILETIME: u64 = 116_444_736_000_000_000;

pub fn filetime_u64(ft: FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

pub fn filetime_to_system_time(ticks: u64) -> Option<SystemTime> {
    let since_unix = ticks.checked_sub(UNIX_EPOCH_AS_FILETIME)?;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_nanos(since_unix.saturating_mul(100)))
}

/// Converts a NUL-terminated (or full) UTF-16 buffer.
pub fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

pub fn is_access_denied(err: &::windows::core::Error) -> bool {
    err.code() == ERROR_ACCESS_DENIED.to_hresult()
}

/// User-facing description of a Win32 error.
pub fn describe(err: &::windows::core::Error) -> String {
    if is_access_denied(err) {
        "Access denied".to_owned()
    } else {
        let msg = err.message();
        if msg.is_empty() {
            format!("{:?}", err.code())
        } else {
            msg.trim().to_owned()
        }
    }
}
