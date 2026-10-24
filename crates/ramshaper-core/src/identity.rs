//! Process identity: PID plus the exact native creation marker.
//!
//! Windows identity is `(pid, creation FILETIME in 100-ns units)` from
//! `GetProcessTimes` (contract 14 §1, ADR-008). The creation value is an OS
//! identity marker preserved at full precision — it is **not** a monotonic
//! duration clock and must never be used to measure elapsed time. Owner/session
//! context is deliberately *not* part of this type.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique process identity within one OS instance: PID plus the unmodified
/// creation FILETIME. Two identities are equal only if both fields match; a
/// reused PID gets a new creation marker and compares unequal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessIdentity {
    /// OS process identifier (recyclable on its own).
    pub pid: u32,
    /// Creation FILETIME in 100-nanosecond units, exactly as reported by the
    /// OS. Full 64-bit precision; serialized as a decimal string in JSON to
    /// avoid consumer precision loss.
    pub creation_filetime_100ns: u64,
}

impl ProcessIdentity {
    /// Construct an identity from raw parts (adapter boundary).
    #[must_use]
    pub const fn new(pid: u32, creation_filetime_100ns: u64) -> Self {
        Self {
            pid,
            creation_filetime_100ns,
        }
    }

    /// True when `self` and `observed` refer to the same OS process instance.
    #[must_use]
    pub fn matches(&self, observed: &ProcessIdentity) -> bool {
        self.pid == observed.pid && self.creation_filetime_100ns == observed.creation_filetime_100ns
    }
}

impl fmt::Display for ProcessIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "pid {} (creation {} × 100 ns)",
            self.pid, self.creation_filetime_100ns
        )
    }
}

// JSON carries FILETIME identities as decimal strings (contract 14 §2).
impl Serialize for ProcessIdentity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Raw<'a> {
            pid: u32,
            creation_filetime_100ns: String,
            #[serde(rename = "creation_filetime_note")]
            note: &'a str,
        }
        Raw {
            pid: self.pid,
            creation_filetime_100ns: self.creation_filetime_100ns.to_string(),
            note: "exact 100-ns FILETIME as decimal string; identity marker, not a clock",
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ProcessIdentity {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            pid: u32,
            creation_filetime_100ns: String,
        }
        let raw = Raw::deserialize(deserializer)?;
        let creation = raw
            .creation_filetime_100ns
            .parse::<u64>()
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            pid: raw.pid,
            creation_filetime_100ns: creation,
        })
    }
}

/// Result of re-checking an identity against the current OS state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityVerification {
    /// The PID still resolves to a process with the exact same creation
    /// marker: same instance.
    #[error("identity matches")]
    Matches,
    /// The PID resolves, but to a different creation marker: the original
    /// process exited and the PID was reused. The replacement must never
    /// inherit history or authority from the old identity.
    #[error("pid now belongs to a different process instance (observed {observed})")]
    Mismatch {
        /// What the PID currently resolves to.
        observed: ProcessIdentity,
    },
    /// The identity can no longer be queried (process exited, access denied
    /// or an OS error). Exited and denied are expected, safe outcomes.
    #[error("identity unreachable: {reason}")]
    Unreachable {
        /// Why the identity could not be re-queried.
        reason: crate::snapshot::QueryFailure,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::QueryFailure;

    #[test]
    fn identity_equality_requires_both_fields() {
        let a = ProcessIdentity::new(42, 1_000);
        let same = ProcessIdentity::new(42, 1_000);
        let reused = ProcessIdentity::new(42, 2_000); // same PID, new instance
        let other_pid = ProcessIdentity::new(43, 1_000);
        assert!(a.matches(&same));
        assert!(!a.matches(&reused));
        assert!(!a.matches(&other_pid));
    }

    #[test]
    fn json_serializes_filetime_as_decimal_string() {
        let id = ProcessIdentity::new(1234, 133_928_374_849_754_321);
        let json = serde_json::to_string(&id).expect("serialize");
        assert!(json.contains("\"creation_filetime_100ns\":\"133928374849754321\""));
        let back: ProcessIdentity = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, id);
    }

    #[test]
    fn verification_variants_distinguish_reuse_from_gone() {
        let original = ProcessIdentity::new(7, 10);
        let replacement = ProcessIdentity::new(7, 999);
        assert_eq!(
            IdentityVerification::Mismatch {
                observed: replacement
            },
            IdentityVerification::Mismatch {
                observed: replacement
            }
        );
        let unreachable = IdentityVerification::Unreachable {
            reason: QueryFailure::Exited { code: 87 },
        };
        assert_ne!(unreachable, IdentityVerification::Matches);
        assert!(format!("{unreachable}").contains("exited or PID not found"));
        // The mismatch variant must surface the observed identity for audits.
        assert!(format!(
            "{}",
            IdentityVerification::Mismatch {
                observed: replacement
            }
        )
        .contains("999"));
        assert!(original.matches(&original));
    }
}
