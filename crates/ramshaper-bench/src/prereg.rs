//! Preregistration verification (RS-024 Stage B; doc 08 §3–§4).
//!
//! Doc 08 §4: "Planning thresholds below must be frozen or replaced with a
//! justified workload-specific rule **before data collection**", and §3:
//! "Different workload needs must be declared before the trial, not after
//! inspecting results."
//!
//! This module makes that rule **executable**: a trial's operator freezes
//! a `Preregistration` document (JSON) — workload schedule, block count,
//! bootstrap parameters, decision thresholds — *before* running, and the
//! bench tools refuse to execute or analyze a trial that deviates from it,
//! naming every deviation. The frozen file is the evidence that the
//! thresholds were not tuned after inspecting results.
//!
//! The document is data, not authority: it grants nothing and gates
//! nothing except the bench tools' own consistency check.

use serde::Deserialize;

use crate::analysis::{BootstrapConfig, Guardrails};
use crate::workload::WorkloadSpec;

/// Preregistration document schema version.
pub const PREREG_SCHEMA: &str = "ramshaper.preregistration.v1";

/// The frozen trial plan.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preregistration {
    /// Schema version; must equal [`PREREG_SCHEMA`].
    pub schema: String,
    /// When the plan was frozen (UTC, informational — ordering with the
    /// data collection is an operator/harness responsibility).
    pub declared_at_utc: String,
    /// The declared workload need (doc 08 §3: declared before the trial).
    /// Free text; never empty.
    pub workload_declaration: String,
    /// The frozen workload schedule.
    pub workload: FrozenWorkload,
    /// The frozen bootstrap parameters (resamples + seed).
    pub bootstrap: FrozenBootstrap,
    /// The frozen decision thresholds.
    pub thresholds: FrozenThresholds,
}

/// Frozen workload schedule.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenWorkload {
    /// Measured events per run.
    pub events_per_run: usize,
    /// Warmup events (excluded by design).
    pub warmup_events: usize,
    /// Per-task compute effort.
    pub task_iters: u64,
    /// Paired block count.
    pub pilot_blocks: usize,
    /// The block-order randomization seed (frozen so the AB/BA sequence
    /// cannot be re-rolled after peeking at results).
    pub order_seed: u64,
}

/// Frozen bootstrap parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenBootstrap {
    /// Resample count.
    pub resamples: usize,
    /// Recorded seed.
    pub seed: u64,
}

/// Frozen decision thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenThresholds {
    /// Primary benefit threshold, percent.
    pub primary_benefit_pct: f64,
    /// Foreground completion guardrail, percent.
    pub foreground_max_slowdown_pct: f64,
    /// Background completion guardrail, percent.
    pub background_max_slowdown_pct: f64,
    /// Confirmatory minimum block count (below → exploratory).
    pub min_blocks_confirmatory: usize,
}

impl Preregistration {
    /// Parse and validate a frozen document. Structural rules (the
    /// workload minimum, positive thresholds, nonempty declaration) are
    /// checked here — a malformed plan is refused before any trial.
    pub fn parse(text: &str) -> Result<Self, String> {
        let document: Self = serde_json::from_str(text)
            .map_err(|error| format!("invalid preregistration: {error}"))?;
        document.validate()?;
        Ok(document)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != PREREG_SCHEMA {
            return Err(format!(
                "preregistration schema '{}' is not {PREREG_SCHEMA}",
                self.schema
            ));
        }
        if self.workload_declaration.trim().is_empty() {
            return Err(
                "workload_declaration must state the workload need before the trial \
                 (doc 08 §3)"
                    .to_string(),
            );
        }
        if self.workload.events_per_run < 100 {
            return Err(format!(
                "events_per_run {} is below the descriptive-p95 minimum of 100",
                self.workload.events_per_run
            ));
        }
        if self.workload.pilot_blocks == 0 {
            return Err("pilot_blocks must be at least 1".to_string());
        }
        if self.thresholds.primary_benefit_pct <= 0.0 {
            return Err("primary_benefit_pct must be positive".to_string());
        }
        if self.thresholds.foreground_max_slowdown_pct < 0.0
            || self.thresholds.background_max_slowdown_pct < 0.0
        {
            return Err("guardrail bounds must be nonnegative".to_string());
        }
        if self.bootstrap.resamples == 0 {
            return Err("bootstrap resamples must be at least 1".to_string());
        }
        Ok(())
    }
}

/// One named deviation between the frozen plan and the attempted run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deviation {
    /// What differs (dotted parameter name).
    pub parameter: &'static str,
    /// The frozen value.
    pub planned: String,
    /// The attempted value.
    pub attempted: String,
}

impl std::fmt::Display for Deviation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: planned {}, attempted {}",
            self.parameter, self.planned, self.attempted
        )
    }
}

