//! Read-only Windows monitor implementing the platform contracts.

use ramshaper_core::envelope::LowMemorySignal;
use ramshaper_core::identity::ProcessIdentity;
use ramshaper_core::process::ProcessScan;
use ramshaper_core::snapshot::{OsInfo, QueryFailure, SystemSample};
use ramshaper_platform::capabilities::{
    CapabilityInfo, CapabilitySet, CAPABILITY_LOW_MEMORY_NOTIFICATION, CAPABILITY_PROCESS_IDENTITY,
    CAPABILITY_SYSTEM_MEMORY_COUNTERS,
};
use ramshaper_platform::telemetry::{
    MemoryConditionSource, ReadOnlyProcessMonitor, ReadOnlySystemMonitor,
};

use crate::identity;
use crate::memnotify::MemoryResourceNotification;
use crate::osinfo;
use crate::perfinfo;

/// Windows adapter. Owns the low-memory notification object for its lifetime
/// (RAII; closed on drop). Constructing it performs no process mutation.
pub struct WindowsSystemMonitor {
    low_memory: Option<MemoryResourceNotification>,
    low_memory_unavailable_reason: Option<String>,
    os: OsInfo,
}

impl WindowsSystemMonitor {
    /// Create the adapter. If the low-memory notification object cannot be
    /// created (resource exhaustion), the capability is reported unsupported
    /// and low-memory metrics degrade to explicit `unsupported` envelopes.
    pub fn new() -> Self {
        let (low_memory, reason) = match MemoryResourceNotification::create_low() {
            Ok(notification) => (Some(notification), None),
            Err(error) => (
                None,
                Some(format!(
                    "CreateMemoryResourceNotification failed (Win32 {})",
                    crate::error::win32_code(&error)
                )),
            ),
        };
        Self {
            low_memory,
            low_memory_unavailable_reason: reason,
            os: osinfo::collect_os_info(),
        }
    }
}

impl Default for WindowsSystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadOnlySystemMonitor for WindowsSystemMonitor {
    fn capabilities(&self) -> CapabilitySet {
        let mut set = CapabilitySet::new();
        set.push(CapabilityInfo::supported(CAPABILITY_SYSTEM_MEMORY_COUNTERS));
        match (&self.low_memory, &self.low_memory_unavailable_reason) {
            (Some(_), _) => set.push(CapabilityInfo::supported(
                CAPABILITY_LOW_MEMORY_NOTIFICATION,
            )),
            (None, Some(reason)) => {
                set.push(CapabilityInfo::unsupported(
                    CAPABILITY_LOW_MEMORY_NOTIFICATION,
                    reason.clone(),
                ));
            }
            (None, None) => set.push(CapabilityInfo::unsupported(
                CAPABILITY_LOW_MEMORY_NOTIFICATION,
                "not initialized",
            )),
        }
        set.push(CapabilityInfo::supported(CAPABILITY_PROCESS_IDENTITY));
        set
    }

    fn collect_system(&self) -> SystemSample {
        let memory = perfinfo::query_global_memory()
            .map_err(|e| crate::error::to_query_failure("GetPerformanceInfo", &e));
        let low_memory = self.query_low_memory();
        SystemSample { memory, low_memory }
    }

    fn os_info(&self) -> OsInfo {
        self.os.clone()
    }
}

impl MemoryConditionSource for WindowsSystemMonitor {
    fn query_low_memory(&self) -> Result<LowMemorySignal, QueryFailure> {
        match &self.low_memory {
            Some(notification) => notification
                .query()
                .map_err(|e| crate::error::to_query_failure("QueryMemoryResourceNotification", &e)),
            None => Err(QueryFailure::Unsupported(
                self.low_memory_unavailable_reason
                    .clone()
                    .unwrap_or_else(|| "low-memory notification object unavailable".to_string()),
            )),
        }
    }
}

impl ReadOnlyProcessMonitor for WindowsSystemMonitor {
    fn collect_processes(&self) -> ProcessScan {
        crate::processes::scan_processes()
    }
}

/// Pseudonymous agent-instance ID for this process instance:
/// `cli-<pid>-<exact creation FILETIME>`. Because the creation FILETIME is
/// unique per process instance, a later process with a reused PID gets a
/// different instance ID. It is an identifier, not a clock.
#[must_use]
pub fn agent_instance_id() -> String {
    identity::current_process_identity().map_or_else(
        |_| "cli-unknown".to_string(),
        |ProcessIdentity {
             pid,
             creation_filetime_100ns,
         }| { format!("cli-{pid}-{creation_filetime_100ns}") },
    )
}
