//! Windows services via the Service Control Manager.
//!
//! Enumeration needs only `SC_MANAGER_ENUMERATE_SERVICE`, which standard
//! users have. Per-service configuration is read with `SERVICE_QUERY_CONFIG`
//! (also granted to standard users for most services; failures leave the
//! fields unavailable). Starting, stopping and reconfiguring use the specific
//! right for that action only, and typically require administrator rights.

use ::windows::Win32::System::Services::{
    ChangeServiceConfig2W, ChangeServiceConfigW, ControlService, ENUM_SERVICE_STATUS_PROCESSW,
    ENUM_SERVICE_TYPE, EnumServicesStatusExW, OpenSCManagerW, OpenServiceW, QUERY_SERVICE_CONFIGW,
    QueryServiceConfig2W, QueryServiceConfigW, SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_CONNECT,
    SC_MANAGER_ENUMERATE_SERVICE, SERVICE_AUTO_START, SERVICE_BOOT_START, SERVICE_CHANGE_CONFIG,
    SERVICE_CONFIG, SERVICE_CONFIG_DELAYED_AUTO_START_INFO, SERVICE_CONFIG_DESCRIPTION, SERVICE_CONTROL_STOP,
    SERVICE_DELAYED_AUTO_START_INFO, SERVICE_DEMAND_START, SERVICE_DESCRIPTIONW, SERVICE_DISABLED,
    SERVICE_ERROR, SERVICE_NO_CHANGE, SERVICE_QUERY_CONFIG, SERVICE_START, SERVICE_START_TYPE,
    SERVICE_STATE_ALL, SERVICE_STATUS, SERVICE_STOP, SERVICE_SYSTEM_START, SERVICE_WIN32, StartServiceW,
};
use ::windows::core::{HSTRING, Owned, PCWSTR, PWSTR};

use super::util::describe;
use crate::core::services::{ServiceControl, ServiceInfo, ServiceState, StartType};

/// `SERVICE_ACCEPT_STOP` from winsvc.h.
const SERVICE_ACCEPT_STOP: u32 = 0x1;
/// `SERVICE_WIN32_SHARE_PROCESS` from winnt.h.
const SERVICE_WIN32_SHARE_PROCESS: u32 = 0x20;
const ERROR_MORE_DATA: u32 = 234;

fn pwstr_to_string(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the SCM returns NUL-terminated strings inside the caller's buffer.
    unsafe { p.to_string() }.ok()
}

/// Splits a REG_MULTI_SZ-style double-NUL-terminated list.
fn multi_sz(p: PWSTR) -> Vec<String> {
    let mut out = Vec::new();
    if p.is_null() {
        return out;
    }
    let mut cur = p.0.cast_const();
    loop {
        // SAFETY: each element is NUL-terminated and the list ends with an
        // empty string, all inside the buffer returned by the SCM.
        let len = (0..).take_while(|&i| unsafe { *cur.add(i) } != 0).count();
        if len == 0 {
            break;
        }
        // SAFETY: `len` units were just verified readable.
        out.push(String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(cur, len)
        }));
        // SAFETY: skip the element and its terminator.
        cur = unsafe { cur.add(len + 1) };
    }
    out
}

fn open_manager(access: u32) -> Result<Owned<SC_HANDLE>, String> {
    // SAFETY: null machine/database names select the local active database.
    unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access) }
        .map(|h| unsafe { Owned::new(h) })
        .map_err(|e| format!("Cannot open the Service Control Manager: {}", describe(&e)))
}

fn open_service(
    scm: &Owned<SC_HANDLE>,
    name: &str,
    access: u32,
) -> Result<Owned<SC_HANDLE>, ::windows::core::Error> {
    let name = HSTRING::from(name);
    // SAFETY: valid SCM handle and NUL-terminated name; handle ownership taken.
    unsafe { OpenServiceW(**scm, &name, access) }.map(|h| unsafe { Owned::new(h) })
}

/// Calls a "query into caller buffer" API, growing the buffer as requested.
fn query_buffer(
    mut call: impl FnMut(Option<&mut [u8]>, &mut u32) -> ::windows::core::Result<()>,
) -> Option<Vec<u64>> {
    let mut needed = 0u32;
    let _ = call(None, &mut needed);
    if needed == 0 {
        return None;
    }
    // u64 storage keeps the returned structures aligned.
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: viewing an initialized u64 buffer as bytes is always valid.
    let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 8) };
    call(Some(bytes), &mut needed).ok()?;
    Some(buf)
}

