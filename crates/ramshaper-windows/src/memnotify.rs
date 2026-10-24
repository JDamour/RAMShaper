//! Low-memory resource notification object.
//!
//! `CreateMemoryResourceNotification(LowMemoryResourceNotification)` creates
//! an OS object whose signaled state means "available physical memory is
//! low". The state can remain signaled for a long time, so consumers must
//! *query* it on a rate-limited cadence instead of waiting on it
//! unconditionally (doc 04 §B; the watch engine in `ramshaper-core` does
//! exactly that). The handle is closed exactly once via RAII.

use ramshaper_core::envelope::LowMemorySignal;
use windows::Win32::System::Memory::{
    CreateMemoryResourceNotification, LowMemoryResourceNotification,
    QueryMemoryResourceNotification,
};

use crate::handle::OwnedHandle;

/// Owned low-memory resource notification object.
pub struct MemoryResourceNotification {
    handle: OwnedHandle,
}

impl MemoryResourceNotification {
    /// Create the OS low-memory notification object. Failure is possible on
    /// resource exhaustion and is surfaced with its error code.
    pub fn create_low() -> Result<Self, windows::core::Error> {
        // SAFETY: plain call; the returned handle is checked by windows-rs
        // (invalid handle -> Err) before we adopt it for RAII ownership.
        let handle = unsafe { CreateMemoryResourceNotification(LowMemoryResourceNotification)? };
        // SAFETY: CreateMemoryResourceNotification returned a valid owned handle.
        Ok(Self {
            handle: unsafe { OwnedHandle::adopt(handle) },
        })
    }

    /// Query the current condition state. This is a non-blocking query, not
    /// a wait. `Signaled` is an OS resource condition; it does not by itself
    /// prove a user-visible slowdown.
    pub fn query(&self) -> Result<LowMemorySignal, windows::core::Error> {
        let mut state = windows::core::BOOL::default();
        // SAFETY: `state` is a valid out-pointer of the documented type and
        // the handle is owned and never closed while borrowed.
        unsafe { QueryMemoryResourceNotification(self.handle.raw(), &mut state)? };
        Ok(if state.as_bool() {
            LowMemorySignal::Signaled
        } else {
            LowMemorySignal::NotSignaled
        })
    }
}
