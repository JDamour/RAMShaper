//! OS signal-source adapters for the eligibility matrix (R-03).
//!
//! The pure decision logic lives in `ramshaper-core::eligibility` (ADR-028);
//! this module is the tested OS signal side of the contract: each adapter
//! observes one signal through minimal-rights, documented APIs and maps every
//! failure to the observation's `Unknown` variant — which always denies
//! downstream. Nothing here writes a process setting: the capability probe
//! opens a handle with `PROCESS_SET_INFORMATION` and immediately closes it.
//!
//! Classification is deliberately narrow (doc 04: "do not claim universal
//! detection"): the only tested signal today is *self-identification* —
//! RAMShaper's own PID and its well-known image names. Every other target
//! classifies as `Unknown` (deny) until the RS-024 Stage B tested signal
//! matrix covers it.

use ramshaper_core::eligibility::{
    ChildObservation, FocusObservation, OwnerSessionObservation, PriorityCapability,
    PriorityClassObservation, ProtectedCategory, SafetyClassification,
};
use ramshaper_core::snapshot::QueryFailure;
use windows::Win32::System::Threading::{
    GetPriorityClass, OpenProcess, ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
    HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, REALTIME_PRIORITY_CLASS,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::error::open_process_failure;
use crate::handle::OwnedHandle;
use crate::{identity, processes, token};

/// Owner/session observation through the validated process token: the
/// target's user SID is compared against the current user's, and the target's
/// logon session against the current one. Any query failure (exit, access
/// denied, OS error) is `Unknown`, never a guess.
#[must_use]
pub fn observe_owner_session(pid: u32) -> OwnerSessionObservation {
    let observed = (|| {
        Result::<_, QueryFailure>::Ok((
            token::user_sid_of(pid)?,
            token::current_user_sid()?,
            identity::session_id(pid)?,
            token::current_session_id()?,
        ))
    })();
    let (target_sid, current_sid, target_session, current_session) = match observed {
        Ok(parts) => parts,
        Err(failure) => {
            return OwnerSessionObservation::Unknown {
                reason: failure.to_string(),
            }
        }
    };
    if target_sid.0.eq_ignore_ascii_case(&current_sid.0) && target_session == current_session {
        OwnerSessionObservation::CurrentUserSession
    } else {
        OwnerSessionObservation::Different {
            detail: format!(
                "target user {0} session {target_session}; current user {1} session {current_session}",
                target_sid.0, current_sid.0
            ),
        }
    }
}

/// Foreground observation right now: the foreground window's process is
/// compared to the target PID. The observation's age is zero by construction
/// (fresh query); staleness is decided in core against
/// `APPLY_INPUT_FRESHNESS_MS`. No foreground window (locked/pre-logon
/// session) or an unreadable window owner is `Unknown`.
#[must_use]
pub fn observe_focus(target_pid: u32) -> FocusObservation {
    // SAFETY: plain read-only desktop query; returns a possibly-null handle.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return FocusObservation::Unknown {
            reason: "no foreground window (no interactive desktop)".to_string(),
        };
    }
    let mut foreground_pid: u32 = 0;
    // SAFETY: valid live window handle and out-pointer.
    let _thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut foreground_pid)) };
    if foreground_pid == 0 {
        return FocusObservation::Unknown {
            reason: "GetWindowThreadProcessId returned no owner PID".to_string(),
        };
    }
    FocusObservation::Observed {
        target_foreground: foreground_pid == target_pid,
        observed_age_ms: 0,
    }
}

