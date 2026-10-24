//! Deterministic fake adapters and fixtures (RS-002).
//!
//! Used by core/CLI tests to cover success, denial, unsupported and OS-error
//! paths without touching the real OS. All fixture data is synthetic.

use std::cell::RefCell;
use std::collections::VecDeque;

use ramshaper_core::envelope::LowMemorySignal;
use ramshaper_core::snapshot::{GlobalMemoryRaw, OsInfo, QueryFailure, SystemSample, ToolInfo};

use crate::capabilities::{CapabilityInfo, CapabilitySet, CAPABILITY_LOW_MEMORY_NOTIFICATION};
use crate::telemetry::{MemoryConditionSource, ReadOnlySystemMonitor};

/// Synthetic AT-02 fixture: 16 GiB physical / 4 GiB available / 12 GiB commit
/// used / 32 GiB commit limit, 4 KiB pages. Exact byte values.
#[must_use]
pub fn raw_memory_fixture_16gib_4k() -> GlobalMemoryRaw {
    GlobalMemoryRaw {
        physical_total_pages: 4_194_304,
        physical_available_pages: 1_048_576,
        commit_total_pages: 3_145_728,
        commit_limit_pages: 8_388_608,
        page_size_bytes: 4096,
    }
}

/// Synthetic OS metadata fixture (marked synthetic in the product name).
#[must_use]
pub fn os_fixture() -> OsInfo {
    OsInfo {
        product_name: Some("Windows 11 Pro (synthetic fixture)".to_string()),
        version: Some("10.0.26200.9711".to_string()),
        build: Some(26200),
        ubr: Some(9711),
        display_version: Some("25H2".to_string()),
        arch: Some("x86_64".to_string()),
        source: "synthetic",
    }
}

/// Tool-info fixture for tests.
#[must_use]
pub fn tool_fixture() -> ToolInfo {
    ToolInfo {
        name: "ramshaper-cli".to_string(),
        version: "0.1.0 (test fixture)".to_string(),
    }
}

fn advance<T: Clone>(script: &RefCell<VecDeque<T>>) -> Option<T> {
    let mut queue = script.borrow_mut();
    match queue.len() {
        0 => None,
        1 => queue.front().cloned(),
        _ => queue.pop_front(),
    }
}

/// Scripted read-only monitor. Plays the given results in order and repeats
/// the last one once the script is exhausted.
#[derive(Debug, Clone)]
pub struct FakeSystemMonitor {
    memory: RefCell<VecDeque<Result<GlobalMemoryRaw, QueryFailure>>>,
    low_memory: RefCell<VecDeque<Result<LowMemorySignal, QueryFailure>>>,
    os: OsInfo,
}

impl FakeSystemMonitor {
    /// Script both sources.
    #[must_use]
    pub fn new(
        memory: Vec<Result<GlobalMemoryRaw, QueryFailure>>,
        low_memory: Vec<Result<LowMemorySignal, QueryFailure>>,
    ) -> Self {
        Self {
            memory: RefCell::new(VecDeque::from(memory)),
            low_memory: RefCell::new(VecDeque::from(low_memory)),
            os: os_fixture(),
        }
    }

    /// Always-healthy monitor.
    #[must_use]
    pub fn healthy() -> Self {
        Self::new(
            vec![Ok(raw_memory_fixture_16gib_4k())],
            vec![Ok(LowMemorySignal::NotSignaled)],
        )
    }

    /// Memory queries always denied.
    #[must_use]
    pub fn memory_denied() -> Self {
        Self::new(
            vec![Err(QueryFailure::AccessDenied)],
            vec![Ok(LowMemorySignal::NotSignaled)],
        )
    }

    /// Memory queries always fail with a Win32 error.
    #[must_use]
    pub fn memory_os_error(code: u32) -> Self {
        Self::new(
            vec![Err(QueryFailure::OsError {
                code,
                function: "GetPerformanceInfo",
            })],
            vec![Ok(LowMemorySignal::NotSignaled)],
        )
    }