struct Config {
    start_type: StartType,
    binary_path: Option<String>,
    account: Option<String>,
    dependencies: Vec<String>,
}

fn read_config(service: &Owned<SC_HANDLE>) -> Option<Config> {
    let buf = query_buffer(|bytes, needed| {
        let len = bytes.as_ref().map_or(0, |b| b.len()) as u32;
        let ptr = bytes.map(|b| b.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>());
        // SAFETY: `ptr` (if any) points to `len` writable, aligned bytes.
        unsafe { QueryServiceConfigW(**service, ptr, len, needed) }
    })?;
    // SAFETY: on success the buffer starts with QUERY_SERVICE_CONFIGW whose
    // string pointers point into the same buffer.
    let cfg = unsafe { &*buf.as_ptr().cast::<QUERY_SERVICE_CONFIGW>() };
    let delayed = read_config2(service, SERVICE_CONFIG_DELAYED_AUTO_START_INFO).is_some_and(|b| {
        // SAFETY: buffer holds SERVICE_DELAYED_AUTO_START_INFO.
        unsafe {
            (*b.as_ptr().cast::<SERVICE_DELAYED_AUTO_START_INFO>())
                .fDelayedAutostart
                .as_bool()
        }
    });
    let start_type = match cfg.dwStartType {
        SERVICE_BOOT_START => StartType::Boot,
        SERVICE_SYSTEM_START => StartType::System,
        SERVICE_AUTO_START if delayed => StartType::AutomaticDelayed,
        SERVICE_AUTO_START => StartType::Automatic,
        SERVICE_DISABLED => StartType::Disabled,
        _ => StartType::Manual,
    };
    Some(Config {
        start_type,
        binary_path: pwstr_to_string(cfg.lpBinaryPathName).filter(|s| !s.is_empty()),
        account: pwstr_to_string(cfg.lpServiceStartName).filter(|s| !s.is_empty()),
        dependencies: multi_sz(cfg.lpDependencies),
    })
}

fn read_config2(service: &Owned<SC_HANDLE>, level: SERVICE_CONFIG) -> Option<Vec<u64>> {
    query_buffer(|bytes, needed| {
        // SAFETY: buffer slice (if any) is writable.
        unsafe { QueryServiceConfig2W(**service, level, bytes, needed) }
    })
}

fn read_description(service: &Owned<SC_HANDLE>) -> Option<String> {
    let buf = read_config2(service, SERVICE_CONFIG_DESCRIPTION)?;
    // SAFETY: buffer holds SERVICE_DESCRIPTIONW pointing into itself.
    let desc = unsafe { &*buf.as_ptr().cast::<SERVICE_DESCRIPTIONW>() };
    pwstr_to_string(desc.lpDescription)
        .filter(|s| !s.is_empty())
        // Some descriptions are indirect resource strings ("@%SystemRoot%\...dll,-100").
        .filter(|s| !s.starts_with('@'))
}