/// Verify the workload spec + block count against the plan.
///
/// # Errors
/// One [`Deviation`] per mismatched parameter.
pub fn verify_workload(
    prereg: &Preregistration,
    spec: &WorkloadSpec,
    pilot_blocks: usize,
) -> Result<(), Vec<Deviation>> {
    let mut deviations = Vec::new();
    let frozen = &prereg.workload;
    if spec.events_per_run != frozen.events_per_run {
        deviations.push(Deviation {
            parameter: "workload.events_per_run",
            planned: frozen.events_per_run.to_string(),
            attempted: spec.events_per_run.to_string(),
        });
    }
    if spec.warmup_events != frozen.warmup_events {
        deviations.push(Deviation {
            parameter: "workload.warmup_events",
            planned: frozen.warmup_events.to_string(),
            attempted: spec.warmup_events.to_string(),
        });
    }
    if spec.task_iters != frozen.task_iters {
        deviations.push(Deviation {
            parameter: "workload.task_iters",
            planned: frozen.task_iters.to_string(),
            attempted: spec.task_iters.to_string(),
        });
    }
    if pilot_blocks != frozen.pilot_blocks {
        deviations.push(Deviation {
            parameter: "workload.pilot_blocks",
            planned: frozen.pilot_blocks.to_string(),
            attempted: pilot_blocks.to_string(),
        });
    }
    if spec.seed != frozen.order_seed {
        deviations.push(Deviation {
            parameter: "workload.order_seed",
            planned: frozen.order_seed.to_string(),
            attempted: spec.seed.to_string(),
        });
    }
    if deviations.is_empty() {
        Ok(())
    } else {
        Err(deviations)
    }
}

/// Verify the analysis configuration against the plan.
///
/// # Errors
/// One [`Deviation`] per mismatched parameter.
pub fn verify_analysis(
    prereg: &Preregistration,
    config: &BootstrapConfig,
    guardrails: &Guardrails,
    min_blocks_confirmatory: usize,
) -> Result<(), Vec<Deviation>> {
    let mut deviations = Vec::new();
    if config.resamples != prereg.bootstrap.resamples {
        deviations.push(Deviation {
            parameter: "bootstrap.resamples",
            planned: prereg.bootstrap.resamples.to_string(),
            attempted: config.resamples.to_string(),
        });
    }
    if config.seed != prereg.bootstrap.seed {
        deviations.push(Deviation {
            parameter: "bootstrap.seed",
            planned: prereg.bootstrap.seed.to_string(),
            attempted: config.seed.to_string(),
        });
    }
    if (guardrails.foreground_max_slowdown_pct - prereg.thresholds.foreground_max_slowdown_pct)
        .abs()
        > f64::EPSILON
    {
        deviations.push(Deviation {
            parameter: "thresholds.foreground_max_slowdown_pct",
            planned: format!("{}", prereg.thresholds.foreground_max_slowdown_pct),
            attempted: format!("{}", guardrails.foreground_max_slowdown_pct),
        });
    }
    if (guardrails.background_max_slowdown_pct - prereg.thresholds.background_max_slowdown_pct)
        .abs()
        > f64::EPSILON
    {
        deviations.push(Deviation {
            parameter: "thresholds.background_max_slowdown_pct",
            planned: format!("{}", prereg.thresholds.background_max_slowdown_pct),
            attempted: format!("{}", guardrails.background_max_slowdown_pct),
        });
    }
    if min_blocks_confirmatory != prereg.thresholds.min_blocks_confirmatory {
        deviations.push(Deviation {
            parameter: "thresholds.min_blocks_confirmatory",
            planned: prereg.thresholds.min_blocks_confirmatory.to_string(),
            attempted: min_blocks_confirmatory.to_string(),
        });
    }
    if deviations.is_empty() {
        Ok(())
    } else {
        Err(deviations)
    }
}

