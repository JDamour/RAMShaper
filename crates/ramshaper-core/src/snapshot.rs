//! System snapshot composition (contract 14 §2).
//!
//! `build_snapshot` is a pure function: raw page counts from a successful
//! `GetPerformanceInfo` snapshot (or typed failures), a low-memory reading, OS
//! metadata and clock positions become the versioned snapshot object. All
//! page-to-byte conversion is checked; impossible combinations (available >
//! total, used > limit) become explicit error envelopes rather than wrapped or
//! invented values.

use serde::Serialize;

use crate::envelope::{LowMemorySignal, MetricEnvelope};
use crate::status::MetricStatus;
use crate::units::{byte_difference, pages_to_bytes, PageConversionError};

/// Stable metric names (JSON keys).
pub mod metric_names {
    /// Physical memory total, bytes.
    pub const PHYSICAL_TOTAL: &str = "physical_total_bytes";
    /// Physical memory available, bytes.
    pub const PHYSICAL_AVAILABLE: &str = "physical_available_bytes";
    /// Physical memory in use (derived: total − available), bytes.
    pub const PHYSICAL_IN_USE: &str = "physical_in_use_bytes";
    /// System commit charge used, bytes.
    pub const COMMIT_USED: &str = "commit_used_bytes";
    /// System commit limit, bytes.
    pub const COMMIT_LIMIT: &str = "commit_limit_bytes";
    /// Commit headroom (derived: limit − used), bytes.
    pub const COMMIT_HEADROOM: &str = "commit_headroom_bytes";
    /// Low-memory resource notification state.
    pub const LOW_MEMORY: &str = "low_memory";
}

/// Documented source APIs.
pub mod sources {
    /// Global memory/commit page counters.
    pub const PERFORMANCE_INFO: &str = "GetPerformanceInfo";
    /// Low-memory resource condition query.
    pub const MEMORY_RESOURCE_NOTIFICATION: &str = "QueryMemoryResourceNotification";
}

/// Snapshot schema version (contract 14 §8: one versioned object).
pub const SNAPSHOT_SCHEMA: &str = "ramshaper.snapshot.v1";

/// Typed query failure. Access denial, unsupported capability and process exit
/// are expected outcomes, not exceptional ones.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueryFailure {
    /// The OS denied access (for example a protected process).
    #[error("access denied")]
    AccessDenied,
    /// The capability is unavailable on this system or was disabled.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The target process exited or the PID no longer resolves.
    #[error("target process exited or PID not found (Win32 code {code})")]
    Exited {
        /// Win32 error code observed while resolving the PID.
        code: u32,
    },
    /// Any other OS failure, with the preserved native error code.
    #[error("OS error {code} from {function}")]
    OsError {
        /// Preserved native (Win32) error code.
        code: u32,
        /// Name of the failing API call, for audits.
        function: &'static str,
    },
}

impl QueryFailure {
    /// Map a failure to the metric status required by contract 14 §2.
    #[must_use]
    pub fn to_status(&self) -> MetricStatus {
        match self {
            QueryFailure::AccessDenied => MetricStatus::AccessDenied,
            QueryFailure::Unsupported(_) => MetricStatus::Unsupported,
            QueryFailure::Exited { .. } => MetricStatus::Exited,
            QueryFailure::OsError { .. } => MetricStatus::Error,
        }
    }

    /// Machine-readable error code for the envelope.
    #[must_use]
    pub fn error_code(&self) -> String {
        match self {
            QueryFailure::AccessDenied => "access_denied".to_string(),
            QueryFailure::Unsupported(_) => "unsupported".to_string(),
            QueryFailure::Exited { code } => format!("exited_{code}"),
            QueryFailure::OsError { code, .. } => format!("win32_{code}"),
        }
    }
}

/// Raw global memory counters from one successful `GetPerformanceInfo` call.
/// Page counts and page size come from the same snapshot, so derived values
/// describe one consistent moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalMemoryRaw {
    /// Total physical pages.
    pub physical_total_pages: u64,
    /// Available physical pages.
    pub physical_available_pages: u64,
    /// Committed pages (system commit charge).
    pub commit_total_pages: u64,
    /// Commit limit pages (physical + page-file backing).
    pub commit_limit_pages: u64,
    /// Page size in bytes as reported by the same API call.
    pub page_size_bytes: u64,
}

