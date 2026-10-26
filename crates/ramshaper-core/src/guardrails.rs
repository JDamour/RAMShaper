//! Policy guardrails (RS-018; contract 14 §5–§6, AT-14).
//!
//! Pure decision state the agent composes around the coordinator, restore
//! engine and journal:
//!
//! - **one-action cap**: at most one outstanding action at a time
//!   (reserve/release; the coordinator slot models the same rule for one
//!   coordinator, this book models it across the agent);
//! - **cooldown**: 60 s per *identity* after a terminal record, keyed so
//!   one target's cooldown never affects another;
//! - **target disable**: a failed apply, verification or restore disables
//!   *that target's* policy until an explicit user retry
//!   ([`TargetBook::re_enable`]) — never a timeout;
//! - **safety trips**: a pure evaluation of the safety observations
//!   (child spawned, foreground return, session lock, stale focus) that
//!   maps to the RS-017 restore triggers, records the incompatibility
//!   finding (AT-14: "incompatibility is recorded") and forbids any
//!   explicit write to an unapproved descendant by construction.
//!
//! Nothing here touches the OS; the agent performs whatever a decision
//! calls for.

use serde::Serialize;

use crate::identity::ProcessIdentity;
use crate::journal::ActionState;

/// Cooldown imposed after a terminal record (contract 14 §5: "After
/// completion, impose a 60 s per-identity cooldown and require a new
/// confirmation").
pub const COOLDOWN_MS: u64 = 60_000;

/// Per-identity cooldown and disable book.
#[derive(Debug, Default)]
pub struct TargetBook {
    /// Identity key (`pid:filetime`) → cooldown-until (Unix ms).
    cooldown_until: std::collections::HashMap<String, u64>,
    /// Identity key → why the target's policy is disabled.
    disabled: std::collections::HashMap<String, String>,
}

fn key(identity: &ProcessIdentity) -> String {
    format!("{}:{}", identity.pid, identity.creation_filetime_100ns)
}

impl TargetBook {
    /// Record a terminal state for the identity: starts its 60 s cooldown.
    /// `Resolved`/`TargetGone` start the cooldown; pinned records do not
    /// (their target still needs attention, not another experiment).
    pub fn after_terminal(
        &mut self,
        identity: &ProcessIdentity,
        state: ActionState,
        now_unix_ms: u64,
    ) {
        if matches!(state, ActionState::Restored | ActionState::TargetGone) {
            self.cooldown_until
                .insert(key(identity), now_unix_ms.saturating_add(COOLDOWN_MS));
        }
    }

    /// Whether the identity may start a new experiment now (cooldown
    /// expired or never started, and the target is not disabled).
    #[must_use]
    pub fn admissible(&self, identity: &ProcessIdentity, now_unix_ms: u64) -> bool {
        if self.disabled.contains_key(&key(identity)) {
            return false;
        }
        self.cooldown_until
            .get(&key(identity))
            .is_none_or(|until| *until <= now_unix_ms)
    }

    /// When the identity's cooldown ends, if one is running.
    #[must_use]
    pub fn cooldown_until(&self, identity: &ProcessIdentity) -> Option<u64> {
        self.cooldown_until.get(&key(identity)).copied()
    }

    /// Disable the identity's policy (failed apply, verification or
    /// restore). Only [`TargetBook::re_enable`] lifts it — never time.
    pub fn disable(&mut self, identity: &ProcessIdentity, reason: &str) {
        self.disabled.insert(key(identity), reason.to_string());
    }

    /// Explicit user retry: re-enable the identity's policy.
    pub fn re_enable(&mut self, identity: &ProcessIdentity) {
        self.disabled.remove(&key(identity));
    }

    /// Why the identity is disabled, if it is.
    #[must_use]
    pub fn disabled_reason(&self, identity: &ProcessIdentity) -> Option<&str> {
        self.disabled.get(&key(identity)).map(String::as_str)
    }
}

/// The one-action cap across the agent (contract 14 §6: the policy
/// coordinator is the only writer; one action changes one property —
/// there is never more than one outstanding action).
#[derive(Debug, Default)]
pub struct OneActionCap {
    outstanding: Option<String>,
}

impl OneActionCap {
    /// Reserve the single slot for `action_id`. Fails when another action
    /// is outstanding.
    pub fn reserve(&mut self, action_id: &str) -> Result<(), SlotTaken> {
        match &self.outstanding {
            Some(current) if current != action_id => Err(SlotTaken {
                held_by: current.clone(),
            }),
            // Reserving with the same ID is idempotent (repeat
            // confirmations hold, AT-14).
            _ => {
                self.outstanding = Some(action_id.to_string());
                Ok(())
            }
        }
    }

