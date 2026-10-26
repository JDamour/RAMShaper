//! The real CPU-priority actuator (RS-016, behind the disabled-actuation
//! gate).
//!
//! Implements `ramshaper-core::coordinator::PriorityActuator` against
//! `SetPriorityClass`. Safety properties (contract 14 §1/§6):
//!
//! - **one handle per write**, opened with `PROCESS_SET_INFORMATION` plus
//!   the `PROCESS_QUERY_LIMITED_INFORMATION` right the verification reads
//!   need — minimal rights, no inheritance;
//! - the exact creation identity is validated **through the write handle**
//!   before any effect; a PID that cannot be validated (exited, reused,
//!   access denied) is refused with `IdentityMismatch` and nothing is
//!   written — the eligibility capability probe (RS-015/R-03) is what
//!   pre-screens "supported" separately;
//! - every write is **read back through the same handle**; the caller
//!   (coordinator) compares the read-back to the desired value;
//! - a failed `SetPriorityClass` is never assumed to have had no effect:
//!   the value is read back, and a read-back equal to the desired value is
//!   the honest `UnknownOutcome` (the write may have landed).

use ramshaper_core::coordinator::{ActuatorError, PriorityActuator};
use ramshaper_core::identity::ProcessIdentity;
use windows::Win32::System::Threading::{
    GetPriorityClass, OpenProcess, SetPriorityClass, PROCESS_CREATION_FLAGS,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
};

use crate::error::win32_code;
use crate::handle::OwnedHandle;
use crate::identity;

/// The Windows actuator. Stateless: every write opens its own validated
/// handle, so nothing survives between operations.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsPriorityActuator;

impl PriorityActuator for WindowsPriorityActuator {
    fn read(&self, expected: &ProcessIdentity) -> Result<u32, ActuatorError> {
        // SAFETY: plain call with a numeric PID; query-only rights.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, expected.pid) }
            .map_err(|_| ActuatorError::IdentityMismatch)
            .map(|handle| {
                // SAFETY: valid owned handle from OpenProcess.
                unsafe { OwnedHandle::adopt(handle) }
            })?;
        // Validate the exact creation identity through the handle: a
        // reused PID must never lend its value to the consented action.
        match identity::creation_filetime(&handle) {
            Ok(creation) if creation == expected.creation_filetime_100ns => {}
            _ => return Err(ActuatorError::IdentityMismatch),
        }
        read_class(&handle).ok_or(ActuatorError::OsFailure {
            code: std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32,
            function: "GetPriorityClass",
        })
    }

    fn write_verified(
        &mut self,
        expected: &ProcessIdentity,
        desired: u32,
    ) -> Result<u32, ActuatorError> {
        // SAFETY: plain call with a numeric PID; no inherit; null rejected.
        let handle = unsafe {
            OpenProcess(
                PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                expected.pid,
            )
        }
        .map_err(|_| ActuatorError::IdentityMismatch)
        .map(|handle| {
            // SAFETY: OpenProcess returned a valid owned handle.
            unsafe { OwnedHandle::adopt(handle) }
        })?;

        // Validate the exact creation identity through the write handle:
        // a reused PID must never inherit the consented write.
        match identity::creation_filetime(&handle) {
            Ok(creation) if creation == expected.creation_filetime_100ns => {}
            _ => return Err(ActuatorError::IdentityMismatch),
        }

        // SAFETY: owned validated handle; the requested value is the
        // policy's Below Normal constant in the 0.2 build.
        match unsafe { SetPriorityClass(handle.raw(), PROCESS_CREATION_FLAGS(desired)) } {
            Ok(()) => match read_class(&handle) {
                Some(read_back) => Ok(read_back),
                None => Err(ActuatorError::UnknownOutcome),
            },
            Err(error) => {
                // Never assume no effect: read back and classify honestly.
                match read_class(&handle) {
                    Some(value) if value == desired => Err(ActuatorError::UnknownOutcome),
                    Some(_) => Err(ActuatorError::OsFailure {
                        code: win32_code(&error),
                        function: "SetPriorityClass",
                    }),
                    None => Err(ActuatorError::UnknownOutcome),
                }
            }
        }
    }
}

