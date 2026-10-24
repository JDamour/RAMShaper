//! `pressure_v1` — the observation classifier (RS-009, contract 14 §4).
//!
//! This is an *observation* classifier, not an intervention trigger and not
//! a diagnostic claim about any slowdown. States:
//!
//! - `Unknown`: global physical/commit values or low-memory state missing,
//!   stale or invalid; also the state after any missing observation.
//! - `Elevated`: low-memory condition signaled in three consecutive valid
//!   observations spanning at least 2 s (first to last).
//! - `Normal`: five consecutive valid observations spanning at least 4 s
//!   with no low-memory condition.
//!
//! The previous valid state is retained while a transition is pending; the
//! pending evidence (consecutive counts, window) is exposed. `Critical` is
//! reserved and never emitted by v1. Deterministic: replaying the same
//! observation sequence produces byte-identical outputs (AT-07).

use serde::Serialize;

/// Rule version marker emitted with every observation.
pub const PRESSURE_RULE_VERSION: &str = "pressure_v1";

/// Classified pressure level. `Critical` deliberately does not exist here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureLevel {
    /// Inputs missing/stale/invalid, or not enough consecutive evidence yet.
    Unknown,
    /// Low-memory condition sustained per the v1 rule.
    Elevated,
    /// Sustained absence of the low-memory condition per the v1 rule.
    Normal,
}

impl PressureLevel {
    /// Stable machine-readable name (used in JSON and text output).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PressureLevel::Unknown => "unknown",
            PressureLevel::Elevated => "elevated",
            PressureLevel::Normal => "normal",
        }
    }

    /// Human-facing sentence per contract 14 §4.
    #[must_use]
    pub fn display_text(self) -> &'static str {
        match self {
            PressureLevel::Unknown => "Insufficient valid observations to classify memory pressure",
            PressureLevel::Elevated => "Low-memory signal observed (OS resource condition)",
            // Contract wording: the UI says "No low-memory signal observed"
            // and explicitly does not establish absence of CPU/I/O contention.
            PressureLevel::Normal => "No low-memory signal observed",
        }
    }
}

/// Evidence quality of the inputs used for one observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceQuality {
    /// All required inputs present and valid.
    Complete,
    /// Some inputs missing (listed in `missing_inputs`).
    Partial,
    /// No usable inputs.
    Unavailable,
}

/// Required input names for evidence reporting.
pub const PRESSURE_INPUTS: [&str; 5] = [
    "physical_total",
    "physical_available",
    "commit_used",
    "commit_limit",
    "low_memory_state",
];

/// One observation fed to the classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PressureInput {
    /// Whether the OS low-memory condition is signaled (valid input only).
    pub low_memory_signaled: bool,
    /// Monotonic timestamp of this observation (ms, current agent instance).
    pub monotonic_ms: u64,
}

/// Classifier output for one observation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PressureObservation {
    /// Rule version (`pressure_v1`).
    pub rule_version: &'static str,
    /// Classified level after this observation.
    pub level: PressureLevel,
    /// Evidence quality of this observation's inputs.
    pub evidence: EvidenceQuality,
    /// Names of missing inputs (empty when complete).
    pub missing_inputs: Vec<&'static str>,
    /// Pending evidence: consecutive observations with the signal.
    pub consecutive_signal: u32,
    /// Pending evidence: consecutive observations without the signal.
    pub consecutive_clear: u32,
    /// Span of the current consecutive run (ms, first to last observation).
    pub run_window_ms: u64,
}

/// Deterministic state machine. Create once per agent instance and feed one
/// observation per sampled snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct PressureClassifier {
    level: PressureLevel,
    consecutive_signal: u32,
    consecutive_clear: u32,
    run_first_monotonic_ms: Option<u64>,
    last_monotonic_ms: Option<u64>,
}