    /// Release the slot (action reached a terminal state).
    pub fn release(&mut self, action_id: &str) {
        if self.outstanding.as_deref() == Some(action_id) {
            self.outstanding = None;
        }
    }

    /// Who holds the slot, if anyone.
    #[must_use]
    pub fn holder(&self) -> Option<&str> {
        self.outstanding.as_deref()
    }
}

/// The slot is held by another action.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the one-action slot is held by '{held_by}'")]
pub struct SlotTaken {
    /// The action currently holding the slot.
    pub held_by: String,
}

/// A safety observation the guardrails evaluate (AT-14's noisy events and
/// child-spawning targets).
#[derive(Debug, Clone)]
pub enum SafetyObservation {
    /// A live direct child of the target was observed (R-10: an observed
    /// new child invalidates the experiment and restores the parent).
    ChildSpawned {
        /// The consented parent.
        parent: ProcessIdentity,
        /// The observed child.
        child: ProcessIdentity,
        /// The child's image name (label for the finding).
        child_image: String,
    },
    /// The target returned to the foreground.
    ForegroundReturned,
    /// The session was locked.
    SessionLocked,
    /// The focus/foreground observation is stale beyond the apply
    /// freshness window.
    FocusStale {
        /// Age of the last valid observation, ms.
        age_ms: u64,
    },
    /// The system slept and has resumed (detected as a jump of the
    /// biased−unbiased interrupt-time difference). The experiment is
    /// invalid across a sleep: the user walked away, wall-clock TTLs
    /// aged, and the pre-sleep observations no longer describe the
    /// machine.
    SystemResumed {
        /// How much sleep the detection attributes to the gap, ms.
        sleep_ms: u64,
    },
}

/// What the guardrails decided.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Decision {
    /// Nothing observed that trips a guardrail.
    Continue,
    /// Initiate restoration through the RS-017 engine with the mapped
    /// trigger. A child observation additionally records the
    /// incompatibility finding and carries the no-descendant-write rule.
    Trip {
        /// The mapped RS-017 restore trigger.
        trigger: TripTrigger,
        /// Set only for child observations (AT-14): the parent restores
        /// and the unapproved descendant must receive no explicit write.
        finding: Option<CompatibilityFinding>,
    },
}

/// The RS-017 trigger a safety observation maps to. Mirrors
/// `crate::journal`'s restore-trigger vocabulary at the guardrail
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TripTrigger {
    /// Observed child invalidated the experiment.
    ChildSpawned,
    /// Foreground return.
    ForegroundReturned,
    /// Session lock.
    SessionLock,
    /// Stale safety input.
    StaleSafetyInput,
    /// The system resumed from sleep; the contract's restore reason is
    /// recorded as `Suspend`.
    SystemResumed,
}

impl TripTrigger {
    /// Stable machine code (evidence trail).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::ChildSpawned => "child_spawned",
            Self::ForegroundReturned => "foreground_returned",
            Self::SessionLock => "session_lock",
            Self::StaleSafetyInput => "stale_safety_input",
            Self::SystemResumed => "system_resumed",
        }
    }
}

/// A recorded incompatibility (AT-14: "incompatibility is recorded").
/// Findings feed the tested-compatibility allowlist process (RS-024
/// Stage B) — a finding is evidence, not a verdict about the app.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompatibilityFinding {
    /// Parent image name (label).
    pub parent_image: String,
    /// Observed child image name (label).
    pub child_image: String,
    /// Child identity (pid + creation FILETIME) at observation time.
    pub child_pid: u32,
    pub child_creation_filetime: u64,
    /// When the finding was recorded (Unix ms).
    pub recorded_at_unix_ms: u64,
}