/// Current CPU priority class through a query-limited handle. Only `Normal`
/// is actionable downstream; the class value is reported verbatim otherwise.
#[must_use]
pub fn observe_priority_class(pid: u32) -> PriorityClassObservation {
    let handle = match open_query_handle(pid) {
        Ok(handle) => handle,
        Err(failure) => {
            return PriorityClassObservation::Unknown {
                reason: failure.to_string(),
            }
        }
    };
    // SAFETY: owned query-only handle; plain read.
    let class = unsafe { GetPriorityClass(handle.raw()) };
    // The windows-rs constants are newtypes, so their values are bound to
    // local consts to stay usable in match patterns.
    const NORMAL: u32 = NORMAL_PRIORITY_CLASS.0;
    const ABOVE_NORMAL: u32 = ABOVE_NORMAL_PRIORITY_CLASS.0;
    const BELOW_NORMAL: u32 = BELOW_NORMAL_PRIORITY_CLASS.0;
    const HIGH: u32 = HIGH_PRIORITY_CLASS.0;
    const IDLE: u32 = IDLE_PRIORITY_CLASS.0;
    const REALTIME: u32 = REALTIME_PRIORITY_CLASS.0;
    match class {
        0 => PriorityClassObservation::Unknown {
            reason: last_error_query_failure("GetPriorityClass"),
        },
        NORMAL => PriorityClassObservation::Normal,
        ABOVE_NORMAL => PriorityClassObservation::Other {
            detail: "ABOVE_NORMAL_PRIORITY_CLASS".to_string(),
        },
        BELOW_NORMAL => PriorityClassObservation::Other {
            detail: "BELOW_NORMAL_PRIORITY_CLASS".to_string(),
        },
        HIGH => PriorityClassObservation::Other {
            detail: "HIGH_PRIORITY_CLASS".to_string(),
        },
        IDLE => PriorityClassObservation::Other {
            detail: "IDLE_PRIORITY_CLASS".to_string(),
        },
        REALTIME => PriorityClassObservation::Other {
            detail: "REALTIME_PRIORITY_CLASS".to_string(),
        },
        other => PriorityClassObservation::Other {
            detail: format!("PRIORITY_CLASS({other:#x})"),
        },
    }
}

/// Live direct children of `target_pid` from one Toolhelp snapshot (R-10:
/// leaf workloads only). The process table is the tested evidence source;
/// a snapshot failure is `Unknown`, never a silent "no children".
#[must_use]
pub fn observe_children(target_pid: u32) -> ChildObservation {
    match processes::enumerate_pid_parents() {
        Ok(rows) => ChildObservation::Observed {
            live_children: rows
                .into_iter()
                .filter(|&(pid, parent)| parent == target_pid && pid != target_pid)
                .count() as u32,
        },
        Err(failure) => ChildObservation::Unknown {
            reason: failure.to_string(),
        },
    }
}

/// Capability probe for the actuation API: opens the target with
/// `PROCESS_SET_INFORMATION` (the right `SetPriorityClass` needs) and closes
/// the handle without writing anything. Denial is the tested "cannot change"
/// signal; exit and OS errors are `Unknown`.
#[must_use]
pub fn probe_priority_capability(pid: u32) -> PriorityCapability {
    // SAFETY: plain call with a numeric PID; no inherit; handle closed by RAII.
    match unsafe { OpenProcess(PROCESS_SET_INFORMATION, false, pid) } {
        Ok(handle) => {
            // SAFETY: valid owned handle; nothing is written through it.
            drop(unsafe { OwnedHandle::adopt(handle) });
            PriorityCapability::Supported
        }
        Err(error) => match open_process_failure(&error) {
            QueryFailure::AccessDenied => PriorityCapability::Unsupported,
            failure => PriorityCapability::Unknown {
                reason: failure.to_string(),
            },
        },
    }
}

/// Safety classification from the tested signal matrix (R-03). Today's tested
/// coverage is self-identification only: the target's PID or image name
/// matching RAMShaper's own process is `protected_ramshaper`. Everything else
/// is `Unknown` (deny) until RS-024 Stage B adds tested coverage — the image
/// name is compared only against RAMShaper's *own* well-known binary names,
/// never against a denylist of other software.
#[must_use]
pub fn classify_target(
    target_pid: u32,
    target_image_name: &str,
    self_pid: u32,
    self_image_names: &[&str],
) -> SafetyClassification {
    if target_pid == self_pid
        || self_image_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(target_image_name))
    {
        return SafetyClassification::Protected {
            category: ProtectedCategory::RamShaper,
        };
    }
    SafetyClassification::Unknown {
        reason: "no tested classification signal covers this target (RS-024 Stage B pending)"
            .to_string(),
    }
}