impl PressureClassifier {
    /// New classifier starting from `Unknown`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            level: PressureLevel::Unknown,
            consecutive_signal: 0,
            consecutive_clear: 0,
            run_first_monotonic_ms: None,
            last_monotonic_ms: None,
        }
    }

    /// Feed one observation. `Some(input)` when every required input of the
    /// snapshot was present and valid; `None` marks a missing/invalid
    /// observation (which resets consecutive counts and yields `Unknown`).
    pub fn observe(&mut self, input: Option<PressureInput>) -> PressureObservation {
        let Some(input) = input else {
            // Missing observation: reset counts; state becomes Unknown.
            self.level = PressureLevel::Unknown;
            self.consecutive_signal = 0;
            self.consecutive_clear = 0;
            self.run_first_monotonic_ms = None;
            self.last_monotonic_ms = None;
            return PressureObservation {
                rule_version: PRESSURE_RULE_VERSION,
                level: self.level,
                evidence: EvidenceQuality::Unavailable,
                missing_inputs: PRESSURE_INPUTS.to_vec(),
                consecutive_signal: 0,
                consecutive_clear: 0,
                run_window_ms: 0,
            };
        };

        let now = input.monotonic_ms;

        if input.low_memory_signaled {
            self.consecutive_signal += 1;
            self.consecutive_clear = 0;
        } else {
            self.consecutive_clear += 1;
            self.consecutive_signal = 0;
        }
        // A new run (either direction) restarts the window anchor.
        if self.consecutive_signal + self.consecutive_clear == 1 {
            self.run_first_monotonic_ms = Some(now);
        }

        // Transitions (contract 14 §4). The previous valid level is retained
        // until a new rule qualifies.
        if self.consecutive_signal >= 3 && self.span_ms(now) >= 2000 {
            self.level = PressureLevel::Elevated;
        } else if self.consecutive_clear >= 5 && self.span_ms(now) >= 4000 {
            self.level = PressureLevel::Normal;
        }

        self.last_monotonic_ms = Some(now);
        PressureObservation {
            rule_version: PRESSURE_RULE_VERSION,
            level: self.level,
            evidence: EvidenceQuality::Complete,
            missing_inputs: Vec::new(),
            consecutive_signal: self.consecutive_signal,
            consecutive_clear: self.consecutive_clear,
            run_window_ms: self.span_ms(now),
        }
    }

    fn span_ms(&self, now: u64) -> u64 {
        self.run_first_monotonic_ms
            .map_or(0, |first| now.saturating_sub(first))
    }
}

