//! Startup entries: Run/RunOnce registry keys and Startup folders.
//!
//! Enable/disable uses the same mechanism as Task Manager: a 12-byte value in
//! `...\Explorer\StartupApproved\{Run,Run32,StartupFolder}` whose first byte is
//! 2 (enabled) or 3 (disabled). The original Run value or shortcut is never
//! modified or deleted, so a change is always reversible.

use std::path::PathBuf;
use std::time::SystemTime;

use ::windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR};
use ::windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_BINARY,
    REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE, RegCreateKeyExW, RegEnumValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use ::windows::core::{HSTRING, Owned, PCWSTR, PWSTR};

use super::files::{expand_env, file_version_info};
use super::util::wide_to_string;
use crate::core::startup::{
    StartupEntry, StartupSource, StartupStatus, approved_value, executable_from_command, status_from_approved,
};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_ONCE: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
const RUN_32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

struct Location {
    root: HKEY,
    root_name: &'static str,
    key: &'static str,
    /// StartupApproved subkey, if the source supports toggling.
    approved: Option<&'static str>,
}

fn registry_location(source: StartupSource) -> Option<Location> {
    let (root, root_name) = if source.is_machine_wide() {
        (HKEY_LOCAL_MACHINE, "HKLM")
    } else {
        (HKEY_CURRENT_USER, "HKCU")
    };
    let (key, approved) = match source {
        StartupSource::RunUser | StartupSource::RunMachine => (RUN, Some("Run")),
        StartupSource::RunOnceUser | StartupSource::RunOnceMachine => (RUN_ONCE, None),
        StartupSource::RunMachine32 => (RUN_32, Some("Run32")),
        StartupSource::FolderUser | StartupSource::FolderCommon => return None,
    };
    Some(Location {
        root,
        root_name,
        key,
        approved,
    })
}

fn approved_root(source: StartupSource) -> HKEY {
    if source.is_machine_wide() {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    }
}

fn approved_subkey(source: StartupSource) -> Option<&'static str> {
    match source {
        StartupSource::FolderUser | StartupSource::FolderCommon => Some("StartupFolder"),
        other => registry_location(other).and_then(|l| l.approved),
    }
}

fn folder(source: StartupSource) -> Option<PathBuf> {
    let (var, rel) = match source {
        StartupSource::FolderUser => ("APPDATA", r"Microsoft\Windows\Start Menu\Programs\Startup"),
        StartupSource::FolderCommon => ("ProgramData", r"Microsoft\Windows\Start Menu\Programs\StartUp"),
        _ => return None,
    };
    std::env::var_os(var).map(|base| PathBuf::from(base).join(rel))
}

fn open_key(
    root: HKEY,
    path: &str,
    access: ::windows::Win32::System::Registry::REG_SAM_FLAGS,
) -> Result<Owned<HKEY>, WIN32_ERROR> {
    let mut key = HKEY::default();
    let sub = HSTRING::from(path);
    // SAFETY: valid predefined root, NUL-terminated subkey and out-pointer.
    let rc = unsafe { RegOpenKeyExW(root, &sub, None, access | KEY_WOW64_64KEY, &mut key) };
    if rc == ERROR_SUCCESS {
        // SAFETY: the opened key is owned and closed by Owned.
        Ok(unsafe { Owned::new(key) })
    } else {
        Err(rc)
    }
}

fn read_value(key: &Owned<HKEY>, name: &str) -> Option<Vec<u8>> {
    let value = HSTRING::from(name);
    let mut size = 0u32;
    // SAFETY: size query.
    if unsafe { RegQueryValueExW(**key, &value, None, None, None, Some(&mut size)) } != ERROR_SUCCESS {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `data` has `size` writable bytes.
    let rc = unsafe {
        RegQueryValueExW(
            **key,
            &value,
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&mut size),
        )
    };
    (rc == ERROR_SUCCESS).then(|| {
        data.truncate(size as usize);
        data
    })
}

fn approved_status(source: StartupSource, name: &str) -> StartupStatus {
    let Some(sub) = approved_subkey(source) else {
        return StartupStatus::Enabled;
    };
    let data = open_key(approved_root(source), &format!(r"{APPROVED}\{sub}"), KEY_READ)
        .ok()
        .and_then(|k| read_value(&k, name));
    status_from_approved(data.as_deref())
}

fn utf16_bytes_to_string(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    wide_to_string(&units)
}

