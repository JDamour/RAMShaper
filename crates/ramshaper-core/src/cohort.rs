//! AT-21 hardware-cohort classification (RS-025, observe-only).
//!
//! The declared measurement cohorts are 4, 8 and 16 GiB of total physical
//! memory. This module is pure: it classifies a *measured* total-physical-
//! memory reading into a cohort, and cross-checks a *declared* cohort
//! (from the closed `[cohort]` config section) against that reading so a
//! measurement can never be published under a cohort the machine does not
//! belong to — a larger machine cannot substitute for a declared cohort.
//! A machine that matches no declared cohort classifies as
//! [`HardwareCohort::NonCohort`]; its observations are baselines only.
//!
//! Membership bands: a machine belongs to a nominal cohort when its measured
//! total is within [0.80, 1.20) × nominal bytes. The band absorbs OS/hardware
//! reservations (a "64 GiB" machine reports 63.75 GiB here) while the bands
//! of adjacent cohorts (powers of two apart) can never overlap, and in-between
//! sizes (6, 12, 32 GiB, …) fall outside every band and stay non-cohort.
//!
//! Classification is labeling and refusal only: it never changes collection
//! behavior and grants no authority (config stays observe-only).

use serde::{Deserialize, Serialize};

/// Nominal size of the 4 GiB cohort, bytes.
pub const COHORT_4GIB_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Nominal size of the 8 GiB cohort, bytes.
pub const COHORT_8GIB_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Nominal size of the 16 GiB cohort, bytes.
pub const COHORT_16GIB_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Cohort membership band, as exact fractions of the nominal size:
/// `[4/5 × nominal, 6/5 × nominal)` (±20%). Exact rational bounds keep the
/// bands deterministic and non-overlapping (adjacent cohorts are 2× apart,
/// so the 12.5% reservation shortfall of the validation machine is inside a
/// band while in-between sizes like 6 or 12 GiB stay outside every band).
const BAND_DENOMINATOR: u64 = 5;
const BAND_LO_NUMERATOR: u64 = 4;
const BAND_HI_NUMERATOR: u64 = 6;

/// Declared hardware cohort for AT-21 measurement runs.
///
/// `NonCohort` ("none") is the only value a machine can always declare:
/// it marks every observation as a non-cohort baseline. `4gib`/`8gib`/
/// `16gib` are accepted only when the measured total physical memory
/// classifies the machine into the same cohort (see [`verify_declared`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HardwareCohort {
    /// 4 GiB cohort ("4gib").
    #[serde(rename = "4gib")]
    Cohort4GiB,
    /// 8 GiB cohort ("8gib").
    #[serde(rename = "8gib")]
    Cohort8GiB,
    /// 16 GiB cohort ("16gib").
    #[serde(rename = "16gib")]
    Cohort16GiB,
    /// No declared cohort ("none"): observations are non-cohort baselines.
    #[serde(rename = "none")]
    NonCohort,
}

impl HardwareCohort {
    /// Every valid TOML declaration string, for error messages.
    pub const VALID_DECLARATIONS: &'static str = "'4gib', '8gib', '16gib', 'none'";

    /// Canonical string used in TOML declarations and JSON reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cohort4GiB => "4gib",
            Self::Cohort8GiB => "8gib",
            Self::Cohort16GiB => "16gib",
            Self::NonCohort => "none",
        }
    }

    /// Parse a TOML declaration string. Unknown strings are errors, never
    /// silently a default.
    ///
    /// # Errors
    /// Returns the input unchanged inside the error for field-level reporting.
    pub fn parse_declaration(text: &str) -> Result<Self, UnknownCohortDeclaration> {
        match text {
            "4gib" => Ok(Self::Cohort4GiB),
            "8gib" => Ok(Self::Cohort8GiB),
            "16gib" => Ok(Self::Cohort16GiB),
            "none" => Ok(Self::NonCohort),
            other => Err(UnknownCohortDeclaration(other.to_string())),
        }
    }

    /// Nominal size in bytes, or `None` for [`HardwareCohort::NonCohort`].
    #[must_use]
    pub fn nominal_bytes(self) -> Option<u64> {
        match self {
            Self::Cohort4GiB => Some(COHORT_4GIB_BYTES),
            Self::Cohort8GiB => Some(COHORT_8GIB_BYTES),
            Self::Cohort16GiB => Some(COHORT_16GIB_BYTES),
            Self::NonCohort => None,
        }
    }

    /// Membership band `[lo, hi)` in bytes, or `None` for
    /// [`HardwareCohort::NonCohort`].
    #[must_use]
    pub fn band(self) -> Option<(u64, u64)> {
        let nominal = self.nominal_bytes()?;
        let lo = nominal * BAND_LO_NUMERATOR / BAND_DENOMINATOR;
        let hi = nominal * BAND_HI_NUMERATOR / BAND_DENOMINATOR;
        Some((lo, hi))
    }

    /// Classify a *measured* total physical memory reading. This is what the
    /// machine actually is — independent of any declaration.
    #[must_use]
    pub fn from_total_bytes(total_physical_bytes: u64) -> Self {
        for cohort in [Self::Cohort4GiB, Self::Cohort8GiB, Self::Cohort16GiB] {
            let (lo, hi) = cohort.band().unwrap_or((0, 0));
            if (lo..hi).contains(&total_physical_bytes) {
                return cohort;
            }
        }
        Self::NonCohort
    }
}

