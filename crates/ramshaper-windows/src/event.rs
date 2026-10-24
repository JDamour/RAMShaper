//! Manual-reset events used as stop/cancel signals.
//!
//! Distinct from `console::ConsoleCancel` (which additionally owns the
//! Ctrl+C handler): this is a plain event any thread can trigger, used for
//! agent lifecycle and collector shutdown. Implements the core
//! [`WaitHandle`] contract so watch loops can wait on it.

use ramshaper_core::watch::{WaitHandle, WaitOutcome};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Threading::{
    CreateEventExW, SetEvent, WaitForSingleObjectEx, CREATE_EVENT_MANUAL_RESET, EVENT_ALL_ACCESS,
};

use crate::handle::OwnedHandle;

/// Manual-reset event (starts unsignaled).
pub struct ManualResetEvent {
    handle: OwnedHandle,
}

impl ManualResetEvent {
    /// Create an unnamed manual-reset event.
    pub fn new() -> Result<Self, ramshaper_core::snapshot::QueryFailure> {
        let failure = |function: &'static str, e: windows::core::Error| {
            crate::error::to_query_failure(function, &e)
        };
        // SAFETY: manual-reset, unnamed; windows-rs validates the handle.
        let handle = unsafe {
            CreateEventExW(
                None,
                PCWSTR::null(),
                CREATE_EVENT_MANUAL_RESET,
                EVENT_ALL_ACCESS.0,
            )
        }
        .map_err(|e| failure("CreateEventExW", e))?;
        // SAFETY: valid owned event handle.
        Ok(Self {
            handle: unsafe { OwnedHandle::adopt(handle) },
        })
    }

    /// Signal the event (latches; all waiters wake).
    pub fn trigger(&self) {
        // SAFETY: owned, valid event handle.
        unsafe {
            let _ = SetEvent(self.handle.raw());
        }
    }

    /// Raw handle (for multi-object waits).
    pub fn raw(&self) -> windows::Win32::Foundation::HANDLE {
        self.handle.raw()
    }
}

impl WaitHandle for ManualResetEvent {
    fn wait(&self, timeout_ms: u32) -> WaitOutcome {
        // SAFETY: owned event handle.
        let result = unsafe { WaitForSingleObjectEx(self.handle.raw(), timeout_ms, false) };
        if result == WAIT_OBJECT_0 {
            WaitOutcome::Signaled
        } else {
            debug_assert_eq!(result, WAIT_TIMEOUT, "unexpected wait result {result:?}");
            WaitOutcome::TimedOut
        }
    }
}

impl std::fmt::Debug for ManualResetEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManualResetEvent")
            .field("handle", &self.handle)
            .finish()
    }
}
