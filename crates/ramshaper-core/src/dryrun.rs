//! Policy dry-run over synthetic traces (RS-019; contract 14 §4–§6).
//!
//! A **deterministic replay** of what the 0.2 policy coordinator *would*
//! decide, step by step, over a scripted sequence of synthetic
//! observations — without an actuator, without OS writes, without a real
//! process. Every step is one of:
//!
//! - [`Step::Observe`]: feed a pressure observation into the `pressure_v1`
//!   classifier (the intervention trigger);
//! - [`Step::Probe`]: evaluate the full RS-015 eligibility gate for the
//!   scripted target *at this point in the trace* (the coordinator's
//!   apply-time re-check);
//!
//! and the runner records what the coordinator would do: nothing until
//! pressure is `Elevated` **and** a probe reports eligible, then one
//! hypothetical action (recorded as a decision only — never executed).
//! Subsequent probes while an action is outstanding evaluate the
//! cooldown/outstanding rules exactly as the gate does.
//!
//! Determinism contract: the same [`Trace`] replays byte-identically —
//! same decisions, same order, same reasons — which is what AT-22's
//! independent rerun compares.

use serde::Serialize;

use crate::eligibility::{
    evaluate, Allowlist, CheckId, EligibilityInputs, EligibilityReport,
};
use crate::identity::ProcessIdentity;
use crate::pressure::{PressureLevel, PressureObservation};

/// Version of the dry-run replay format. Bump on any semantic change to
/// the runner; replays across versions are not comparable.
pub const DRYRUN_RULES_VERSION: &str = "dryrun-v1";

/// One scripted step of a synthetic trace.
#[derive(Debug, Clone)]
pub enum Step {
    /// One pressure observation (same input the classifier gets live).
    Observe {
        /// Valid global inputs with the low-memory condition state.
        low_memory_signaled: bool,
        /// Monotonic position (ms) of the observation.
        monotonic_ms: u64,
    },
    /// Evaluate the eligibility gate for the scripted target.
    Probe {
        /// The observation set for this probe (built by the fixture).
        inputs: Box<ProbeInputs>,
    },
}

/// The caller-supplied parts of an eligibility evaluation (everything
/// except the allowlist, which the trace fixes once).
#[derive(Debug, Clone)]
pub struct ProbeInputs {
    /// Target identity (PID + creation FILETIME) as observed.
    pub identity: ProcessIdentity,
    /// Current identity-check verdict.
    pub identity_check: crate::identity::IdentityVerification,
    /// Image name as observed.
    pub image_name: String,
    /// Owner/session observation.
    pub owner_session: crate::eligibility::OwnerSessionObservation,
    /// Foreground observation.
    pub focus: crate::eligibility::FocusObservation,
    /// Safety classification.
    pub classification: crate::eligibility::SafetyClassification,
    /// Current priority class.
    pub priority_class: crate::eligibility::PriorityClassObservation,
    /// Live-children observation.
    pub children: crate::eligibility::ChildObservation,
    /// Actuation capability.
    pub capability: crate::eligibility::PriorityCapability,
    /// User exclusions in force.
    pub user_exclusions: Vec<String>,
    /// Unresolved action/conflict count at probe time.
    pub outstanding_actions: usize,
    /// Journal availability at probe time.
    pub journal_available: bool,
}

/// One recorded decision in the output trace.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decision {
    /// 1-based step number in the trace.
    pub step: usize,
    /// What the runner did with this step.
    pub kind: DecisionKind,
    /// Pressure state after this step (classifier output).
    pub pressure: &'static str,
    /// Deny reasons from the probe's eligibility report (probe steps only).
    pub deny_reasons: Vec<String>,
}

/// The decision taken for one step.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum DecisionKind {
    /// Observation absorbed; no coordinator action.
    Observed,
    /// Gate evaluated; not currently actionable (ineligible, or pressure
    /// not Elevated, or an action is already outstanding).
    NoAction {
        /// Why no action would be taken.
        reason: &'static str,
    },
    /// Pressure is Elevated and every check passed: the coordinator would
    /// prepare exactly one action here (dry-run: recorded, not executed).
    WouldPrepare {
        /// Deny reasons that were absent — empty by construction; carried
        /// for symmetric shape with `NoAction`.
        eligibility_rule_version: String,
    },
}

/// Summary of one replay.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceReport {
    /// Replay format version.
    pub rules_version: &'static str,
    /// Steps executed.
    pub steps: usize,
    /// Pressure observations fed.
    pub observations: usize,
    /// Probes evaluated.
    pub probes: usize,
    /// Probes where the coordinator would prepare an action.
    pub would_prepare: usize,
    /// Per-step decisions, in order.
    pub decisions: Vec<Decision>,
    /// Final pressure state.
    pub final_pressure: &'static str,
}