/// An unparsable `[cohort] declared` string, echoed for the field-level error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown cohort declaration '{0}'")]
pub struct UnknownCohortDeclaration(pub String);

/// Why a declared cohort could not be verified against this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CohortError {
    /// The machine's measured memory does not classify into the declared
    /// cohort. This is the "no better-machine substitution" refusal: results
    /// from a machine outside the cohort must not be published as cohort data.
    #[error(
        "declared cohort '{}' does not match this machine (measured total physical \
         memory classifies as '{}'); a machine outside a cohort cannot substitute for it",
        declared.as_str(),
        measured.as_str()
    )]
    DeclaredCohortMismatch {
        /// Cohort from the configuration.
        declared: HardwareCohort,
        /// What the measured memory classifies as.
        measured: HardwareCohort,
    },
    /// The machine's total physical memory was unavailable, so a declared
    /// cohort cannot be verified at all.
    #[error(
        "declared cohort '{}' cannot be verified: total physical memory is unavailable \
         on this machine",
        declared.as_str()
    )]
    MeasuredTotalUnavailable {
        /// Cohort from the configuration.
        declared: HardwareCohort,
    },
}

/// Cross-check a declared cohort against the measured total physical memory
/// and return the classification the observation must carry.
///
/// - `declared = NonCohort` always verifies: the observation is a baseline.
/// - a named cohort verifies only when the measured memory classifies the
///   machine into exactly that cohort;
/// - `measured_total_bytes = None` cannot verify a named cohort.
///
/// # Errors
/// [`CohortError::DeclaredCohortMismatch`] when the machine's measured memory
/// classifies outside the declared cohort, [`CohortError::MeasuredTotalUnavailable`]
/// when no reading exists to verify a named cohort against.
pub fn verify_declared(
    declared: HardwareCohort,
    measured_total_bytes: Option<u64>,
) -> Result<HardwareCohort, CohortError> {
    match declared {
        HardwareCohort::NonCohort => Ok(HardwareCohort::NonCohort),
        cohort => {
            let Some(total) = measured_total_bytes else {
                return Err(CohortError::MeasuredTotalUnavailable { declared: cohort });
            };
            let measured = HardwareCohort::from_total_bytes(total);
            if measured == cohort {
                Ok(cohort)
            } else {
                Err(CohortError::DeclaredCohortMismatch {
                    declared: cohort,
                    measured,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn nominal_sizes_classify_into_their_cohort() {
        assert_eq!(
            HardwareCohort::from_total_bytes(4 * GIB),
            HardwareCohort::Cohort4GiB
        );
        assert_eq!(
            HardwareCohort::from_total_bytes(8 * GIB),
            HardwareCohort::Cohort8GiB
        );
        assert_eq!(
            HardwareCohort::from_total_bytes(16 * GIB),
            HardwareCohort::Cohort16GiB
        );
    }

    #[test]
    fn band_edges_are_inclusive_low_and_exclusive_high() {
        let (lo, hi) = HardwareCohort::Cohort8GiB.band().expect("band");
        assert_eq!(lo, 8 * GIB * 4 / 5);
        assert_eq!(hi, 8 * GIB * 6 / 5);
        assert_eq!(
            HardwareCohort::from_total_bytes(lo),
            HardwareCohort::Cohort8GiB
        );
        assert_eq!(
            HardwareCohort::from_total_bytes(lo - 1),
            HardwareCohort::NonCohort
        );
        assert_eq!(
            HardwareCohort::from_total_bytes(hi - 1),
            HardwareCohort::Cohort8GiB
        );
        assert_eq!(
            HardwareCohort::from_total_bytes(hi),
            HardwareCohort::NonCohort
        );
    }

    #[test]
    fn in_between_and_larger_sizes_are_non_cohort() {
        for total in [2 * GIB, 6 * GIB, 12 * GIB, 32 * GIB, u64::MAX] {
            assert_eq!(
                HardwareCohort::from_total_bytes(total),
                HardwareCohort::NonCohort,
                "{total} bytes must not classify into any declared cohort"
            );
        }
    }

    /// The validation machine (64 GiB nominal, reporting 63.75 GiB) is a
    /// non-cohort baseline. Recorded value from 16_IMPLEMENTATION_EVIDENCE.md.
    #[test]
    fn validation_machine_64gib_is_non_cohort_baseline() {
        assert_eq!(
            HardwareCohort::from_total_bytes(68_448_034_816),
            HardwareCohort::NonCohort
        );
    }

    #[test]
    fn reservation_shortfalls_stay_in_cohort() {
        // A 16 GiB machine reporting 15.5 GiB; an 8 GiB machine at 7.4 GiB.
        assert_eq!(
            HardwareCohort::from_total_bytes(16 * GIB - GIB / 2),
            HardwareCohort::Cohort16GiB
        );
        assert_eq!(
            HardwareCohort::from_total_bytes(8 * GIB - 3 * GIB / 5),
            HardwareCohort::Cohort8GiB
        );
    }

    #[test]
    fn bands_never_overlap() {
        let mut cohort_ranges = [
            HardwareCohort::Cohort4GiB,
            HardwareCohort::Cohort8GiB,
            HardwareCohort::Cohort16GiB,
        ]
        .map(|c| c.band().expect("band"));
        cohort_ranges.sort_by_key(|&(lo, _)| lo);
        for pair in cohort_ranges.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "bands overlap: {pair:?}");
        }
    }

    #[test]
    fn declaration_strings_roundtrip() {
        for cohort in [
            HardwareCohort::Cohort4GiB,
            HardwareCohort::Cohort8GiB,
            HardwareCohort::Cohort16GiB,
            HardwareCohort::NonCohort,
        ] {
            assert_eq!(
                HardwareCohort::parse_declaration(cohort.as_str()),
                Ok(cohort)
            );
        }
        assert_eq!(
            HardwareCohort::parse_declaration("64gib"),
            Err(UnknownCohortDeclaration("64gib".to_string()))
        );
        assert_eq!(
            HardwareCohort::parse_declaration(""),
            Err(UnknownCohortDeclaration(String::new()))
        );
    }

    #[test]
    fn serde_uses_canonical_strings() {
        assert_eq!(
            serde_json::to_value(HardwareCohort::Cohort4GiB).expect("ser"),
            "4gib"
        );
        assert_eq!(
            serde_json::to_value(HardwareCohort::NonCohort).expect("ser"),
            "none"
        );
        let de: HardwareCohort = serde_json::from_value(serde_json::json!("16gib")).expect("de");
        assert_eq!(de, HardwareCohort::Cohort16GiB);
    }

    #[test]
    fn declared_none_always_verifies_as_baseline() {
        // Even on a huge machine: "none" claims nothing, so nothing can clash.
        assert_eq!(
            verify_declared(HardwareCohort::NonCohort, Some(u64::MAX)),
            Ok(HardwareCohort::NonCohort)
        );
        // Even with no reading at all: a baseline needs no verification.
        assert_eq!(
            verify_declared(HardwareCohort::NonCohort, None),
            Ok(HardwareCohort::NonCohort)
        );
    }

    #[test]
    fn declared_cohort_verifies_only_on_a_matching_machine() {
        let eight_gib = 8 * GIB;
        assert_eq!(
            verify_declared(HardwareCohort::Cohort8GiB, Some(eight_gib)),
            Ok(HardwareCohort::Cohort8GiB)
        );
    }

    /// The core AT-21 rule: a better machine cannot substitute for a declared
    /// cohort. The 64 GiB validation machine declaring any cohort is refused.
    #[test]
    fn better_machine_cannot_substitute_for_a_declared_cohort() {
        let validation_machine = 68_448_034_816;
        for declared in [
            HardwareCohort::Cohort4GiB,
            HardwareCohort::Cohort8GiB,
            HardwareCohort::Cohort16GiB,
        ] {
            assert_eq!(
                verify_declared(declared, Some(validation_machine)),
                Err(CohortError::DeclaredCohortMismatch {
                    declared,
                    measured: HardwareCohort::NonCohort
                })
            );
        }
        // The reverse direction (a smaller machine claiming a bigger cohort)
        // is refused as well.
        assert_eq!(
            verify_declared(HardwareCohort::Cohort16GiB, Some(8 * GIB)),
            Err(CohortError::DeclaredCohortMismatch {
                declared: HardwareCohort::Cohort16GiB,
                measured: HardwareCohort::Cohort8GiB
            })
        );
    }

    #[test]
    fn declared_cohort_without_a_reading_is_unverifiable() {
        assert_eq!(
            verify_declared(HardwareCohort::Cohort4GiB, None),
            Err(CohortError::MeasuredTotalUnavailable {
                declared: HardwareCohort::Cohort4GiB
            })
        );
    }
}