/// Evaluate one safety observation. Pure.
#[must_use]
pub fn evaluate(observation: &SafetyObservation, now_unix_ms: u64) -> Decision {
    match observation {
        SafetyObservation::ChildSpawned {
            parent: _,
            child,
            child_image,
        } => Decision::Trip {
            trigger: TripTrigger::ChildSpawned,
            finding: Some(CompatibilityFinding {
                parent_image: String::new(), // the agent fills the label
                child_image: child_image.clone(),
                child_pid: child.pid,
                child_creation_filetime: child.creation_filetime_100ns,
                recorded_at_unix_ms: now_unix_ms,
            }),
        },
        SafetyObservation::ForegroundReturned => Decision::Trip {
            trigger: TripTrigger::ForegroundReturned,
            finding: None,
        },
        SafetyObservation::SessionLocked => Decision::Trip {
            trigger: TripTrigger::SessionLock,
            finding: None,
        },
        SafetyObservation::FocusStale { age_ms: _ } => Decision::Trip {
            trigger: TripTrigger::StaleSafetyInput,
            finding: None,
        },
        SafetyObservation::SystemResumed { sleep_ms: _ } => Decision::Trip {
            trigger: TripTrigger::SystemResumed,
            finding: None,
        },
    }
}

/// Detects system sleep as a *change* of the biased−unbiased
/// interrupt-time difference (windows adapter `interrupt_time_pair`):
/// the difference equals accumulated sleep since boot, so growth between
/// two observations means the system slept in between. Pure and testable;
/// the ticker holds one detector for the agent's lifetime.
///
/// Immunities (documented, both by construction):
/// - wall-clock changes never matter (no wall clock is read);
/// - the agent process itself being suspended never matters (both
///   counters are system-wide and keep running, so the difference does
///   not grow).
#[derive(Debug, Default)]
pub struct ResumeDetector {
    last_diff_ms: Option<u64>,
}

/// Growth of the sleep accumulator between two ticker passes that counts
/// as a sleep. Both counters tick at the OS interrupt granularity
/// (~15.6 ms), so measurement jitter is small; a real sleep is at least
/// seconds. One full second keeps jitter out while catching any real
/// sleep the 1 s ticker could observe.
pub const RESUME_SLEEP_GAP_MS: u64 = 1_000;

