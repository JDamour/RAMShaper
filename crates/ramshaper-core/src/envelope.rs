//! Metric envelopes (contract 14 §2).
//!
//! Every metric carries `value`/`state`, `status`, `source`, `observed_at_utc`,
//! `sample_sequence` and `age_ms`, plus an optional machine-readable error
//! code. Invariant enforced by the constructors: a non-OK metric never carries
//! a value (so missing/denied/failed data can never masquerade as zero).

use serde::{Deserialize, Serialize};
use std::fmt;

/// State of the OS low-memory resource condition. A signal is an OS resource
/// condition; it does not by itself prove a user-visible slowdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LowMemorySignal {
    /// The low-memory resource condition is currently signaled.
    Signaled,
    /// The low-memory resource condition is not signaled.
    NotSignaled,
}

impl LowMemorySignal {
    /// Stable machine-readable name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            LowMemorySignal::Signaled => "signaled",
            LowMemorySignal::NotSignaled => "not_signaled",
        }
    }
}

impl fmt::Display for LowMemorySignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LowMemorySignal::Signaled => f.write_str("signaled"),
            LowMemorySignal::NotSignaled => f.write_str("not signaled"),
        }
    }
}

/// One metric reading with provenance, status and age. Serialization only:
/// `name`/`source` are `'static` and not deserialized from external data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetricEnvelope {
    /// Metric name, stable across releases (e.g. `physical_total_bytes`).
    pub name: &'static str,
    /// Byte value, present only when `status == ok`. Integer, never a float.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<u64>,
    /// Low-memory condition state, present only for `low_memory` when OK.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<LowMemorySignal>,
    /// Collection status.
    pub status: crate::status::MetricStatus,
    /// OS API the value came from (e.g. `GetPerformanceInfo`).
    pub source: &'static str,
    /// RFC 3339 UTC timestamp of the observation.
    pub observed_at_utc: String,
    /// Sequence number of the containing snapshot.
    pub sample_sequence: u64,
    /// Age in milliseconds, computed in the current agent instance.
    pub age_ms: u64,
    /// Machine-readable error code when the status is not OK (e.g. a Win32
    /// error code or an internal conversion code such as `ZeroPageSize`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Short human-readable explanation (redacted text, no user data).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl MetricEnvelope {
    fn envelope(
        name: &'static str,
        source: &'static str,
        status: crate::status::MetricStatus,
        observed_at_utc: String,
        sample_sequence: u64,
    ) -> Self {
        Self {
            name,
            value: None,
            state: None,
            status,
            source,
            observed_at_utc,
            sample_sequence,
            age_ms: 0,
            error_code: None,
            detail: None,
        }
    }

    /// Successful byte-metric envelope.
    #[must_use]
    pub fn ok_bytes(
        name: &'static str,
        source: &'static str,
        bytes: u64,
        observed_at_utc: String,
        sample_sequence: u64,
    ) -> Self {
        Self::envelope(
            name,
            source,
            crate::status::MetricStatus::Ok,
            observed_at_utc,
            sample_sequence,
        )
        .with_value(bytes)
    }

    /// Successful low-memory state envelope.
    #[must_use]
    pub fn ok_signal(
        name: &'static str,
        source: &'static str,
        state: LowMemorySignal,
        observed_at_utc: String,
        sample_sequence: u64,
    ) -> Self {
        Self::envelope(
            name,
            source,
            crate::status::MetricStatus::Ok,
            observed_at_utc,
            sample_sequence,
        )
        .with_state(state)
    }

    /// Non-OK envelope. The status must not be `ok`; value and state stay
    /// absent so unavailable data can never read as zero.
    #[must_use]
    pub fn unavailable(
        name: &'static str,
        source: &'static str,
        status: crate::status::MetricStatus,
        error_code: Option<&str>,
        detail: Option<String>,
        observed_at_utc: String,
        sample_sequence: u64,
    ) -> Self {
        debug_assert!(
            !status.is_ok(),
            "unavailable envelopes must not use status ok"
        );
        let mut e = Self::envelope(name, source, status, observed_at_utc, sample_sequence);
        e.error_code = error_code.map(str::to_string);
        e.detail = detail;
        e
    }

    fn with_value(mut self, bytes: u64) -> Self {
        self.value = Some(bytes);
        self
    }

    fn with_state(mut self, state: LowMemorySignal) -> Self {
        self.state = Some(state);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::MetricStatus;

    fn at() -> String {
        "2026-10-22T00:00:00.000Z".to_string()
    }

    #[test]
    fn ok_envelope_carries_value() {
        let e =
            MetricEnvelope::ok_bytes("physical_total_bytes", "GetPerformanceInfo", 4096, at(), 1);
        assert_eq!(e.value, Some(4096));
        assert!(e.status.is_ok());
        assert_eq!(e.state, None);
    }

    #[test]
    fn unavailable_envelope_never_carries_a_value() {
        let e = MetricEnvelope::unavailable(
            "physical_total_bytes",
            "GetPerformanceInfo",
            MetricStatus::AccessDenied,
            Some("5"),
            Some("OpenProcess denied".to_string()),
            at(),
            1,
        );
        assert_eq!(e.value, None);
        assert_eq!(e.state, None);
        assert_eq!(e.status, MetricStatus::AccessDenied);
        assert_eq!(e.error_code.as_deref(), Some("5"));
    }

    #[test]
    fn json_shape_and_field_set() {
        let e = MetricEnvelope::ok_signal(
            "low_memory",
            "QueryMemoryResourceNotification",
            LowMemorySignal::NotSignaled,
            at(),
            7,
        );
        let json = serde_json::to_value(&e).expect("ser");
        assert_eq!(json["state"], "not_signaled");
        assert_eq!(json["status"], "ok");
        assert_eq!(json["source"], "QueryMemoryResourceNotification");
        assert_eq!(json["sample_sequence"], 7);
        assert_eq!(json["age_ms"], 0);
        // Absent-optionals are omitted, not null.
        assert!(json.get("value").is_none());
        assert!(json.get("error_code").is_none());
        assert!(json.get("detail").is_none());
    }
}