    /// Low-memory condition always unavailable.
    #[must_use]
    pub fn low_memory_unavailable(reason: &str) -> Self {
        Self::new(
            vec![Ok(raw_memory_fixture_16gib_4k())],
            vec![Err(QueryFailure::Unsupported(reason.to_string()))],
        )
    }

    fn next_memory(&self) -> Result<GlobalMemoryRaw, QueryFailure> {
        advance(&self.memory)
            .unwrap_or_else(|| Err(QueryFailure::Unsupported("empty fake script".to_string())))
    }

    fn next_low_memory(&self) -> Result<LowMemorySignal, QueryFailure> {
        advance(&self.low_memory)
            .unwrap_or_else(|| Err(QueryFailure::Unsupported("empty fake script".to_string())))
    }
}

impl ReadOnlySystemMonitor for FakeSystemMonitor {
    fn capabilities(&self) -> CapabilitySet {
        let mut set = CapabilitySet::new();
        set.push(CapabilityInfo::supported(
            crate::capabilities::CAPABILITY_SYSTEM_MEMORY_COUNTERS,
        ));
        set.push(CapabilityInfo::supported(
            crate::capabilities::CAPABILITY_PROCESS_IDENTITY,
        ));
        set.push(CapabilityInfo::supported(
            CAPABILITY_LOW_MEMORY_NOTIFICATION,
        ));
        set
    }

    fn collect_system(&self) -> SystemSample {
        SystemSample {
            memory: self.next_memory(),
            low_memory: self.next_low_memory(),
        }
    }

    fn os_info(&self) -> OsInfo {
        self.os.clone()
    }
}

impl MemoryConditionSource for FakeSystemMonitor {
    fn query_low_memory(&self) -> Result<LowMemorySignal, QueryFailure> {
        self.next_low_memory()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_fake_yields_exact_values() {
        let fake = FakeSystemMonitor::healthy();
        let sample = fake.collect_system();
        let raw = sample.memory.expect("ok");
        assert_eq!(raw, raw_memory_fixture_16gib_4k());
        assert_eq!(sample.low_memory, Ok(LowMemorySignal::NotSignaled));
        // Repeats last scripted value.
        let again = fake.collect_system();
        assert_eq!(again.memory, Ok(raw));
    }

    #[test]
    fn denied_fake_reports_denial_not_zero() {
        let fake = FakeSystemMonitor::memory_denied();
        assert_eq!(
            fake.collect_system().memory,
            Err(QueryFailure::AccessDenied)
        );
    }

    #[test]
    fn os_error_fake_preserves_code() {
        let fake = FakeSystemMonitor::memory_os_error(1455);
        assert_eq!(
            fake.collect_system().memory,
            Err(QueryFailure::OsError {
                code: 1455,
                function: "GetPerformanceInfo"
            })
        );
    }

    #[test]
    fn low_memory_unavailable_is_partial() {
        let fake = FakeSystemMonitor::low_memory_unavailable("not created");
        let sample = fake.collect_system();
        assert!(sample.memory.is_ok());
        assert_eq!(
            sample.low_memory,
            Err(QueryFailure::Unsupported("not created".to_string()))
        );
    }

    #[test]
    fn scripted_sequence_plays_in_order_then_repeats() {
        let fake = FakeSystemMonitor::new(
            vec![
                Ok(raw_memory_fixture_16gib_4k()),
                Err(QueryFailure::AccessDenied),
            ],
            vec![
                Ok(LowMemorySignal::Signaled),
                Ok(LowMemorySignal::NotSignaled),
            ],
        );
        let first = fake.collect_system();
        let second = fake.collect_system();
        let third = fake.collect_system();
        assert!(first.memory.is_ok() && first.low_memory == Ok(LowMemorySignal::Signaled));
        assert_eq!(second.memory, Err(QueryFailure::AccessDenied));
        assert_eq!(second.low_memory, Ok(LowMemorySignal::NotSignaled));
        // Script exhausted: repeats last entries.
        assert_eq!(third.memory, Err(QueryFailure::AccessDenied));
        assert_eq!(third.low_memory, Ok(LowMemorySignal::NotSignaled));
    }

    #[test]
    fn capabilities_report_supported() {
        let fake = FakeSystemMonitor::healthy();
        let set = fake.capabilities();
        assert!(set.is_supported(crate::capabilities::CAPABILITY_SYSTEM_MEMORY_COUNTERS));
        assert!(set.is_supported(CAPABILITY_LOW_MEMORY_NOTIFICATION));
    }

    #[test]
    fn empty_script_reports_unsupported_not_panic() {
        let fake = FakeSystemMonitor::new(vec![], vec![]);
        let sample = fake.collect_system();
        assert!(matches!(sample.memory, Err(QueryFailure::Unsupported(_))));
    }
}

/// Deterministic process-scan fixture helpers (churn, reuse, denial).
pub mod process_fixtures {
    use ramshaper_core::identity::ProcessIdentity;
    use ramshaper_core::process::ProcessCountersRaw;
    use ramshaper_core::snapshot::QueryFailure;

