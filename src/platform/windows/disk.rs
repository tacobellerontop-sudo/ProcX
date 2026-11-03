//! Physical disk counters via `IOCTL_DISK_PERFORMANCE`.
//!
//! The device `\\.\PhysicalDriveN` is opened with *zero* desired access,
//! which is sufficient for this query and needs no administrator rights.
//! `DISK_PERFORMANCE` reports cumulative bytes and an `IdleTime` alongside the
//! `QueryTime` at which the counters were read (both 100 ns units), from which
//! active time is derived as `1 - ΔIdleTime / ΔQueryTime`.
//! <https://learn.microsoft.com/windows/win32/api/winioctl/ns-winioctl-disk_performance>

use std::mem::size_of;

use ::windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use ::windows::Win32::System::IO::DeviceIoControl;
use ::windows::Win32::System::Ioctl::{DISK_PERFORMANCE, IOCTL_DISK_PERFORMANCE};
use ::windows::core::HSTRING;

use super::handle::OwnedHandle;
use crate::platform::PhysicalDiskCounters;

pub fn physical_disk_counters(index: u32) -> Option<PhysicalDiskCounters> {
    let path = HSTRING::from(format!(r"\\.\PhysicalDrive{index}"));
    // SAFETY: `path` is a valid NUL-terminated wide string; the handle is owned.
    let raw = unsafe {
        CreateFileW(
            &path,
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .ok()?;
    let device = OwnedHandle::new(raw)?;
    let mut perf = DISK_PERFORMANCE::default();
    let mut returned = 0u32;
    // SAFETY: output buffer is a DISK_PERFORMANCE of the stated size; no input buffer.
    unsafe {
        DeviceIoControl(
            device.raw(),
            IOCTL_DISK_PERFORMANCE,
            None,
            0,
            Some((&raw mut perf).cast()),
            size_of::<DISK_PERFORMANCE>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .ok()?;
    Some(PhysicalDiskCounters {
        index,
        bytes_read: perf.BytesRead.max(0) as u64,
        bytes_written: perf.BytesWritten.max(0) as u64,
        idle_time: perf.IdleTime.max(0) as u64,
        query_time: perf.QueryTime.max(0) as u64,
    })
}