impl Default for PressureClassifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Derive evidence quality from a snapshot's metric statuses (which inputs
/// are missing). Used by callers to decide `Some`/`None` for `observe` and
/// to report quality when inputs are partial.
#[must_use]
pub fn missing_inputs(
    physical_total_ok: bool,
    physical_available_ok: bool,
    commit_used_ok: bool,
    commit_limit_ok: bool,
    low_memory_ok: bool,
) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !physical_total_ok {
        missing.push(PRESSURE_INPUTS[0]);
    }
    if !physical_available_ok {
        missing.push(PRESSURE_INPUTS[1]);
    }
    if !commit_used_ok {
        missing.push(PRESSURE_INPUTS[2]);
    }
    if !commit_limit_ok {
        missing.push(PRESSURE_INPUTS[3]);
    }
    if !low_memory_ok {
        missing.push(PRESSURE_INPUTS[4]);
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNAL: PressureInput = PressureInput {
        low_memory_signaled: true,
        monotonic_ms: 0,
    };
    const CLEAR: PressureInput = PressureInput {
        low_memory_signaled: false,
        monotonic_ms: 0,
    };

    fn at(ms: u64, signaled: bool) -> PressureInput {
        PressureInput {
            low_memory_signaled: signaled,
            monotonic_ms: ms,
        }
    }

    #[test]
    fn elevated_requires_three_observations_spanning_two_seconds() {
        let mut c = PressureClassifier::new();
        // Three signals inside 1 s: count reaches 3 but the span is short.
        let o1 = c.observe(Some(at(0, true)));
        assert_eq!(o1.level, PressureLevel::Unknown);
        let o2 = c.observe(Some(at(400, true)));
        assert_eq!(o2.level, PressureLevel::Unknown); // pending, retains Unknown
        assert_eq!(o2.consecutive_signal, 2);
        let o3 = c.observe(Some(at(900, true)));
        assert_eq!(o3.level, PressureLevel::Unknown, "span 900 ms < 2 s");
        // Fourth signal at 2.1 s: >= 3 consecutive and span >= 2 s.
        let o4 = c.observe(Some(at(2100, true)));
        assert_eq!(o4.level, PressureLevel::Elevated);
        assert_eq!(o4.run_window_ms, 2100);
    }

    #[test]
    fn normal_requires_five_observations_spanning_four_seconds() {
        let mut c = PressureClassifier::new();
        // Four clears at 0..3000 ms.
        for ms in [0, 1000, 2000, 3000] {
            let _ = c.observe(Some(at(ms, false)));
        }
        // Fifth clear, but the run spans only 3 s: still Unknown.
        assert_eq!(
            c.observe(Some(at(3000, false))).level,
            PressureLevel::Unknown
        );
        // Sixth clear at 4.1 s: count >= 5 and span >= 4 s -> Normal.
        let o6 = c.observe(Some(at(4100, false)));
        assert_eq!(o6.level, PressureLevel::Normal);
        assert_eq!(o6.consecutive_clear, 6);
    }

    #[test]
    fn previous_valid_state_is_retained_while_transition_pending() {
        let mut c = PressureClassifier::new();
        // Establish Elevated with signals at 0, 1100, 2200, 2300.
        for ms in [0, 1100, 2200] {
            let _ = c.observe(Some(at(ms, true)));
        }
        assert_eq!(
            c.observe(Some(at(2300, true))).level,
            PressureLevel::Elevated
        );
        // Clears at 3300, 4300, 5400, 6500 (4 clears, span 3200 ms): still
        // Elevated — the Normal rule needs 5 clears spanning >= 4 s.
        assert_eq!(
            c.observe(Some(at(3300, false))).level,
            PressureLevel::Elevated
        );
        assert_eq!(
            c.observe(Some(at(4300, false))).level,
            PressureLevel::Elevated
        );
        assert_eq!(
            c.observe(Some(at(5400, false))).level,
            PressureLevel::Elevated
        );
        assert_eq!(
            c.observe(Some(at(6500, false))).level,
            PressureLevel::Elevated
        );
        // Fifth clear at 7600: span 7600-3300 = 4300 ms -> Normal.
        assert_eq!(
            c.observe(Some(at(7600, false))).level,
            PressureLevel::Normal
        );
    }

    #[test]
    fn missing_observation_resets_to_unknown() {
        let mut c = PressureClassifier::new();
        for ms in [0, 1100, 2200] {
            let _ = c.observe(Some(at(ms, true)));
        }
        assert_eq!(
            c.observe(Some(at(2200, true))).level,
            PressureLevel::Elevated
        );
        let after_gap = c.observe(None);
        assert_eq!(after_gap.level, PressureLevel::Unknown);
        assert_eq!(after_gap.consecutive_signal, 0);
        assert_eq!(after_gap.evidence, EvidenceQuality::Unavailable);
        assert_eq!(after_gap.missing_inputs.len(), 5);
    }

    #[test]
    fn mixed_signal_breaks_runs_and_resets_windows() {
        let mut c = PressureClassifier::new();
        let _ = c.observe(Some(at(0, true)));
        let _ = c.observe(Some(at(1000, true)));
        let mixed = c.observe(Some(at(2000, false)));
        assert_eq!(mixed.consecutive_signal, 0);
        assert_eq!(mixed.consecutive_clear, 1);
        assert_eq!(mixed.run_window_ms, 0);
    }

    #[test]
    fn critical_is_never_emitted_and_replays_are_identical() {
        // Drive a long mixed sequence; v1 must never produce a Critical
        // variant (it cannot exist) and replaying twice matches exactly.
        let sequence: Vec<Option<PressureInput>> = (0..30)
            .map(|i| match i % 7 {
                3 => None, // periodic missing observation
                _ => Some(at(i as u64 * 1500, i % 5 == 0)),
            })
            .collect();
        let run = |seq: &[Option<PressureInput>]| -> Vec<PressureObservation> {
            let mut c = PressureClassifier::new();
            seq.iter().map(|o| c.observe(*o)).collect()
        };
        let first = run(&sequence);
        let second = run(&sequence);
        assert_eq!(first, second);
        assert!(first.iter().all(|o| o.rule_version == "pressure_v1"));
    }

    #[test]
    fn display_text_never_claims_contention_absence_beyond_low_memory() {
        assert!(PressureLevel::Normal
            .display_text()
            .contains("No low-memory signal observed"));
        assert!(PressureLevel::Elevated
            .display_text()
            .contains("resource condition"));
    }

    #[test]
    fn missing_inputs_lists_exact_names() {
        assert!(missing_inputs(true, true, true, true, true).is_empty());
        assert_eq!(
            missing_inputs(false, true, false, true, false),
            vec!["physical_total", "commit_used", "low_memory_state"]
        );
    }

    #[test]
    fn signal_and_clear_constants_are_distinct() {
        assert_ne!(SIGNAL, CLEAR);
    }
}
