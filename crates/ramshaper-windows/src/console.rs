//! Ctrl+C cancellation for watch loops.
//!
//! `SetConsoleCtrlHandler` delivers console events on a separate thread, and
//! its callback cannot capture context, so the cancel event handle is stored
//! in a process-global slot. The handler sets the event for Ctrl+C, Ctrl+Break,
//! close, logoff and shutdown; the watch loop waits on that event (never on
//! OS condition objects), making every wait cancellable (RS-006).

use std::sync::atomic::{AtomicUsize, Ordering};

use ramshaper_core::watch::{WaitHandle, WaitOutcome};
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::Threading::{
    CreateEventExW, SetEvent, WaitForSingleObjectEx, CREATE_EVENT_MANUAL_RESET, EVENT_ALL_ACCESS,
};

use crate::handle::OwnedHandle;

static CANCEL_EVENT: AtomicUsize = AtomicUsize::new(0);

/// SAFETY (audit note): runs on a console-control thread; only touches the
/// atomic slot and `SetEvent`, which are signal-safe here.
unsafe extern "system" fn ctrl_handler(_ctrl_type: u32) -> BOOL {
    let stored = CANCEL_EVENT.load(Ordering::SeqCst);
    if stored != 0 {
        // SAFETY: the stored value is a valid event handle published by
        // `install` and kept alive by the `ConsoleCancel` owner.
        unsafe {
            let _ = SetEvent(windows::Win32::Foundation::HANDLE(stored as *mut _));
        }
    }
    BOOL(1) // handled: suppress default termination; the loop exits cleanly.
}

/// Installed console cancellation. Setting the OS handler is paired with its
/// removal in `Drop`; the event handle is RAII-owned.
pub struct ConsoleCancel {
    event: OwnedHandle,
}

impl ConsoleCancel {
    /// Create the cancel event and install the console handler. Only one
    /// instance may exist per process at a time.
    pub fn install() -> Result<Self, ramshaper_core::snapshot::QueryFailure> {
        let failure = |function: &'static str, e: windows::core::Error| {
            crate::error::to_query_failure(function, &e)
        };
        // SAFETY: manual-reset, unnamed, not-inheritable event via
        // CreateEventExW (no security descriptor; null attributes); windows-rs
        // validates the returned handle.
        let handle = unsafe {
            CreateEventExW(
                None,
                PCWSTR::null(),
                CREATE_EVENT_MANUAL_RESET,
                EVENT_ALL_ACCESS.0,
            )
        }
        .map_err(|e| failure("CreateEventExW", e))?;
        // SAFETY: valid owned event handle from CreateEventW.
        let event = unsafe { OwnedHandle::adopt(handle) };
        CANCEL_EVENT.store(event.raw().0 as usize, Ordering::SeqCst);
        // SAFETY: plain call with our static handler.
        if let Err(e) = unsafe { SetConsoleCtrlHandler(Some(ctrl_handler), true) } {
            CANCEL_EVENT.store(0, Ordering::SeqCst);
            return Err(failure("SetConsoleCtrlHandler", e));
        }
        Ok(Self { event })
    }

    /// Programmatically trigger cancellation (used by tests and future
    /// programmatic stops).
    pub fn trigger(&self) {
        // SAFETY: owned, valid event handle.
        unsafe {
            let _ = SetEvent(self.event.raw());
        }
    }
}

impl WaitHandle for ConsoleCancel {
    fn wait(&self, timeout_ms: u32) -> WaitOutcome {
        // SAFETY: owned event handle; alertable wait not required.
        let result = unsafe { WaitForSingleObjectEx(self.event.raw(), timeout_ms, false) };
        if result == WAIT_OBJECT_0 {
            WaitOutcome::Signaled
        } else {
            debug_assert_eq!(result, WAIT_TIMEOUT, "unexpected wait result {result:?}");
            WaitOutcome::TimedOut
        }
    }
}

impl Drop for ConsoleCancel {
    fn drop(&mut self) {
        // Remove our handler and clear the slot before the event handle is
        // closed (field drop order: explicit removal happens first).
        // SAFETY: plain call removing the handler registered above.
        unsafe {
            let _ = SetConsoleCtrlHandler(None, true);
        }
        CANCEL_EVENT.store(0, Ordering::SeqCst);
    }
}