/// One collection attempt: raw success values or typed failures, per source.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemSample {
    /// Global memory counters, or the typed failure of the query.
    pub memory: Result<GlobalMemoryRaw, QueryFailure>,
    /// Low-memory resource condition, or the typed failure of the query.
    pub low_memory: Result<LowMemorySignal, QueryFailure>,
}

/// OS/build metadata. Optional fields stay explicit `null` when a source is
/// unavailable; nothing is invented.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OsInfo {
    /// Product name from the registry (e.g. "Windows 11 Pro"), if readable.
    pub product_name: Option<String>,
    /// Version string (e.g. "10.0.26200.9711").
    pub version: Option<String>,
    /// OS build number (e.g. 26200).
    pub build: Option<u32>,
    /// Update revision (UBR) if readable.
    pub ubr: Option<u32>,
    /// Display version / release id (e.g. "25H2"), if readable.
    pub display_version: Option<String>,
    /// Native processor architecture (e.g. "x86_64").
    pub arch: Option<String>,
    /// Where the metadata came from, for audits.
    pub source: &'static str,
}

impl OsInfo {
    /// An `OsInfo` with every field unavailable.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            product_name: None,
            version: None,
            build: None,
            ubr: None,
            display_version: None,
            arch: None,
            source: "unavailable",
        }
    }
}

/// Build tool identity.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolInfo {
    /// Tool name (e.g. `ramshaper-cli`).
    pub name: String,
    /// Tool version.
    pub version: String,
}

/// Everything `build_snapshot` needs.
#[derive(Debug, Clone)]
pub struct SnapshotInputs {
    /// Raw collection results for this snapshot.
    pub sample: SystemSample,
    /// OS/build metadata.
    pub os: OsInfo,
    /// Tool metadata.
    pub tool: ToolInfo,
    /// Agent-instance identifier (pseudonymous, process-bound).
    pub agent_instance_id: String,
    /// Snapshot sequence number within this agent instance.
    pub sequence: u64,
    /// Wall-clock observation time (Unix epoch milliseconds, UTC).
    pub observed_at_unix_ms: u64,
    /// Monotonic position at collection time (age baseline).
    pub monotonic_ms: u64,
}

/// All metric envelopes of one snapshot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SystemMetrics {
    /// See `metric_names::PHYSICAL_TOTAL`.
    #[serde(rename = "physical_total_bytes")]
    pub physical_total: MetricEnvelope,
    /// See `metric_names::PHYSICAL_AVAILABLE`.
    #[serde(rename = "physical_available_bytes")]
    pub physical_available: MetricEnvelope,
    /// See `metric_names::PHYSICAL_IN_USE`.
    #[serde(rename = "physical_in_use_bytes")]
    pub physical_in_use: MetricEnvelope,
    /// See `metric_names::COMMIT_USED`.
    #[serde(rename = "commit_used_bytes")]
    pub commit_used: MetricEnvelope,
    /// See `metric_names::COMMIT_LIMIT`.
    #[serde(rename = "commit_limit_bytes")]
    pub commit_limit: MetricEnvelope,
    /// See `metric_names::COMMIT_HEADROOM`.
    #[serde(rename = "commit_headroom_bytes")]
    pub commit_headroom: MetricEnvelope,
    /// See `metric_names::LOW_MEMORY`.
    pub low_memory: MetricEnvelope,
}

impl SystemMetrics {
    /// All byte-metric envelopes, in display order.
    #[must_use]
    pub fn all_byte_metrics(&self) -> [&MetricEnvelope; 6] {
        [
            &self.physical_total,
            &self.physical_available,
            &self.physical_in_use,
            &self.commit_used,
            &self.commit_limit,
            &self.commit_headroom,
        ]
    }
}