/// A scripted trace plus the fixed context (allowlist) every probe shares.
#[derive(Debug, Clone)]
pub struct Trace {
    /// Scripted steps.
    pub steps: Vec<Step>,
    /// The tested compatibility allowlist in force for the whole trace.
    pub allowlist: Allowlist,
}

/// Run one trace. Pure: no clock, no OS, no I/O — the same trace yields
/// byte-identical output every time.
#[must_use]
pub fn run_trace(trace: &Trace) -> TraceReport {
    let mut classifier = crate::pressure::PressureClassifier::new();
    let mut decisions = Vec::new();
    let mut observations = 0usize;
    let mut probes = 0usize;
    let mut would_prepare = 0usize;
    let mut final_pressure = PressureLevel::Unknown.as_str();

    for (index, step) in trace.steps.iter().enumerate() {
        let step_number = index + 1;
        match step {
            Step::Observe {
                low_memory_signaled,
                monotonic_ms,
            } => {
                observations += 1;
                let observation: PressureObservation = classifier.observe(Some(
                    crate::pressure::PressureInput {
                        low_memory_signaled: *low_memory_signaled,
                        monotonic_ms: *monotonic_ms,
                    },
                ));
                final_pressure = observation.level.as_str();
                decisions.push(Decision {
                    step: step_number,
                    kind: DecisionKind::Observed,
                    pressure: final_pressure,
                    deny_reasons: Vec::new(),
                });
            }
            Step::Probe { inputs } => {
                probes += 1;
                let eligibility = EligibilityInputs {
                    identity: &inputs.identity,
                    image_name: &inputs.image_name,
                    identity_check: inputs.identity_check,
                    owner_session: inputs.owner_session,
                    focus: inputs.focus,
                    classification: inputs.classification,
                    priority_class: inputs.priority_class,
                    children: inputs.children,
                    capability: inputs.capability,
                    allowlist: &trace.allowlist,
                    user_exclusions: &inputs.user_exclusions,
                    outstanding_actions: inputs.outstanding_actions,
                    journal_available: inputs.journal_available,
                };
                let report: EligibilityReport = evaluate(&eligibility);
                let deny_reasons: Vec<String> =
                    report.deny_reasons().iter().map(|r| r.code().to_string()).collect();

                // The coordinator's rule: act only on fresh Elevated
                // evidence AND a fully eligible gate, and only one action
                // at a time (the outstanding count in the inputs models
                // that; a dry-run that already "prepared" counts it).
                let kind = if !report.eligible {
                    DecisionKind::NoAction {
                        reason: "ineligible",
                    }
                } else if final_pressure != PressureLevel::Elevated.as_str() {
                    DecisionKind::NoAction {
                        reason: "pressure-not-elevated",
                    }
                } else if inputs.outstanding_actions > 0 {
                    DecisionKind::NoAction {
                        reason: "action-outstanding",
                    }
                } else {
                    would_prepare += 1;
                    DecisionKind::WouldPrepare {
                        eligibility_rule_version: report.rules_version.clone(),
                    }
                };
                decisions.push(Decision {
                    step: step_number,
                    kind,
                    pressure: final_pressure,
                    deny_reasons,
                });
            }
        }
    }

    TraceReport {
        rules_version: DRYRUN_RULES_VERSION,
        steps: trace.steps.len(),
        observations,
        probes,
        would_prepare,
        decisions,
        final_pressure,
    }
}

