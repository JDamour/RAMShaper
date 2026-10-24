//! Read-only telemetry adapter contracts.
//!
//! The first CLI calls these directly (architecture doc 03 §1); the later
//! agent exposes them over IPC. Every method is read-only: collecting data or
//! reporting capabilities, never changing process or system state.

use crate::capabilities::CapabilitySet;
use ramshaper_core::envelope::LowMemorySignal;
use ramshaper_core::process::ProcessScan;
use ramshaper_core::snapshot::{OsInfo, QueryFailure, SystemSample};

/// Read-only system telemetry source.
pub trait ReadOnlySystemMonitor {
    /// Capabilities supported by this adapter instance on this system.
    fn capabilities(&self) -> CapabilitySet;

    /// Collect one global system sample. Partial failure is expressed inside
    /// the sample (per-source `Result`s), not by panicking or returning zero.
    fn collect_system(&self) -> SystemSample;

    /// OS/build metadata (may contain `None` fields when sources fail).
    fn os_info(&self) -> OsInfo;
}

/// Narrow read-only contract for the low-memory resource condition alone.
pub trait MemoryConditionSource {
    /// Query the current condition state without collecting other metrics.
    fn query_low_memory(&self) -> Result<LowMemorySignal, QueryFailure>;
}

/// Read-only process telemetry source (RS-007). Enumerates the process table
/// and queries per-process counters; never opens write-capable handles.
pub trait ReadOnlyProcessMonitor {
    /// One scan pass over the process table with raw per-process counters.
    /// Per-process failures (denied, exited) live inside the entries; a
    /// whole-scan failure is reported in `ProcessScan::scan_failure`.
    fn collect_processes(&self) -> ProcessScan;
}