/// One versioned snapshot (contract 14 §8).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SystemSnapshot {
    /// Schema identifier, e.g. `ramshaper.snapshot.v1`.
    pub schema: String,
    /// RFC 3339 UTC generation time.
    pub generated_at_utc: String,
    /// Pseudonymous agent-instance ID (process-bound).
    pub agent_instance_id: String,
    /// Snapshot sequence within this agent instance.
    pub sample_sequence: u64,
    /// OS/build metadata.
    pub os: OsInfo,
    /// Tool metadata.
    pub tool: ToolInfo,
    /// Metric envelopes.
    pub metrics: SystemMetrics,
    /// Monotonic collection time, used to compute `age_ms` on re-display.
    /// Never serialized: monotonic values do not survive restarts.
    #[serde(skip)]
    pub collected_monotonic_ms: u64,
}

/// Coarse health of a snapshot, for CLI exit codes (contract 14 §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotHealth {
    /// Every metric is OK.
    Ok,
    /// Usable global sample, but at least one metric is not OK.
    Partial,
    /// No usable global memory sample at all.
    NoUsableGlobalSample,
}

/// Compose a snapshot from raw inputs. Pure and deterministic.
#[must_use]
pub fn build_snapshot(inputs: SnapshotInputs) -> SystemSnapshot {
    let observed_at_utc = crate::rfc3339::format_utc_from_unix_ms(inputs.observed_at_unix_ms);
    let seq = inputs.sequence;
    let memory = &inputs.sample.memory;

    let physical_total = page_metric(
        metric_names::PHYSICAL_TOTAL,
        memory,
        |raw| raw.physical_total_pages,
        &observed_at_utc,
        seq,
    );
    let physical_available = page_metric(
        metric_names::PHYSICAL_AVAILABLE,
        memory,
        |raw| raw.physical_available_pages,
        &observed_at_utc,
        seq,
    );
    let commit_used = page_metric(
        metric_names::COMMIT_USED,
        memory,
        |raw| raw.commit_total_pages,
        &observed_at_utc,
        seq,
    );
    let commit_limit = page_metric(
        metric_names::COMMIT_LIMIT,
        memory,
        |raw| raw.commit_limit_pages,
        &observed_at_utc,
        seq,
    );
    let physical_in_use = difference_metric(
        metric_names::PHYSICAL_IN_USE,
        memory,
        |raw| (raw.physical_total_pages, raw.physical_available_pages),
        "derived: in_use = total - available (one snapshot)",
        &observed_at_utc,
        seq,
    );
    let commit_headroom = difference_metric(
        metric_names::COMMIT_HEADROOM,
        memory,
        |raw| (raw.commit_limit_pages, raw.commit_total_pages),
        "derived: headroom = limit - used (one snapshot)",
        &observed_at_utc,
        seq,
    );

    let low_memory = match &inputs.sample.low_memory {
        Ok(state) => MetricEnvelope::ok_signal(
            metric_names::LOW_MEMORY,
            sources::MEMORY_RESOURCE_NOTIFICATION,
            *state,
            observed_at_utc.clone(),
            seq,
        ),
        Err(failure) => unavailable_envelope(
            metric_names::LOW_MEMORY,
            sources::MEMORY_RESOURCE_NOTIFICATION,
            failure,
            &observed_at_utc,
            seq,
        ),
    };

    SystemSnapshot {
        schema: SNAPSHOT_SCHEMA.to_string(),
        generated_at_utc: observed_at_utc,
        agent_instance_id: inputs.agent_instance_id,
        sample_sequence: seq,
        os: inputs.os,
        tool: inputs.tool,
        metrics: SystemMetrics {
            physical_total,
            physical_available,
            physical_in_use,
            commit_used,
            commit_limit,
            commit_headroom,
            low_memory,
        },
        collected_monotonic_ms: inputs.monotonic_ms,
    }
}

/// Classify snapshot health for exit-code mapping (contract 14 §8).
#[must_use]
pub fn classify_snapshot(snapshot: &SystemSnapshot) -> SnapshotHealth {
    let m = &snapshot.metrics;
    let usable_global = m.physical_total.status.is_ok()
        || m.physical_available.status.is_ok()
        || m.commit_used.status.is_ok();
    if !usable_global {
        return SnapshotHealth::NoUsableGlobalSample;
    }
    let every_metric_ok = m.physical_total.status.is_ok()
        && m.physical_available.status.is_ok()
        && m.physical_in_use.status.is_ok()
        && m.commit_used.status.is_ok()
        && m.commit_limit.status.is_ok()
        && m.commit_headroom.status.is_ok()
        && m.low_memory.status.is_ok();
    if every_metric_ok {
        SnapshotHealth::Ok
    } else {
        SnapshotHealth::Partial
    }
}