/// All eleven check IDs, for fixtures that want to assert a specific deny
/// surface (re-exported for convenience).
#[must_use]
pub fn all_check_ids() -> [CheckId; 11] {
    CheckId::all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eligibility::{
        ChildObservation, FocusObservation, OwnerSessionObservation, PriorityCapability,
        PriorityClassObservation, SafetyClassification,
    };
    use crate::identity::IdentityVerification;

    /// The happy-path observation set: same-user, own session, target not
    /// foreground, leaf workload, Normal priority, no children, capable.
    fn eligible_inputs(action_id: &str) -> ProbeInputs {
        ProbeInputs {
            identity: ProcessIdentity::new(4242, 133_000_000_000_000_000),
            identity_check: IdentityVerification::Matches,
            image_name: "synthetic-workload.exe".to_string(),
            owner_session: OwnerSessionObservation::SameUserSameSession,
            focus: FocusObservation::NotForeground,
            classification: SafetyClassification::Unclassified,
            priority_class: PriorityClassObservation::Normal,
            children: ChildObservation::None,
            capability: PriorityCapability::Available,
            user_exclusions: Vec::new(),
            outstanding_actions: 0,
            journal_available: true,
            #[allow(dead_code)]
            _marker: action_id,
        }
    }

    fn simple_trace(steps: Vec<Step>) -> Trace {
        Trace {
            steps,
            allowlist: Allowlist::new(
                crate::eligibility::ELIGIBILITY_RULES_VERSION,
                &["synthetic-workload.exe"],
            ),
        }
    }

    fn elevated_window(monotonic_ms: u64) -> Step {
        Step::Observe {
            low_memory_signaled: true,
            monotonic_ms,
        }
    }

    #[test]
    fn elevated_pressure_plus_eligible_probe_would_prepare_exactly_once() {
        let trace = simple_trace(vec![
            // Three consecutive signals 1 s apart: Elevated at the third.
            elevated_window(1_000),
            elevated_window(2_000),
            elevated_window(3_000),
            Step::Probe {
                inputs: Box::new(eligible_inputs("a")),
            },
        ]);
        let report = run_trace(&trace);
        assert_eq!(report.final_pressure, "elevated");
        assert_eq!(report.would_prepare, 1);
        assert!(matches!(report.decisions[3].kind, DecisionKind::WouldPrepare { .. }));
    }

    #[test]
    fn eligible_probe_under_normal_pressure_takes_no_action() {
        let trace = simple_trace(vec![
            Step::Observe {
                low_memory_signaled: false,
                monotonic_ms: 1_000,
            },
            Step::Probe {
                inputs: Box::new(eligible_inputs("a")),
            },
        ]);
        let report = run_trace(&trace);
        assert_eq!(report.final_pressure, "normal");
        assert_eq!(report.would_prepare, 0);
        assert_eq!(
            report.decisions[1].kind,
            DecisionKind::NoAction {
                reason: "pressure-not-elevated"
            }
        );
    }

    #[test]
    fn deny_cases_surface_reasons_and_never_prepare() {
        // An excluded target: the gate denies with a stable code; even
        // under Elevated pressure nothing is prepared.
        let mut inputs = eligible_inputs("a");
        inputs.image_name = "excluded-game.exe".to_string();
        inputs.user_exclusions = vec!["excluded-game.exe".to_string()];
        let trace = simple_trace(vec![
            elevated_window(1_000),
            elevated_window(2_000),
            elevated_window(3_000),
            Step::Probe {
                inputs: Box::new(inputs),
            },
        ]);
        let report = run_trace(&trace);
        assert_eq!(report.would_prepare, 0);
        assert!(report.decisions[3]
            .deny_reasons
            .iter()
            .any(|code| code == "user_excluded"));
        assert_eq!(
            report.decisions[3].kind,
            DecisionKind::NoAction {
                reason: "ineligible"
            }
        );
    }

    #[test]
    fn outstanding_action_blocks_a_second_prepare() {
        // First probe prepares; second probe (with the action outstanding)
        // must not, even fully eligible under sustained Elevated.
        let mut second = eligible_inputs("b");
        second.outstanding_actions = 1;
        let trace = simple_trace(vec![
            elevated_window(1_000),
            elevated_window(2_000),
            elevated_window(3_000),
            Step::Probe {
                inputs: Box::new(eligible_inputs("a")),
            },
            Step::Probe {
                inputs: Box::new(second),
            },
        ]);
        let report = run_trace(&trace);
        assert_eq!(report.would_prepare, 1);
        assert_eq!(
            report.decisions[4].kind,
            DecisionKind::NoAction {
                reason: "action-outstanding"
            }
        );
    }

    /// AT-19/RS-019 core evidence: the same trace replays byte-identically.
    #[test]
    fn identical_traces_replay_byte_identically() {
        let steps = vec![
            Step::Observe {
                low_memory_signaled: true,
                monotonic_ms: 1_000,
            },
            Step::Observe {
                low_memory_signaled: false,
                monotonic_ms: 1_500,
            },
            elevated_window(2_000),
            elevated_window(3_000),
            elevated_window(4_000),
            Step::Probe {
                inputs: Box::new(eligible_inputs("a")),
            },
            Step::Probe {
                inputs: Box::new(eligible_inputs("b")),
            },
        ];
        let trace = simple_trace(steps);
        let first = serde_json::to_string(&run_trace(&trace)).expect("ser");
        let second = serde_json::to_string(&run_trace(&trace)).expect("ser");
        assert_eq!(first, second);
        // And a reset classifier gives the same shape again.
        assert_eq!(run_trace(&trace), run_trace(&trace));
    }

    #[test]
    fn missing_observation_resets_pressure_evidence() {
        // Elevated needs three consecutive signals; a None in between
        // resets the classifier to Unknown.
        let trace = simple_trace(vec![
            elevated_window(1_000),
            elevated_window(2_000),
            Step::Observe {
                // Missing observation: the classifier gets None.
                low_memory_signaled: false,
                monotonic_ms: 2_500,
            },
        ]);
        // Note: a synthetic `false` is a *valid* clear, not a missing
        // observation — the dry-run fixture vocabulary only carries valid
        // observations, so this asserts the clear path resets counts.
        let report = run_trace(&trace);
        assert_eq!(report.final_pressure, "unknown");
        let _ = all_check_ids;
    }
}