pub fn list_services() -> Result<Vec<ServiceInfo>, String> {
    let scm = open_manager(SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE)?;
    let mut all = Vec::new();
    let mut resume = 0u32;
    loop {
        let mut needed = 0u32;
        let mut returned = 0u32;
        // First call sizes the buffer.
        // SAFETY: valid handle and out-pointers.
        let _ = unsafe {
            EnumServicesStatusExW(
                *scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                None,
                &mut needed,
                &mut returned,
                Some(&mut resume),
                PCWSTR::null(),
            )
        };
        if needed == 0 {
            break;
        }
        let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: as above.
        let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 8) };
        // SAFETY: buffer is large enough per the sizing call (more data is handled by looping).
        let result = unsafe {
            EnumServicesStatusExW(
                *scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                Some(bytes),
                &mut needed,
                &mut returned,
                Some(&mut resume),
                PCWSTR::null(),
            )
        };
        let more = matches!(&result, Err(e) if e.code() == ::windows::Win32::Foundation::WIN32_ERROR(ERROR_MORE_DATA).to_hresult());
        if let Err(e) = &result
            && !more
        {
            return Err(format!("Cannot enumerate services: {}", describe(e)));
        }
        // SAFETY: the API wrote `returned` entries at the buffer start.
        let entries = unsafe {
            std::slice::from_raw_parts(
                buf.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(),
                returned as usize,
            )
        };
        for e in entries {
            let Some(name) = pwstr_to_string(e.lpServiceName) else {
                continue;
            };
            let status = &e.ServiceStatusProcess;
            let mut info = ServiceInfo {
                display_name: pwstr_to_string(e.lpDisplayName).unwrap_or_else(|| name.clone()),
                name,
                description: None,
                state: ServiceState::from_raw(status.dwCurrentState.0),
                start_type: None,
                pid: (status.dwProcessId != 0).then_some(status.dwProcessId),
                binary_path: None,
                account: None,
                dependencies: Vec::new(),
                shared_process: status.dwServiceType.0 & SERVICE_WIN32_SHARE_PROCESS != 0,
                can_stop: status.dwControlsAccepted & SERVICE_ACCEPT_STOP != 0,
            };
            if let Ok(service) = open_service(&scm, &info.name, SERVICE_QUERY_CONFIG) {
                if let Some(cfg) = read_config(&service) {
                    info.start_type = Some(cfg.start_type);
                    info.binary_path = cfg.binary_path;
                    info.account = cfg.account;
                    info.dependencies = cfg.dependencies;
                }
                info.description = read_description(&service);
            }
            all.push(info);
        }
        if !more {
            break;
        }
    }
    all.sort_by_key(|s| s.display_name.to_lowercase());
    Ok(all)
}

fn access_message(e: &::windows::core::Error, what: &str) -> String {
    if super::util::is_access_denied(e) {
        format!("Access denied. {what} requires administrator rights; restart ProcX as administrator.")
    } else {
        describe(e)
    }
}

pub fn control_service(name: &str, control: ServiceControl) -> Result<(), String> {
    let scm = open_manager(SC_MANAGER_CONNECT)?;
    match control {
        ServiceControl::Start => {
            let service = open_service(&scm, name, SERVICE_START)
                .map_err(|e| access_message(&e, "Starting services"))?;
            // SAFETY: valid handle, no arguments.
            unsafe { StartServiceW(*service, None) }.map_err(|e| access_message(&e, "Starting services"))
        }
        ServiceControl::Stop => {
            let service = open_service(&scm, name, SERVICE_STOP)
                .map_err(|e| access_message(&e, "Stopping services"))?;
            let mut status = SERVICE_STATUS::default();
            // SAFETY: valid handle and status out-pointer.
            unsafe { ControlService(*service, SERVICE_CONTROL_STOP, &mut status) }
                .map_err(|e| access_message(&e, "Stopping services"))
        }
    }
}

pub fn set_service_start_type(name: &str, start: StartType) -> Result<(), String> {
    let (raw, delayed) = match start {
        StartType::Automatic => (SERVICE_AUTO_START, false),
        StartType::AutomaticDelayed => (SERVICE_AUTO_START, true),
        StartType::Manual => (SERVICE_DEMAND_START, false),
        StartType::Disabled => (SERVICE_DISABLED, false),
        StartType::Boot | StartType::System => {
            return Err("Boot and System start types apply only to drivers".into());
        }
    };
    let scm = open_manager(SC_MANAGER_CONNECT)?;
    let service = open_service(&scm, name, SERVICE_CHANGE_CONFIG)
        .map_err(|e| access_message(&e, "Changing a service's startup type"))?;
    // SAFETY: valid handle; every other setting is left unchanged.
    unsafe {
        ChangeServiceConfigW(
            *service,
            ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
            SERVICE_START_TYPE(raw.0),
            SERVICE_ERROR(SERVICE_NO_CHANGE),
            PCWSTR::null(),
            PCWSTR::null(),
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
        )
    }
    .map_err(|e| access_message(&e, "Changing a service's startup type"))?;
    if raw == SERVICE_AUTO_START {
        let info = SERVICE_DELAYED_AUTO_START_INFO {
            fDelayedAutostart: delayed.into(),
        };
        // SAFETY: `info` is the structure required for this info level.
        unsafe {
            ChangeServiceConfig2W(
                *service,
                SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                Some((&raw const info).cast()),
            )
        }
        .map_err(|e| access_message(&e, "Changing a service's startup type"))?;
    }
    Ok(())
}