impl ResumeDetector {
    /// Observe one `(biased_ms − unbiased_ms)` sample. Returns
    /// `Some(slept_ms)` when the accumulator grew by more than
    /// [`RESUME_SLEEP_GAP_MS`] since the previous observation, `None`
    /// otherwise (including on the first observation, which only anchors
    /// the baseline).
    pub fn observe(&mut self, sleep_accumulated_ms: u64) -> Option<u64> {
        match self.last_diff_ms {
            None => {
                self.last_diff_ms = Some(sleep_accumulated_ms);
                None
            }
            Some(previous) => {
                self.last_diff_ms = Some(sleep_accumulated_ms);
                let grew = sleep_accumulated_ms.saturating_sub(previous);
                (grew > RESUME_SLEEP_GAP_MS).then_some(grew)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(pid: u32) -> ProcessIdentity {
        ProcessIdentity::new(pid, 133_000_000_000_000_000 + u64::from(pid))
    }

    #[test]
    fn cooldown_blocks_for_60s_after_a_terminal_record_then_admits() {
        let mut book = TargetBook::default();
        let target = identity(4242);
        book.after_terminal(&target, ActionState::Restored, 1_000_000);
        assert!(!book.admissible(&target, 1_000_000));
        assert!(!book.admissible(&target, 1_000_000 + COOLDOWN_MS - 1));
        assert!(book.admissible(&target, 1_000_000 + COOLDOWN_MS));
        assert_eq!(book.cooldown_until(&target), Some(1_000_000 + COOLDOWN_MS));
    }

    #[test]
    fn cooldown_is_keyed_per_identity() {
        let mut book = TargetBook::default();
        let a = identity(1);
        let b = identity(2);
        book.after_terminal(&a, ActionState::Restored, 1_000_000);
        assert!(!book.admissible(&a, 1_000_000));
        assert!(
            book.admissible(&b, 1_000_000),
            "one target's cooldown must not affect another"
        );
    }

    #[test]
    fn pinned_records_start_no_cooldown() {
        let mut book = TargetBook::default();
        let target = identity(7);
        // A Conflict record still needs attention: no new experiment.
        book.after_terminal(&target, ActionState::Conflict, 1_000_000);
        assert_eq!(book.cooldown_until(&target), None);
        // (The target stays inadmissible through the journal's unresolved
        // gate — the outstanding action blocks, not the cooldown.)
    }

    #[test]
    fn failed_target_stays_disabled_until_explicit_reenable() {
        let mut book = TargetBook::default();
        let target = identity(9);
        book.disable(&target, "restore failed: access denied");
        assert!(
            !book.admissible(&target, u64::MAX),
            "time never lifts a disable"
        );
        assert_eq!(
            book.disabled_reason(&target),
            Some("restore failed: access denied")
        );
        book.re_enable(&target);
        assert!(book.admissible(&target, 0));
        assert_eq!(book.disabled_reason(&target), None);
    }

    #[test]
    fn one_action_cap_holds_under_repeat_confirmations() {
        let mut cap = OneActionCap::default();
        cap.reserve("act-1").expect("first reserve");
        // Repeat confirmation of the SAME action: idempotent hold.
        cap.reserve("act-1").expect("same-id reserve holds");
        // A different action is refused while the slot is taken.
        let err = cap.reserve("act-2").expect_err("slot taken");
        assert_eq!(err.held_by, "act-1");
        // Terminal release frees exactly that action.
        cap.release("act-1");
        cap.reserve("act-2").expect("free after release");
        // Releasing a non-holder changes nothing.
        cap.release("act-1");
        assert_eq!(cap.holder(), Some("act-2"));
    }

    #[test]
    fn child_spawn_trips_restore_and_records_the_finding() {
        let parent = identity(4242);
        let child = identity(5000);
        let decision = evaluate(
            &SafetyObservation::ChildSpawned {
                parent,
                child,
                child_image: "child-updater.exe".to_string(),
            },
            5_000_000,
        );
        match decision {
            Decision::Trip {
                trigger,
                finding: Some(finding),
            } => {
                assert_eq!(trigger, TripTrigger::ChildSpawned);
                assert_eq!(trigger.code(), "child_spawned");
                assert_eq!(finding.child_image, "child-updater.exe");
                assert_eq!(finding.child_pid, 5000);
                assert_eq!(finding.recorded_at_unix_ms, 5_000_000);
                // AT-14's "unapproved descendants receive no explicit
                // write": the decision is a restore of the PARENT; the
                // child appears only as a recorded finding.
                assert_eq!(finding.parent_image, "");
            }
            other => panic!("expected Trip, got {other:?}"),
        }
    }

    #[test]
    fn foreground_session_and_stale_focus_map_to_their_triggers() {
        assert_eq!(
            evaluate(&SafetyObservation::ForegroundReturned, 1),
            Decision::Trip {
                trigger: TripTrigger::ForegroundReturned,
                finding: None,
            }
        );
        assert_eq!(
            evaluate(&SafetyObservation::SessionLocked, 2),
            Decision::Trip {
                trigger: TripTrigger::SessionLock,
                finding: None,
            }
        );
        assert_eq!(
            evaluate(&SafetyObservation::FocusStale { age_ms: 9_999 }, 3),
            Decision::Trip {
                trigger: TripTrigger::StaleSafetyInput,
                finding: None,
            }
        );
        assert_eq!(
            evaluate(&SafetyObservation::SystemResumed { sleep_ms: 45_000 }, 4),
            Decision::Trip {
                trigger: TripTrigger::SystemResumed,
                finding: None,
            }
        );
    }

    #[test]
    fn trigger_codes_are_stable_for_the_evidence_trail() {
        assert_eq!(TripTrigger::ChildSpawned.code(), "child_spawned");
        assert_eq!(
            TripTrigger::ForegroundReturned.code(),
            "foreground_returned"
        );
        assert_eq!(TripTrigger::SessionLock.code(), "session_lock");
        assert_eq!(TripTrigger::StaleSafetyInput.code(), "stale_safety_input");
        assert_eq!(TripTrigger::SystemResumed.code(), "system_resumed");
    }

    #[test]
    fn resume_detector_needs_a_baseline_and_a_real_jump() {
        let mut detector = ResumeDetector::default();
        // First observation only anchors the baseline.
        assert_eq!(detector.observe(100), None);
        // Small growth while awake (interrupt jitter, short scheduler
        // stalls) is not a sleep — every step stays under the threshold.
        assert_eq!(detector.observe(150), None);
        assert_eq!(detector.observe(700), None);
        // A jump past the threshold is the sleep, reported with its size.
        assert_eq!(detector.observe(700 + 45_678), Some(45_678));
        // Awake again: no further trips.
        assert_eq!(detector.observe(700 + 45_678 + 90), None);
        // A shrinking accumulator (counter reset) is not a sleep — it is
        // ignored, and the next growth is measured from the new baseline.
        assert_eq!(detector.observe(50), None);
        assert_eq!(
            detector.observe(50 + RESUME_SLEEP_GAP_MS + 50),
            Some(RESUME_SLEEP_GAP_MS + 50)
        );
    }
}
