//! RAII ownership for Windows kernel handles.
//!
//! The only place where `CloseHandle` is called. A handle is closed exactly
//! once, when the `OwnedHandle` is dropped; handles are never closed twice
//! and never leaked, including on error paths (drops run during `?`).

use std::fmt;
use windows::Win32::Foundation::{CloseHandle, HANDLE};

/// An owned Windows kernel handle. Constructed only by audited wrapper
/// modules after an API call returned a valid handle.
#[repr(transparent)]
pub struct OwnedHandle(HANDLE);

impl OwnedHandle {
    /// Adopt a handle that the caller has verified to be valid and owned.
    ///
    /// # Safety
    /// `handle` must be a valid, owned kernel handle that nothing else will
    /// close. Adopting the pseudo-handle from `GetCurrentProcess()` is not
    /// allowed (it must not be closed).
    pub unsafe fn adopt(handle: HANDLE) -> Self {
        debug_assert!(!handle.is_invalid(), "adopting an invalid handle");
        Self(handle)
    }

    /// Raw handle value for passing to Win32 APIs.
    pub fn raw(&self) -> HANDLE {
        self.0
    }

    /// True when the handle is the invalid sentinel `NULL`.
    pub fn is_invalid(&self) -> bool {
        self.0.is_invalid()
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the constructor guarantees a valid owned handle; CloseHandle
        // failing (already closed / invalid) is a bug we surface only in
        // debug builds — drop must not panic in OS-facing runtime paths.
        debug_assert!(!self.0.is_invalid());
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

// HANDLE is a pointer, but a kernel handle value is valid process-wide and
// safe to use from any thread; this type owns it uniquely.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl fmt::Debug for OwnedHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OwnedHandle({:?})", self.0)
    }
}
