//! Metric status values (contract 14 §2).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Status of one metric reading. A non-OK metric never has a value;
/// missing/denied/failed readings must not silently become zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricStatus {
    /// Reading collected successfully.
    Ok,
    /// First sample of a rate/derived series; baseline not established yet.
    WarmingUp,
    /// The OS denied access to the queried object.
    AccessDenied,
    /// The capability or counter is not supported on this system/build.
    Unsupported,
    /// The queried process has exited or the PID no longer resolves.
    Exited,
    /// The query failed with an OS error.
    Error,
    /// The reading is valid but too old for display/policy use.
    Stale,
}

impl MetricStatus {
    /// Stable machine-readable name (used in JSON).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MetricStatus::Ok => "ok",
            MetricStatus::WarmingUp => "warming_up",
            MetricStatus::AccessDenied => "access_denied",
            MetricStatus::Unsupported => "unsupported",
            MetricStatus::Exited => "exited",
            MetricStatus::Error => "error",
            MetricStatus::Stale => "stale",
        }
    }

    /// True only for `Ok`.
    #[must_use]
    pub fn is_ok(self) -> bool {
        self == MetricStatus::Ok
    }
}

impl fmt::Display for MetricStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_uses_snake_case_names() {
        assert_eq!(
            serde_json::to_string(&MetricStatus::Ok).expect("ser"),
            "\"ok\""
        );
        assert_eq!(
            serde_json::to_string(&MetricStatus::AccessDenied).expect("ser"),
            "\"access_denied\""
        );
        let back: MetricStatus = serde_json::from_str("\"warming_up\"").expect("de");
        assert_eq!(back, MetricStatus::WarmingUp);
    }
}