/// Envelope for a failed source query.
fn unavailable_envelope(
    name: &'static str,
    source: &'static str,
    failure: &QueryFailure,
    observed_at_utc: &str,
    seq: u64,
) -> MetricEnvelope {
    MetricEnvelope::unavailable(
        name,
        source,
        failure.to_status(),
        Some(&failure.error_code()),
        Some(failure.to_string()),
        observed_at_utc.to_string(),
        seq,
    )
}

/// A direct page-count metric converted with checked arithmetic.
fn page_metric(
    name: &'static str,
    memory: &Result<GlobalMemoryRaw, QueryFailure>,
    selector: fn(&GlobalMemoryRaw) -> u64,
    observed_at_utc: &str,
    seq: u64,
) -> MetricEnvelope {
    match memory {
        Ok(raw) => match pages_to_bytes(selector(raw), raw.page_size_bytes) {
            Ok(bytes) => MetricEnvelope::ok_bytes(
                name,
                sources::PERFORMANCE_INFO,
                bytes,
                observed_at_utc.to_string(),
                seq,
            ),
            Err(e) => MetricEnvelope::unavailable(
                name,
                sources::PERFORMANCE_INFO,
                MetricStatus::Error,
                Some(&conversion_code(&e)),
                Some(e.to_string()),
                observed_at_utc.to_string(),
                seq,
            ),
        },
        Err(failure) => unavailable_envelope(
            name,
            sources::PERFORMANCE_INFO,
            failure,
            observed_at_utc,
            seq,
        ),
    }
}

/// A derived `minuend - subtrahend` byte metric from two page counts of one
/// snapshot. An impossible combination becomes an explicit error envelope,
/// never a wrapped value.
fn difference_metric(
    name: &'static str,
    memory: &Result<GlobalMemoryRaw, QueryFailure>,
    selector: fn(&GlobalMemoryRaw) -> (u64, u64),
    derivation: &str,
    observed_at_utc: &str,
    seq: u64,
) -> MetricEnvelope {
    match memory {
        Ok(raw) => {
            let (minuend_pages, subtrahend_pages) = selector(raw);
            let conversion = pages_to_bytes(minuend_pages, raw.page_size_bytes)
                .and_then(|a| Ok((a, pages_to_bytes(subtrahend_pages, raw.page_size_bytes)?)));
            match conversion {
                Ok((a, b)) => match byte_difference(a, b) {
                    Some(bytes) => {
                        let mut envelope = MetricEnvelope::ok_bytes(
                            name,
                            sources::PERFORMANCE_INFO,
                            bytes,
                            observed_at_utc.to_string(),
                            seq,
                        );
                        envelope.detail = Some(derivation.to_string());
                        envelope
                    }
                    None => MetricEnvelope::unavailable(
                        name,
                        sources::PERFORMANCE_INFO,
                        MetricStatus::Error,
                        Some("inconsistent_counters"),
                        Some(
                            "subtrahend exceeds minuend in one snapshot; refusing to derive"
                                .to_string(),
                        ),
                        observed_at_utc.to_string(),
                        seq,
                    ),
                },
                Err(e) => MetricEnvelope::unavailable(
                    name,
                    sources::PERFORMANCE_INFO,
                    MetricStatus::Error,
                    Some(&conversion_code(&e)),
                    Some(e.to_string()),
                    observed_at_utc.to_string(),
                    seq,
                ),
            }
        }
        Err(failure) => unavailable_envelope(
            name,
            sources::PERFORMANCE_INFO,
            failure,
            observed_at_utc,
            seq,
        ),
    }
}