fn enrich(entry: &mut StartupEntry) {
    if let Some(path) = &entry.path
        && let Some(info) = file_version_info(path)
    {
        entry.publisher = info.company;
        entry.description = info.description;
    }
}

fn registry_entries(source: StartupSource, out: &mut Vec<StartupEntry>) {
    let Some(loc) = registry_location(source) else {
        return;
    };
    let Ok(key) = open_key(loc.root, loc.key, KEY_READ) else {
        return;
    };
    let mut name_buf = vec![0u16; 16_384];
    let mut index = 0u32;
    loop {
        let mut name_len = name_buf.len() as u32;
        let mut kind = 0u32;
        let mut data_len = 0u32;
        // SAFETY: buffers and lengths match; data size is queried first.
        let rc = unsafe {
            RegEnumValueW(
                *key,
                index,
                Some(PWSTR(name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut kind),
                None,
                Some(&mut data_len),
            )
        };
        if rc == ERROR_NO_MORE_ITEMS {
            break;
        }
        index += 1;
        if rc != ERROR_SUCCESS {
            continue;
        }
        let name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
        let kind = REG_VALUE_TYPE(kind);
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            continue;
        }
        let Some(data) = read_value(&key, &name) else {
            continue;
        };
        let command = utf16_bytes_to_string(&data);
        let path = executable_from_command(&command).map(|p| expand_env(&p));
        let mut entry = StartupEntry {
            status: approved_status(source, &name),
            name,
            source,
            location: format!(r"{}\{}", loc.root_name, loc.key),
            command: Some(command),
            path,
            publisher: None,
            description: None,
        };
        enrich(&mut entry);
        out.push(entry);
    }
}

fn folder_entries(source: StartupSource, out: &mut Vec<StartupEntry>) {
    let Some(dir) = folder(source) else { return };
    let Ok(read) = std::fs::read_dir(&dir) else { return };
    for item in read.flatten() {
        let file_name = item.file_name().to_string_lossy().into_owned();
        if file_name.eq_ignore_ascii_case("desktop.ini") {
            continue;
        }
        let path = item.path().to_string_lossy().into_owned();
        let mut entry = StartupEntry {
            status: approved_status(source, &file_name),
            name: file_name,
            source,
            location: dir.to_string_lossy().into_owned(),
            command: None,
            path: Some(path),
            publisher: None,
            description: None,
        };
        // Shortcut targets would need the COM IShellLink API; only direct
        // executables get version information.
        if entry.name.to_ascii_lowercase().ends_with(".exe") {
            enrich(&mut entry);
        }
        out.push(entry);
    }
}

pub fn list_startup() -> Result<Vec<StartupEntry>, String> {
    let mut out = Vec::new();
    for source in StartupSource::ALL {
        match source {
            StartupSource::FolderUser | StartupSource::FolderCommon => folder_entries(source, &mut out),
            _ => registry_entries(source, &mut out),
        }
    }
    Ok(out)
}

fn filetime_now() -> u64 {
    // 100 ns intervals since 1601-01-01.
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    let since_unix = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| (d.as_nanos() / 100) as u64);
    since_unix + EPOCH_DIFF
}

pub fn set_startup_status(source: StartupSource, name: &str, status: StartupStatus) -> Result<(), String> {
    let sub = approved_subkey(source).ok_or("This entry type cannot be enabled or disabled")?;
    let path = HSTRING::from(format!(r"{APPROVED}\{sub}"));
    let mut key = HKEY::default();
    // SAFETY: valid root, subkey and out-pointer; creates the key if missing.
    let rc = unsafe {
        RegCreateKeyExW(
            approved_root(source),
            &path,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_WOW64_64KEY,
            None,
            &mut key,
            None,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(if rc == ::windows::Win32::Foundation::ERROR_ACCESS_DENIED {
            "Access denied. Changing entries for all users requires administrator rights.".into()
        } else {
            format!("Registry error {}", rc.0)
        });
    }
    // SAFETY: the created key is owned.
    let key = unsafe { Owned::new(key) };
    let value = approved_value(status, filetime_now());
    let name = HSTRING::from(name);
    // SAFETY: valid key; `value` is the REG_BINARY payload.
    let rc = unsafe { RegSetValueExW(*key, &name, None, REG_BINARY, Some(&value)) };
    if rc == ERROR_SUCCESS {
        Ok(())
    } else if rc == ERROR_FILE_NOT_FOUND {
        Err("The startup entry no longer exists".into())
    } else {
        Err(format!("Registry error {}", rc.0))
    }
}
