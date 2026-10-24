//! Windows process identity: PID plus the exact creation FILETIME.
//!
//! Identity is `(pid, creation FILETIME in 100-ns units)` from `GetProcessTimes`
//! on a handle opened with `PROCESS_QUERY_LIMITED_INFORMATION` only (contract
//! 14 §1, ADR-008). The creation value is preserved at full 64-bit precision
//! and is an identity marker, not a monotonic clock. Session context is
//! queried separately and deliberately kept out of the identity type.
//!
//! Exit, PID reuse and access denial are expected outcomes:
//! - `OpenProcess` failing with `ERROR_INVALID_PARAMETER` means the PID no
//!   longer resolves (process exited / not yet visible) -> [`QueryFailure::Exited`].
//! - `OpenProcess` failing with `ERROR_ACCESS_DENIED` (protected process)
//!   -> [`QueryFailure::AccessDenied`].
//! - A resolved PID whose creation FILETIME differs from the expected one is
//!   a *different process instance* (reuse); see [`verify_identity`].

use ramshaper_core::identity::{IdentityVerification, ProcessIdentity};
use ramshaper_core::snapshot::QueryFailure;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::error::open_process_failure;
use crate::handle::OwnedHandle;

/// `GetExitCodeProcess` sentinel meaning "has not exited yet".
const STILL_ACTIVE: u32 = 259;

/// Open a query-only handle for `pid`. Minimal rights: no process setting can
/// be changed through this handle.
fn open_query_handle(pid: u32) -> Result<OwnedHandle, QueryFailure> {
    // SAFETY: plain call with a numeric PID; inherit=false. windows-rs
    // returns Err for a null/invalid handle, so adoption below is sound.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|e| open_process_failure(&e))?;
    // SAFETY: OpenProcess returned a valid owned handle.
    Ok(unsafe { OwnedHandle::adopt(handle) })
}

/// Combine a FILETIME pair into the exact 64-bit 100-ns value (no precision
/// loss, no arithmetic on the identity marker).
fn filetime_to_u64(ft: &FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

/// Read the creation FILETIME through an open handle.
fn creation_filetime(handle: &OwnedHandle) -> Result<u64, QueryFailure> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: four valid out-pointers of the documented type; the handle is
    // owned and query-only. Exit time may be zero for a live process; the
    // creation time is what we keep.
    unsafe {
        GetProcessTimes(
            handle.raw(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
        .map_err(|e| QueryFailure::OsError {
            code: crate::error::win32_code(&e),
            function: "GetProcessTimes",
        })?;
    }
    Ok(filetime_to_u64(&creation))
}

/// Resolve the current identity of `pid`.
pub fn resolve_identity(pid: u32) -> Result<ProcessIdentity, QueryFailure> {
    let handle = open_query_handle(pid)?;
    Ok(ProcessIdentity::new(pid, creation_filetime(&handle)?))
}

/// Identity of the calling process, queried through the current-process
/// pseudo-handle (which is never closed).
pub fn current_process_identity() -> Result<ProcessIdentity, QueryFailure> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: GetCurrentProcess returns a pseudo-handle valid for the
    // process lifetime (must not be closed); out-pointers are valid.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
        .map_err(|e| QueryFailure::OsError {
            code: crate::error::win32_code(&e),
            function: "GetProcessTimes",
        })?;
    }
    Ok(ProcessIdentity::new(
        unsafe { GetCurrentProcessId() },
        filetime_to_u64(&creation),
    ))
}

/// Re-check `expected` against current OS state (AT-03), using one handle
/// for the whole verification sequence.
///
/// A terminated process can remain *queryable* (same creation FILETIME)
/// until the OS retires its object, so identity equality alone is not
/// liveness: the exit code is checked through the same handle. Outcomes:
/// - same creation marker and still running -> [`IdentityVerification::Matches`];
/// - same PID, different creation marker (PID reuse) ->
///   [`IdentityVerification::Mismatch`] — the replacement must never inherit
///   history or authority;
/// - exited, PID gone, access denied or an OS error ->
///   [`IdentityVerification::Unreachable`].
pub fn verify_identity(expected: &ProcessIdentity) -> IdentityVerification {
    let handle = match open_query_handle(expected.pid) {
        Ok(handle) => handle,
        Err(reason) => return IdentityVerification::Unreachable { reason },
    };
    let creation = match creation_filetime(&handle) {
        Ok(creation) => creation,
        Err(reason) => return IdentityVerification::Unreachable { reason },
    };
    let observed = ProcessIdentity::new(expected.pid, creation);
    if !expected.matches(&observed) {
        return IdentityVerification::Mismatch { observed };
    }
    // Identity matches; confirm the process is still alive through the same
    // handle (exit code becomes non-STILL_ACTIVE as soon as it terminates).
    let mut exit_code: u32 = 0;
    // SAFETY: valid out-pointer; owned query-only handle.
    let alive = unsafe {
        GetExitCodeProcess(handle.raw(), &mut exit_code).map(|()| exit_code == STILL_ACTIVE)
    };
    match alive {
        Ok(true) => IdentityVerification::Matches,
        Ok(false) => IdentityVerification::Unreachable {
            reason: QueryFailure::Exited { code: exit_code },
        },
        Err(e) => IdentityVerification::Unreachable {
            reason: QueryFailure::OsError {
                code: crate::error::win32_code(&e),
                function: "GetExitCodeProcess",
            },
        },
    }
}

/// Session id of `pid` — authorization *context*, deliberately separate from
/// identity (contract 14 §1). Returned as its own typed value so it can never
/// be mistaken for part of the identity tuple.
pub fn session_id(pid: u32) -> Result<u32, QueryFailure> {
    let mut session: u32 = 0;
    // SAFETY: valid out-pointer; numeric PID.
    unsafe {
        ProcessIdToSessionId(pid, &mut session).map_err(|e| QueryFailure::OsError {
            code: crate::error::win32_code(&e),
            function: "ProcessIdToSessionId",
        })?;
    }
    Ok(session)
}