fn conversion_code(e: &PageConversionError) -> String {
    match e {
        PageConversionError::ZeroPageSize => "zero_page_size".to_string(),
        PageConversionError::Overflow => "conversion_overflow".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic AT-02 fixture: 16 GiB physical, 4 GiB available, 12 GiB
    /// commit used, 32 GiB commit limit, 4 KiB pages. All values exact.
    fn raw_16gib_4k() -> GlobalMemoryRaw {
        GlobalMemoryRaw {
            physical_total_pages: 4_194_304,
            physical_available_pages: 1_048_576,
            commit_total_pages: 3_145_728,
            commit_limit_pages: 8_388_608,
            page_size_bytes: 4096,
        }
    }

    fn inputs(sample: SystemSample) -> SnapshotInputs {
        SnapshotInputs {
            sample,
            os: OsInfo::unknown(),
            tool: ToolInfo {
                name: "ramshaper-cli".to_string(),
                version: "0.1.0".to_string(),
            },
            agent_instance_id: "cli-test-1".to_string(),
            sequence: 1,
            observed_at_unix_ms: 1_792_672_496_789, // 2026-10-22T12:34:56.789Z
            monotonic_ms: 42,
        }
    }

    fn ok_sample() -> SystemSample {
        SystemSample {
            memory: Ok(raw_16gib_4k()),
            low_memory: Ok(LowMemorySignal::NotSignaled),
        }
    }

    #[test]
    fn exact_byte_values_from_page_fixtures() {
        let snap = build_snapshot(inputs(ok_sample()));
        let m = &snap.metrics;
        assert_eq!(m.physical_total.value, Some(17_179_869_184)); // 16 GiB
        assert_eq!(m.physical_available.value, Some(4_294_967_296)); // 4 GiB
        assert_eq!(m.physical_in_use.value, Some(12_884_901_888)); // 12 GiB
        assert_eq!(m.commit_used.value, Some(12_884_901_888)); // 12 GiB
        assert_eq!(m.commit_limit.value, Some(34_359_738_368)); // 32 GiB
        assert_eq!(m.commit_headroom.value, Some(21_474_836_480)); // 20 GiB
        assert_eq!(m.low_memory.state, Some(LowMemorySignal::NotSignaled));
        assert!(snap
            .metrics
            .all_byte_metrics()
            .iter()
            .all(|e| e.status.is_ok()));
        assert_eq!(snap.generated_at_utc, "2026-10-22T12:34:56.789Z");
        assert_eq!(snap.schema, "ramshaper.snapshot.v1");
    }

    #[test]
    fn derived_metrics_carry_derivation_notes() {
        let snap = build_snapshot(inputs(ok_sample()));
        assert!(snap
            .metrics
            .physical_in_use
            .detail
            .expect("note")
            .contains("total - available"));
        assert!(snap
            .metrics
            .commit_headroom
            .detail
            .expect("note")
            .contains("limit - used"));
    }

    #[test]
    fn available_exceeding_total_is_rejected_not_wrapped() {
        let mut raw = raw_16gib_4k();
        raw.physical_available_pages = raw.physical_total_pages + 1;
        let snap = build_snapshot(inputs(SystemSample {
            memory: Ok(raw),
            low_memory: Ok(LowMemorySignal::NotSignaled),
        }));
        assert_eq!(snap.metrics.physical_in_use.status, MetricStatus::Error);
        assert_eq!(snap.metrics.physical_in_use.value, None);
        assert_eq!(
            snap.metrics.physical_in_use.error_code.as_deref(),
            Some("inconsistent_counters")
        );
        // Raw readings remain OK; only the derived metric is rejected.
        assert!(snap.metrics.physical_total.status.is_ok());
        assert!(snap.metrics.physical_available.status.is_ok());
        assert_eq!(classify_snapshot(&snap), SnapshotHealth::Partial);
    }

    #[test]
    fn commit_used_exceeding_limit_rejects_headroom() {
        let mut raw = raw_16gib_4k();
        raw.commit_total_pages = raw.commit_limit_pages + 7;
        let snap = build_snapshot(inputs(SystemSample {
            memory: Ok(raw),
            low_memory: Ok(LowMemorySignal::NotSignaled),
        }));
        assert_eq!(snap.metrics.commit_headroom.status, MetricStatus::Error);
        assert_eq!(snap.metrics.commit_headroom.value, None);
    }

    #[test]
    fn zero_page_size_fails_every_conversion_explicitly() {
        let mut raw = raw_16gib_4k();
        raw.page_size_bytes = 0;
        let snap = build_snapshot(inputs(SystemSample {
            memory: Ok(raw),
            low_memory: Ok(LowMemorySignal::NotSignaled),
        }));
        for e in snap.metrics.all_byte_metrics() {
            assert_eq!(e.status, MetricStatus::Error, "{}", e.name);
            assert_eq!(e.value, None);
            assert_eq!(e.error_code.as_deref(), Some("zero_page_size"));
        }
        assert_eq!(
            classify_snapshot(&snap),
            SnapshotHealth::NoUsableGlobalSample
        );
    }

    #[test]
    fn overflowing_page_counts_are_errors() {
        let mut raw = raw_16gib_4k();
        raw.physical_total_pages = u64::MAX;
        raw.commit_limit_pages = u64::MAX;
        let snap = build_snapshot(inputs(SystemSample {
            memory: Ok(raw),
            low_memory: Ok(LowMemorySignal::NotSignaled),
        }));
        assert_eq!(snap.metrics.physical_total.status, MetricStatus::Error);
        assert_eq!(
            snap.metrics.physical_total.error_code.as_deref(),
            Some("conversion_overflow")
        );
        assert_eq!(snap.metrics.commit_limit.status, MetricStatus::Error);
        assert_eq!(snap.metrics.commit_limit.value, None);
        // Non-overflowing metrics from the same snapshot stay usable.
        assert!(snap.metrics.physical_available.status.is_ok());
        assert_eq!(classify_snapshot(&snap), SnapshotHealth::Partial);
    }

    #[test]
    fn memory_query_failure_maps_to_typed_statuses() {
        for (failure, status) in [
            (QueryFailure::AccessDenied, MetricStatus::AccessDenied),
            (
                QueryFailure::Unsupported("not present".into()),
                MetricStatus::Unsupported,
            ),
            (QueryFailure::Exited { code: 87 }, MetricStatus::Exited),
            (
                QueryFailure::OsError {
                    code: 1455,
                    function: "GetPerformanceInfo",
                },
                MetricStatus::Error,
            ),
        ] {
            let snap = build_snapshot(inputs(SystemSample {
                memory: Err(failure.clone()),
                low_memory: Ok(LowMemorySignal::Signaled),
            }));
            for e in snap.metrics.all_byte_metrics() {
                assert_eq!(e.status, status, "{}", e.name);
                assert_eq!(e.value, None, "failed reading must not become zero");
            }
            // Low memory itself is healthy here: partial, not total failure.
            assert_eq!(
                classify_snapshot(&snap),
                SnapshotHealth::NoUsableGlobalSample
            );
        }
    }

    #[test]
    fn low_memory_failure_is_partial_not_fatal() {
        let snap = build_snapshot(inputs(SystemSample {
            memory: Ok(raw_16gib_4k()),
            low_memory: Err(QueryFailure::Unsupported(
                "CreateMemoryResourceNotification failed".into(),
            )),
        }));
        assert_eq!(snap.metrics.low_memory.status, MetricStatus::Unsupported);
        assert_eq!(snap.metrics.low_memory.state, None);
        assert_eq!(classify_snapshot(&snap), SnapshotHealth::Partial);
    }

    #[test]
    fn healthy_fixture_classifies_ok() {
        let snap = build_snapshot(inputs(ok_sample()));
        assert_eq!(classify_snapshot(&snap), SnapshotHealth::Ok);
    }

    #[test]
    fn json_is_one_versioned_object_with_integer_bytes() {
        let snap = build_snapshot(inputs(ok_sample()));
        let json = serde_json::to_value(&snap).expect("ser");
        assert_eq!(json["schema"], "ramshaper.snapshot.v1");
        assert_eq!(json["sample_sequence"], 1);
        assert_eq!(
            json["metrics"]["physical_total_bytes"]["value"],
            17_179_869_184u64
        );
        assert!(json["metrics"]["physical_total_bytes"]["value"].is_u64());
        assert_eq!(json["metrics"]["low_memory"]["state"], "not_signaled");
        // Unavailable variants must serialize without a value key at all.
        let partial = build_snapshot(inputs(SystemSample {
            memory: Ok(raw_16gib_4k()),
            low_memory: Err(QueryFailure::AccessDenied),
        }));
        let pj = serde_json::to_value(&partial).expect("ser");
        assert!(pj["metrics"]["low_memory"].get("state").is_none());
        assert_eq!(pj["metrics"]["low_memory"]["status"], "access_denied");
    }
}
