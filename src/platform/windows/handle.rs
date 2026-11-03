//! RAII wrapper for Win32 kernel handles.

use ::windows::Win32::Foundation::{CloseHandle, HANDLE};

/// Owns a kernel handle and closes it on drop.
pub struct OwnedHandle(HANDLE);

impl OwnedHandle {
    /// Takes ownership of `handle`. Returns `None` for null/invalid handles.
    pub fn new(handle: HANDLE) -> Option<Self> {
        (!handle.is_invalid() && !handle.0.is_null()).then_some(Self(handle))
    }

    pub fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: we own the handle and it is closed exactly once here.
        // A failure to close cannot be meaningfully handled during drop.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