/// Read the priority class through a handle; `None` on failure (the caller
/// decides whether that means "unknown outcome").
pub(crate) fn read_class(handle: &OwnedHandle) -> Option<u32> {
    // SAFETY: owned handle; plain read.
    let class = unsafe { GetPriorityClass(handle.raw()) };
    (class != 0).then_some(class)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eligibility;
    use crate::identity::resolve_identity;
    use crate::processes;
    use ramshaper_core::eligibility::PriorityClassObservation;
    use std::process::{Child, Command};
    use windows::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS};

    /// A quiet, alive child: `ping -n <seconds> 127.0.0.1`.
    fn spawn_quiet_child(seconds: u32) -> Child {
        Command::new("ping")
            .args(["-n", &seconds.to_string(), "127.0.0.1"])
            .spawn()
            .expect("spawn ping child")
    }

    /// An unused PID from the top of the address space (the documented
    /// post-exit `Exited` state; Windows reuses exited PIDs within seconds,
    /// so a real exited child is not a deterministic fixture).
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
            if !used.contains(&candidate) && resolve_identity(candidate).is_err() {
                return candidate;
            }
        }
        panic!("no unresolvable PID found in the top range");
    }

    #[test]
    fn write_on_spawned_child_normal_to_below_normal_and_restored() {
        let mut child = spawn_quiet_child(6);
        let pid = child.id();
        let identity = resolve_identity(pid).expect("fresh child identity");
        let mut actuator = WindowsPriorityActuator;

        let read_back = actuator
            .write_verified(&identity, BELOW_NORMAL_PRIORITY_CLASS.0)
            .expect("write Below Normal");
        assert_eq!(read_back, BELOW_NORMAL_PRIORITY_CLASS.0);
        assert_eq!(
            eligibility::observe_priority_class(pid),
            PriorityClassObservation::Other {
                detail: "BELOW_NORMAL_PRIORITY_CLASS".to_string(),
            },
            "independent adapter read must confirm the write"
        );

        // Restore through the same verified path (the coordinator's
        // restoration uses the identical primitive).
        let restored = actuator
            .write_verified(&identity, NORMAL_PRIORITY_CLASS.0)
            .expect("restore Normal");
        assert_eq!(restored, NORMAL_PRIORITY_CLASS.0);
        assert_eq!(
            eligibility::observe_priority_class(pid),
            PriorityClassObservation::Normal,
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn mismatched_creation_identity_is_refused_without_effect() {
        let mut child = spawn_quiet_child(4);
        let pid = child.id();
        let identity = resolve_identity(pid).expect("fresh child identity");
        let reused = ProcessIdentity::new(pid, identity.creation_filetime_100ns + 1);

        let mut actuator = WindowsPriorityActuator;
        let result = actuator.write_verified(&reused, BELOW_NORMAL_PRIORITY_CLASS.0);
        assert!(
            matches!(result, Err(ActuatorError::IdentityMismatch)),
            "a wrong creation marker must refuse the write: {result:?}"
        );
        assert_eq!(
            eligibility::observe_priority_class(pid),
            PriorityClassObservation::Normal,
            "the class must be untouched"
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn unresolvable_pid_is_refused_without_effect() {
        let pid = unresolvable_pid();
        let identity = ProcessIdentity::new(pid, 12_345);
        let mut actuator = WindowsPriorityActuator;
        let result = actuator.write_verified(&identity, BELOW_NORMAL_PRIORITY_CLASS.0);
        assert!(matches!(result, Err(ActuatorError::IdentityMismatch)));
    }

    #[test]
    fn read_returns_the_class_through_a_validated_handle() {
        let mut child = spawn_quiet_child(4);
        let pid = child.id();
        let identity = resolve_identity(pid).expect("fresh child identity");
        let actuator = WindowsPriorityActuator;

        assert_eq!(actuator.read(&identity), Ok(NORMAL_PRIORITY_CLASS.0));
        let reused = ProcessIdentity::new(pid, identity.creation_filetime_100ns + 1);
        assert_eq!(actuator.read(&reused), Err(ActuatorError::IdentityMismatch));

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn read_on_an_unresolvable_pid_is_identity_mismatch() {
        let identity = ProcessIdentity::new(unresolvable_pid(), 12_345);
        let actuator = WindowsPriorityActuator;
        assert_eq!(
            actuator.read(&identity),
            Err(ActuatorError::IdentityMismatch)
        );
    }
}