    /// A healthy synthetic process with the given identity and counters.
    #[must_use]
    pub fn healthy(
        identity: ProcessIdentity,
        cpu_100ns: u64,
        io_bytes: u64,
        working_set: u64,
    ) -> ProcessCountersRaw {
        ProcessCountersRaw {
            identity,
            name: format!("proc{}.exe", identity.pid),
            parent_pid: 4,
            working_set_bytes: Ok(working_set),
            private_commit_bytes: Ok(working_set * 2),
            cpu_kernel_100ns: Ok(cpu_100ns / 2),
            cpu_user_100ns: Ok(cpu_100ns - cpu_100ns / 2),
            io_read_bytes: Ok(io_bytes),
            io_write_bytes: Ok(io_bytes / 2),
        }
    }

    /// A process whose queries are denied (protected-process shape).
    #[must_use]
    pub fn denied(identity: ProcessIdentity) -> ProcessCountersRaw {
        ProcessCountersRaw {
            identity,
            name: format!("protected{}.exe", identity.pid),
            parent_pid: 4,
            working_set_bytes: Err(QueryFailure::AccessDenied),
            private_commit_bytes: Err(QueryFailure::AccessDenied),
            cpu_kernel_100ns: Err(QueryFailure::AccessDenied),
            cpu_user_100ns: Err(QueryFailure::AccessDenied),
            io_read_bytes: Err(QueryFailure::AccessDenied),
            io_write_bytes: Err(QueryFailure::AccessDenied),
        }
    }
}

/// Scripted process monitor for churn/reuse/denial tests.
#[derive(Debug, Clone, Default)]
pub struct FakeProcessMonitor {
    scans: RefCell<VecDeque<ramshaper_core::process::ProcessScan>>,
    cpu_count: u32,
}

impl FakeProcessMonitor {
    /// Play the given scans in order; the last repeats.
    #[must_use]
    pub fn new(scans: Vec<ramshaper_core::process::ProcessScan>, cpu_count: u32) -> Self {
        Self {
            scans: RefCell::new(VecDeque::from(scans)),
            cpu_count,
        }
    }
}

impl crate::telemetry::ReadOnlyProcessMonitor for FakeProcessMonitor {
    fn collect_processes(&self) -> ramshaper_core::process::ProcessScan {
        let mut queue = self.scans.borrow_mut();
        match queue.len() {
            0 => ramshaper_core::process::ProcessScan {
                processes: Vec::new(),
                truncated: false,
                scan_failure: None,
                logical_cpu_count: self.cpu_count,
            },
            1 => queue.front().cloned().expect("len checked"),
            _ => queue.pop_front().expect("len checked"),
        }
    }
}