/// Open a query-only handle (minimal rights) for the priority-class read.
fn open_query_handle(pid: u32) -> Result<OwnedHandle, QueryFailure> {
    // SAFETY: plain call with a numeric PID; inherit=false; null rejected.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|e| open_process_failure(&e))?;
    // SAFETY: OpenProcess returned a valid owned handle.
    Ok(unsafe { OwnedHandle::adopt(handle) })
}

/// Preserve the immediate Win32 error for a zero-class read (the crate's
/// `to_query_failure` needs the `windows::Error` type; this keeps the same
/// code from `GetLastError` directly).
fn last_error_query_failure(function: &'static str) -> String {
    let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32;
    QueryFailure::OsError { code, function }.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};

    /// A short-lived quiet child: `ping -n <seconds> 127.0.0.1`.
    fn spawn_quiet_child(seconds: u32) -> Child {
        Command::new("ping")
            .args(["-n", &seconds.to_string(), "127.0.0.1"])
            .spawn()
            .expect("spawn ping child")
    }

    /// A PID that no longer resolves (the documented post-exit state,
    /// `ERROR_INVALID_PARAMETER`): an unused PID from the top of the address
    /// space. Windows reuses exited PIDs within seconds, so a real exited
    /// child is not a deterministic fixture — an unresolvable PID is.
    fn unresolvable_pid() -> u32 {
        let used: std::collections::HashSet<u32> = processes::enumerate_pid_parents()
            .expect("process-table snapshot")
            .into_iter()
            .map(|(pid, _parent)| pid)
            .collect();
        for candidate in (0xFF_FF_FCu32 - 4096..=0xFF_FF_FC)
            .rev()
            .filter(|c| c % 4 == 0)
        {
            if !used.contains(&candidate) && identity::resolve_identity(candidate).is_err() {
                return candidate;
            }
        }
        panic!("no unresolvable PID found in the top range");
    }

    #[test]
    fn owner_session_of_current_process_is_current_user_session() {
        let pid = std::process::id();
        assert_eq!(
            observe_owner_session(pid),
            OwnerSessionObservation::CurrentUserSession
        );
    }

    #[test]
    fn owner_session_of_unresolvable_pid_is_unknown() {
        let pid = unresolvable_pid();
        match observe_owner_session(pid) {
            OwnerSessionObservation::Unknown { reason } => {
                println!("unresolvable-PID owner observation unknown: {reason}");
            }
            other => panic!("expected Unknown for an unresolvable PID, got {other:?}"),
        }
    }

    #[test]
    fn owner_session_of_system_process_is_unknown_or_different_never_current() {
        let Some(pid) = processes::pid_of_image("services.exe") else {
            println!("services.exe not found; skipping");
            return;
        };
        match observe_owner_session(pid) {
            OwnerSessionObservation::Unknown { reason } => {
                println!("standard user cannot read services.exe token: {reason}");
            }
            OwnerSessionObservation::Different { detail } => {
                println!("elevated environment observed services.exe context: {detail}");
            }
            OwnerSessionObservation::CurrentUserSession => {
                panic!("services.exe must never observe as the current user's session");
            }
        }
    }

    #[test]
    fn focus_of_windowless_test_process_is_never_foreground() {
        match observe_focus(std::process::id()) {
            FocusObservation::Observed {
                target_foreground,
                observed_age_ms,
            } => {
                assert!(!target_foreground, "the test process owns no windows");
                assert_eq!(observed_age_ms, 0, "adapter observations are fresh");
            }
            FocusObservation::Unknown { reason } => {
                println!("no interactive foreground in this session: {reason}");
            }
        }
    }

    #[test]
    fn priority_class_of_fresh_child_is_normal() {
        let mut child = spawn_quiet_child(3);
        let observed = observe_priority_class(child.id());
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(
            observed,
            PriorityClassObservation::Normal,
            "a fresh child of a normally launched test process runs at Normal"
        );
    }

    #[test]
    fn priority_class_of_unresolvable_pid_is_unknown() {
        let pid = unresolvable_pid();
        match observe_priority_class(pid) {
            PriorityClassObservation::Unknown { reason } => {
                println!("unresolvable-PID class read unknown: {reason}");
            }
            other => panic!("expected Unknown for an unresolvable PID, got {other:?}"),
        }
    }

    #[test]
    fn children_of_current_process_include_the_spawned_child() {
        let mut child = spawn_quiet_child(5);
        let observed = observe_children(std::process::id());
        let _ = child.kill();
        let _ = child.wait();
        match observed {
            ChildObservation::Observed { live_children } => {
                // Other tests in this suite may run concurrently in the same
                // process, so the exact count is not deterministic.
                assert!(live_children >= 1, "the spawned child must be counted");
            }
            ChildObservation::Unknown { reason } => {
                panic!("process-table snapshot failed on a live desktop: {reason}");
            }
        }
    }

    #[test]
    fn children_of_unresolvable_pid_is_unknown() {
        let pid = unresolvable_pid();
        match observe_children(pid) {
            // A count of zero is also honest (the PID is gone from the
            // table); it must never be a *guessed* zero from a failed scan.
            ChildObservation::Observed { live_children } => {
                assert_eq!(live_children, 0);
            }
            ChildObservation::Unknown { reason } => {
                println!("unresolvable-PID children observation unknown: {reason}");
            }
        }
    }

    #[test]
    fn capability_of_own_process_is_supported() {
        assert_eq!(
            probe_priority_capability(std::process::id()),
            PriorityCapability::Supported
        );
    }

    #[test]
    fn capability_is_unsupported_for_system_service_process_on_standard_user() {
        let Some(pid) = processes::pid_of_image("services.exe") else {
            println!("services.exe not found; skipping");
            return;
        };
        match probe_priority_capability(pid) {
            PriorityCapability::Unsupported => {}
            PriorityCapability::Supported => {
                println!("elevated environment opened services.exe; strict assertion skipped");
            }
            PriorityCapability::Unknown { reason } => {
                panic!("services.exe is permanent; expected a readable probe: {reason}");
            }
        }
    }

    #[test]
    fn capability_of_unresolvable_pid_is_unknown() {
        let pid = unresolvable_pid();
        match probe_priority_capability(pid) {
            PriorityCapability::Unknown { reason } => {
                println!("unresolvable-PID capability probe unknown: {reason}");
            }
            other => panic!("expected Unknown for an unresolvable PID, got {other:?}"),
        }
    }

    #[test]
    fn classification_covers_self_identification_only() {
        let self_pid = std::process::id();
        let self_names = [
            "ramshaper-cli.exe",
            "ramshaper-agent.exe",
            "ramshaper-ui.exe",
        ];
        for (pid, image) in [
            (self_pid, "anything.exe"),
            (999_999, "ramshaper-cli.exe"),
            (999_999, "RAMSHAPER-AGENT.EXE"),
        ] {
            assert_eq!(
                classify_target(pid, image, self_pid, &self_names),
                SafetyClassification::Protected {
                    category: ProtectedCategory::RamShaper
                },
                "{image} at pid {pid} must classify as RAMShaper"
            );
        }
        match classify_target(999_999, "notepad.exe", self_pid, &self_names) {
            SafetyClassification::Unknown { .. } => {}
            other => panic!("untested third-party targets must stay Unknown, got {other:?}"),
        }
    }

    #[test]
    fn last_error_helper_preserves_code() {
        // Sanity: the helper formats a typed OsError with the captured code.
        let reason = last_error_query_failure("Test");
        assert!(reason.contains("OS error"), "got: {reason}");
    }
}
