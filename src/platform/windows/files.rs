//! File metadata: version resources and environment expansion.

use ::windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use ::windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use ::windows::core::HSTRING;

use crate::platform::FileVersionInfo;

/// Expands `%VAR%` references (e.g. `%ProgramFiles%`).
pub fn expand_env(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let src = HSTRING::from(text);
    // SAFETY: size query with no output buffer.
    let needed = unsafe { ExpandEnvironmentStringsW(&src, None) } as usize;
    if needed == 0 {
        return text.to_owned();
    }
    let mut buf = vec![0u16; needed];
    // SAFETY: `buf` has the size the API asked for.
    let written = unsafe { ExpandEnvironmentStringsW(&src, Some(&mut buf)) } as usize;
    if written == 0 || written > buf.len() {
        return text.to_owned();
    }
    super::util::wide_to_string(&buf)
}

/// Reads one string from a version resource block.
fn query_string(block: &[u8], sub_block: &str) -> Option<String> {
    let mut ptr = std::ptr::null_mut();
    let mut len = 0u32;
    let key = HSTRING::from(sub_block);
    // SAFETY: `block` is a complete version resource returned by
    // GetFileVersionInfoW; the returned pointer points inside it.
    let ok = unsafe { VerQueryValueW(block.as_ptr().cast(), &key, &mut ptr, &mut len) }.as_bool();
    if !ok || ptr.is_null() || len == 0 {
        return None;
    }
    // SAFETY: string values are `len` UTF-16 units (including NUL) inside `block`.
    let units = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), len as usize) };
    let s = super::util::wide_to_string(units).trim().to_owned();
    (!s.is_empty()).then_some(s)
}

/// Reads CompanyName/FileDescription/FileVersion from a PE version resource,
/// using the first language/code page listed in `\VarFileInfo\Translation`.
/// <https://learn.microsoft.com/windows/win32/api/winver/nf-winver-verqueryvaluew>
pub fn file_version_info(path: &str) -> Option<FileVersionInfo> {
    let file = HSTRING::from(path);
    // SAFETY: valid path string.
    let size = unsafe { GetFileVersionInfoSizeW(&file, None) };
    if size == 0 {
        return None;
    }
    let mut block = vec![0u8; size as usize];
    // SAFETY: `block` has `size` writable bytes.
    unsafe { GetFileVersionInfoW(&file, None, size, block.as_mut_ptr().cast()) }.ok()?;

    let mut ptr = std::ptr::null_mut();
    let mut len = 0u32;
    let key = HSTRING::from(r"\VarFileInfo\Translation");
    // SAFETY: as in `query_string`.
    let ok = unsafe { VerQueryValueW(block.as_ptr().cast(), &key, &mut ptr, &mut len) }.as_bool();
    let prefix = if ok && !ptr.is_null() && len >= 4 {
        // SAFETY: the translation table holds (language, code page) u16 pairs.
        let pair = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), 2) };
        format!(r"\StringFileInfo\{:04x}{:04x}", pair[0], pair[1])
    } else {
        // US English, Unicode: the most common fallback.
        r"\StringFileInfo\040904b0".to_owned()
    };
    let info = FileVersionInfo {
        company: query_string(&block, &format!(r"{prefix}\CompanyName")),
        description: query_string(&block, &format!(r"{prefix}\FileDescription")),
        version: query_string(&block, &format!(r"{prefix}\FileVersion")),
    };
    (info.company.is_some() || info.description.is_some() || info.version.is_some()).then_some(info)
}