/// The preregistered primary benefit threshold, for callers that need the
/// frozen value (the analysis module's constant is the *default*; a
/// preregistration may freeze a justified workload-specific rule).
#[must_use]
pub fn frozen_primary_benefit(prereg: &Preregistration) -> f64 {
    prereg.thresholds.primary_benefit_pct
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::MIN_BLOCKS_FOR_CONFIRMATORY;

    const SAMPLE: &str = r#"{
        "schema": "ramshaper.preregistration.v1",
        "declared_at_utc": "2026-10-26T12:00:00Z",
        "workload_declaration": "Background file-indexer simulation; 100 x ~1ms tasks per run; primary endpoint p95 task latency",
        "workload": {
            "events_per_run": 100,
            "warmup_events": 10,
            "task_iters": 200000,
            "pilot_blocks": 10,
            "order_seed": 4242
        },
        "bootstrap": { "resamples": 10000, "seed": 4242 },
        "thresholds": {
            "primary_benefit_pct": 5.0,
            "foreground_max_slowdown_pct": 2.0,
            "background_max_slowdown_pct": 10.0,
            "min_blocks_confirmatory": 5
        }
    }"#;

    #[test]
    fn sample_document_parses_and_validates() {
        let prereg = Preregistration::parse(SAMPLE).expect("valid");
        assert_eq!(prereg.schema, PREREG_SCHEMA);
        assert_eq!(prereg.workload.pilot_blocks, 10);
        assert_eq!(prereg.bootstrap.seed, 4242);
        assert!(verify_workload(
            &prereg,
            &WorkloadSpec {
                events_per_run: 100,
                warmup_events: 10,
                task_iters: 200_000,
                seed: 4242,
            },
            10,
        )
        .is_ok());
    }

    #[test]
    fn unknown_fields_and_bad_schemas_are_refused() {
        assert!(Preregistration::parse("{ not json }").is_err());
        let swapped = SAMPLE.replace("ramshaper.preregistration.v1", "ramshaper.prereg.v9");
        let error = Preregistration::parse(&swapped).expect_err("must fail");
        assert!(
            error.contains("not ramshaper.preregistration.v1"),
            "{error}"
        );
        // An unknown field is refused (closed schema — a plan cannot sneak
        // unstated parameters in).
        let extra = SAMPLE.replace(
            "\"declared_at_utc\"",
            "\"predeclared_extra\": 1,\n        \"declared_at_utc\"",
        );
        assert!(Preregistration::parse(&extra).is_err());
    }

    #[test]
    fn structural_rules_are_checked_at_parse() {
        // Below the event minimum.
        let few = SAMPLE.replace("\"events_per_run\": 100", "\"events_per_run\": 50");
        assert!(Preregistration::parse(&few)
            .expect_err("must fail")
            .contains("descriptive-p95 minimum"));
        // Empty declaration.
        let empty = SAMPLE.replace(
            "Background file-indexer simulation; 100 x ~1ms tasks per run; primary endpoint p95 task latency",
            "   ",
        );
        assert!(Preregistration::parse(&empty)
            .expect_err("must fail")
            .contains("workload_declaration"));
        // Nonpositive primary threshold.
        let zero = SAMPLE.replace(
            "\"primary_benefit_pct\": 5.0",
            "\"primary_benefit_pct\": 0.0",
        );
        assert!(Preregistration::parse(&zero).is_err());
    }

    #[test]
    fn every_deviation_is_named_not_just_the_first() {
        let prereg = Preregistration::parse(SAMPLE).expect("valid");
        let spec = WorkloadSpec {
            events_per_run: 200,
            warmup_events: 20,
            task_iters: 100_000,
            seed: 1,
        };
        let deviations = verify_workload(&prereg, &spec, 5).expect_err("must fail");
        assert_eq!(deviations.len(), 5, "all five workload deviations named");
        let parameters: Vec<&str> = deviations.iter().map(|d| d.parameter).collect();
        assert!(parameters.contains(&"workload.events_per_run"));
        assert!(parameters.contains(&"workload.warmup_events"));
        assert!(parameters.contains(&"workload.task_iters"));
        assert!(parameters.contains(&"workload.pilot_blocks"));
        assert!(parameters.contains(&"workload.order_seed"));
    }

    #[test]
    fn analysis_deviations_include_seed_and_thresholds() {
        let prereg = Preregistration::parse(SAMPLE).expect("valid");
        let config = BootstrapConfig {
            resamples: 5_000,
            seed: 1,
        };
        let deviations = verify_analysis(
            &prereg,
            &config,
            &Guardrails::default(),
            MIN_BLOCKS_FOR_CONFIRMATORY,
        )
        .expect_err("must fail");
        let parameters: Vec<&str> = deviations.iter().map(|d| d.parameter).collect();
        assert!(parameters.contains(&"bootstrap.resamples"));
        assert!(parameters.contains(&"bootstrap.seed"));
        // Default guardrails and the default confirmatory minimum (5)
        // match the sample's frozen values: neither may be named.
        assert!(!parameters.contains(&"thresholds.foreground_max_slowdown_pct"));
        assert!(!parameters.contains(&"thresholds.min_blocks_confirmatory"));
    }

    #[test]
    fn matching_plan_verifies_clean() {
        let prereg = Preregistration::parse(SAMPLE).expect("valid");
        let spec = WorkloadSpec {
            events_per_run: 100,
            warmup_events: 10,
            task_iters: 200_000,
            seed: 4242,
        };
        assert!(verify_workload(&prereg, &spec, 10).is_ok());
        let config = BootstrapConfig {
            resamples: 10_000,
            seed: 4242,
        };
        assert!(verify_analysis(&prereg, &config, &Guardrails::default(), 5).is_ok());
    }
}
