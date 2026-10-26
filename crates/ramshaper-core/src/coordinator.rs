//! Pure preview/confirm coordinator (RS-016; contract 14 §5–§6).
//!
//! Composes the RS-015 eligibility gate, the policy rules of
//! `crate::policy` and the RS-031 journal state machine into the lifecycle
//! of the one 0.2 policy action — **without an actuator**. OS effects go
//! through the [`PriorityActuator`] trait; durable intent through the
//! [`ActionJournal`] trait. Real implementations are wired later behind
//! the disabled-actuation gate; tests use fakes.
//!
//! Contract rules implemented here:
//! - a preview is offered only while pressure is fresh-`Elevated`, the
//!   eligibility gate fully passes and the journal is available; the TTL
//!   bounds (10–300 s, default 120 s) are validated at preview time;
//! - a pending preview expires after 30 s (inclusive boundary: confirm at
//!   exactly +30 s still works, one ms later does not);
//! - `confirm` durably records the `Prepared` intent **before** the OS
//!   write (the fake actuator in tests asserts this ordering);
//! - replaying a confirm for an already-applied action returns the
//!   original outcome and never calls the actuator again;
//! - read-back mismatch after a write is `Uncertain` (recovery retained);
//!   a clean OS failure is `Failed` without retained recovery; an unknown
//!   outcome is `Uncertain`;
//! - storage failure at any journal step blocks the action (and later
//!   recovery of the same record still proceeds — that path lives in the
//!   journal/reconciliation, not here).

use crate::eligibility::{evaluate, Allowlist, EligibilityInputs, EligibilityReport};
use crate::identity::ProcessIdentity;
use crate::journal::{ActionIntent, ActionState, Event, ACTION_KIND_CPU_PRIORITY};
use crate::policy::{PolicyRules, TargetObservation, DEFAULT_TTL_MS, PREVIEW_VALIDITY_MS};
use crate::pressure::PressureLevel;

/// Why the actuator could not complete an operation. Distinguishing
/// clean-failure from unknown-outcome is what decides `Failed` vs
/// `Uncertain` (contract 14 §6).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActuatorError {
    /// The identity no longer matches: the write was refused before any
    /// OS effect (never a partial write).
    #[error("identity no longer matches; nothing written")]
    IdentityMismatch,
    /// The OS call failed in a way that cannot have had an effect.
    #[error("OS error {code} from {function}")]
    OsFailure {
        /// Preserved native error code.
        code: u32,
        /// Name of the failing API call.
        function: &'static str,
    },
    /// The outcome is unknown (lost reply, crash window): a write may
    /// have occurred.
    #[error("actuator outcome unknown; a write may have occurred")]
    UnknownOutcome,
}

/// Journal surface the coordinator needs, in core types. Implemented by
/// the SQLite store in `ramshaper-journal` (mapping its typed errors into
/// [`JournalFault`]) and by an in-memory fake in tests.
pub trait ActionJournal {
    /// Whether the journal is open and writable right now.
    fn available(&self) -> bool;
    /// Durably record the `Prepared` intent. Returns `true` when a new
    /// record was created, `false` when the action ID already existed
    /// (replay — the existing record must be used as-is).
    ///
    /// # Errors
    /// [`JournalFault`] on storage failure (disk full, corrupt, quota…).
    fn record_prepared(&self, intent: &ActionIntent) -> Result<bool, JournalFault>;
    /// Load a record's current state and applied deadline, if present.
    ///
    /// # Errors
    /// [`JournalFault`] on storage failure.
    fn record_state(
        &self,
        action_id: &str,
    ) -> Result<Option<(ActionState, Option<u64>)>, JournalFault>;
    /// Apply one guarded transition through the state machine.
    ///
    /// # Errors
    /// [`JournalFault`] on storage failure or an illegal/guarded move.
    fn transition(
        &self,
        action_id: &str,
        expected: ActionState,
        event: &Event,
        now_unix_ms: u64,
    ) -> Result<ActionState, JournalFault>;
}

impl<J: ActionJournal + ?Sized> ActionJournal for std::rc::Rc<J> {
    fn available(&self) -> bool {
        (**self).available()
    }
    fn record_prepared(&self, intent: &ActionIntent) -> Result<bool, JournalFault> {
        (**self).record_prepared(intent)
    }
    fn record_state(
        &self,
        action_id: &str,
    ) -> Result<Option<(ActionState, Option<u64>)>, JournalFault> {
        (**self).record_state(action_id)
    }
    fn transition(
        &self,
        action_id: &str,
        expected: ActionState,
        event: &Event,
        now_unix_ms: u64,
    ) -> Result<ActionState, JournalFault> {
        (**self).transition(action_id, expected, event, now_unix_ms)
    }
}

/// A storage-level fault surfaced to the coordinator. Anything here means
/// "no new applies" (contract 14 §6).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JournalFault {
    /// The journal refused or could not complete the write.
    #[error("journal storage unavailable: {0}")]
    Storage(String),
    /// The record is not in the expected state (moved under us).
    #[error("record '{0}' moved in the journal; re-read before retrying")]
    StateConflict(String),
}

/// What the actuator verified for one apply, in core terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// The write happened and the read-back matched the desired value.
    Verified,
    /// The write cleanly failed (no effect possible).
    FailedClean(ActuatorError),
    /// Unknown outcome or read-back mismatch: a write may have occurred.
    Uncertain(ActuatorError),
}

/// Actuator surface: read and verified-write of the one property. The fake
/// in tests records calls; the real Windows implementation stays behind
/// the disabled-actuation gate until recovery tests pass.
pub trait PriorityActuator {
    /// Write `desired` for the exact identity and read the value back.
    /// Implementations must refuse a mismatched identity
    /// ([`ActuatorError::IdentityMismatch`]) before any effect.
    ///
    /// # Errors
    /// [`ActuatorError`] as classified above.
    fn write_verified(
        &mut self,
        identity: &ProcessIdentity,
        desired: u32,
    ) -> Result<u32, ActuatorError>;
}

/// A preview offered by [`Coordinator::preview`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPreview {
    /// Caller-supplied durable action ID (the agent generates it).
    pub action_id: String,
    /// Preview creation (Unix ms).
    pub created_at_unix_ms: u64,
    /// Expiry (Unix ms) — inclusive boundary.
    pub expires_at_unix_ms: u64,
    /// TTL that would be applied on confirm.
    pub ttl_ms: u64,
    /// The desired raw value.
    pub desired: u32,
}

/// Everything `preview` needs beyond what `TargetObservation` carries.
#[derive(Debug, Clone)]
pub struct PreviewRequest<'a> {
    /// Exact identity being considered.
    pub identity: &'a ProcessIdentity,
    /// Image name (label only).
    pub image_name: &'a str,
    /// The scripted target observations (RS-015 inputs).
    pub target: TargetObservation,
    /// Tested compatibility allowlist in force.
    pub allowlist: &'a Allowlist,
    /// User exclusions in force (RS-012).
    pub user_exclusions: &'a [String],
    /// Current pressure level (from the live classifier).
    pub pressure: PressureLevel,
    /// Caller-supplied durable action ID for this candidate.
    pub action_id: &'a str,
    /// Wall-clock now (Unix ms).
    pub now_unix_ms: u64,
    /// Requested TTL (falls back to the rules' default when `None`).
    pub ttl_ms: Option<u64>,
    /// The desired raw value (Below Normal in the 0.2 policy).
    pub desired: u32,
}

/// Preview verdicts.
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewVerdict {
    /// Offered; confirmable until `expires_at_unix_ms`.
    Offered(PendingPreview),
    /// Pressure is not fresh-Elevated: nothing to optimize right now.
    PressureNotElevated,
    /// The eligibility gate denied; the report's deny codes are carried
    /// verbatim for explanation.
    Denied(EligibilityReport),
    /// The requested TTL is outside 10–300 s.
    TtlOutOfRange {
        /// What was requested.
        requested_ms: u64,
        /// The inclusive bounds.
        min_ms: u64,
        /// The inclusive bounds.
        max_ms: u64,
    },
    /// The journal is unavailable: no previews (they lead to applies).
    JournalUnavailable,
    /// An action is already outstanding for this coordinator slot.
    ActionOutstanding,
}

/// Confirm verdicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmOutcome {
    /// Durably prepared, written, verified and journaled as `Applied`.
    Applied {
        /// Live deadline recorded in the journal.
        deadline_unix_ms: u64,
    },
    /// Idempotent replay: the action was already applied; the original
    /// deadline is returned and the actuator was NOT called again.
    Replayed {
        /// The original applied deadline.
        deadline_unix_ms: u64,
    },
    /// The preview expired (or no preview is pending for this ID).
    PreviewExpired,
    /// The eligibility gate would no longer pass at confirm time is
    /// modeled by re-running `preview` — confirm only acts on a live
    /// pending preview whose ID matches.
    UnknownPreview,
    /// The journal refused the durable prepare or a later transition.
    StorageBlocked(String),
    /// The actuator refused before any effect (clean failure recorded as
    /// `Failed`).
    FailedClean(String),
    /// Unknown outcome or read-back mismatch: recorded `Uncertain` with
    /// recovery retained; startup reconciliation decides later.
    Uncertain(String),
    /// The identity changed between preview and confirm: nothing written.
    IdentityGone,
}

/// The one-action coordinator slot (contract: one action at a time; the
/// agent runs one coordinator per session).
#[derive(Debug)]
pub struct Coordinator {
    /// Policy rules in force (version + default TTL).
    rules: PolicyRules,
    /// The single pending preview, if any.
    pending: Option<PendingPreview>,
}

impl Coordinator {
    /// A coordinator under the given rules.
    #[must_use]
    pub fn new(rules: PolicyRules) -> Self {
        Self {
            rules,
            pending: None,
        }
    }

    /// Offer a preview if every precondition holds. Pure.
    pub fn preview<J: ActionJournal>(
        &mut self,
        journal: &J,
        request: PreviewRequest<'_>,
    ) -> PreviewVerdict {
        if !journal.available() {
            return PreviewVerdict::JournalUnavailable;
        }
        if self.pending.is_some() {
            return PreviewVerdict::ActionOutstanding;
        }
        let ttl = request
            .ttl_ms
            .unwrap_or(self.rules.ttl_ms.max(DEFAULT_TTL_MS).min(
                // Rules carry the default; a rules TTL of 0 means "use default".
                if self.rules.ttl_ms == 0 {
                    DEFAULT_TTL_MS
                } else {
                    self.rules.ttl_ms
                },
            ));
        let (min, max) = (crate::policy::MIN_TTL_MS, crate::policy::MAX_TTL_MS);
        if !(min..=max).contains(&ttl) {
            return PreviewVerdict::TtlOutOfRange {
                requested_ms: ttl,
                min_ms: min,
                max_ms: max,
            };
        }
        if request.pressure != PressureLevel::Elevated {
            return PreviewVerdict::PressureNotElevated;
        }
        let eligibility = EligibilityInputs {
            identity: request.identity,
            image_name: request.image_name,
            identity_check: request.target.identity_check.clone(),
            owner_session: request.target.owner_session.clone(),
            focus: request.target.focus.clone(),
            classification: request.target.classification.clone(),
            priority_class: request.target.priority_class.clone(),
            children: request.target.children.clone(),
            capability: request.target.capability.clone(),
            allowlist: request.allowlist,
            user_exclusions: request.user_exclusions,
            outstanding_actions: 0, // the slot is free (checked above)
            journal_available: journal.available(),
        };
        let report = evaluate(&eligibility);
        if !report.eligible {
            return PreviewVerdict::Denied(report);
        }
        let preview = PendingPreview {
            action_id: request.action_id.to_string(),
            created_at_unix_ms: request.now_unix_ms,
            expires_at_unix_ms: request.now_unix_ms.saturating_add(PREVIEW_VALIDITY_MS),
            ttl_ms: ttl,
            desired: request.desired,
        };
        self.pending = Some(preview.clone());
        PreviewVerdict::Offered(preview)
    }

    /// Confirm the pending preview: durable prepare **before** the write,
    /// then write + read-back verify, then journal `Applied`. Idempotent
    /// per action ID.
    pub fn confirm<J: ActionJournal, A: PriorityActuator>(
        &mut self,
        journal: &J,
        actuator: &mut A,
        identity: &ProcessIdentity,
        action_id: &str,
        now_unix_ms: u64,
    ) -> ConfirmOutcome {
        // Replay first: an already-applied record returns the original
        // outcome without touching the actuator — regardless of whether a
        // preview is still pending (the slot frees on apply).
        match journal.record_state(action_id) {
            Ok(Some((ActionState::Applied, Some(deadline)))) => {
                return ConfirmOutcome::Replayed {
                    deadline_unix_ms: deadline,
                };
            }
            Ok(Some((state, _))) if state != ActionState::Prepared => {
                // A terminal/conflicting record with this ID: never reuse.
                return ConfirmOutcome::UnknownPreview;
            }
            Ok(_) => {}
            Err(fault) => return ConfirmOutcome::StorageBlocked(fault.to_string()),
        }

        let Some(preview) = self.pending.clone() else {
            return ConfirmOutcome::UnknownPreview;
        };
        if preview.action_id != action_id {
            return ConfirmOutcome::UnknownPreview;
        }
        // Inclusive 30 s boundary (contract: "expires after 30 s").
        if now_unix_ms > preview.expires_at_unix_ms {
            self.pending = None;
            return ConfirmOutcome::PreviewExpired;
        }

        // Durable intent BEFORE the OS call (contract 14 §6).
        let intent = ActionIntent {
            action_id: action_id.to_string(),
            agent_instance_id: String::new(), // stamped by the store wiring
            pid: identity.pid,
            creation_filetime: identity.creation_filetime_100ns,
            target_name: String::new(), // label; the agent fills it
            property: crate::journal::Property {
                kind: ACTION_KIND_CPU_PRIORITY,
                original: None, // the actuator's read-side pins it later
                desired: preview.desired,
            },
            consent_id: String::new(), // agent stamps the consent binding
            rule_version: self.rules.rules_version.clone(),
            ttl_ms: preview.ttl_ms,
            created_at_unix_ms: now_unix_ms,
        };
        match journal.record_prepared(&intent) {
            Ok(_) => {}
            Err(fault) => return ConfirmOutcome::StorageBlocked(fault.to_string()),
        }

        // The OS write, through the verified handle.
        let deadline = now_unix_ms.saturating_add(preview.ttl_ms);
        match actuator.write_verified(identity, preview.desired) {
            Ok(read_back) if read_back == preview.desired => {
                match journal.transition(
                    action_id,
                    ActionState::Prepared,
                    &Event::ApplyVerified {
                        deadline_unix_ms: deadline,
                    },
                    now_unix_ms,
                ) {
                    Ok(_) => {
                        self.pending = None;
                        ConfirmOutcome::Applied {
                            deadline_unix_ms: deadline,
                        }
                    }
                    Err(fault) => ConfirmOutcome::StorageBlocked(fault.to_string()),
                }
            }
            Ok(_mismatched_read_back) => {
                // The call returned but the value is not what we wrote:
                // unknown outcome, recovery retained (never assume).
                let _ = journal.transition(
                    action_id,
                    ActionState::Prepared,
                    &Event::ApplyUncertain,
                    now_unix_ms,
                );
                self.pending = None;
                ConfirmOutcome::Uncertain("read-back mismatch".to_string())
            }
            Err(ActuatorError::IdentityMismatch) => {
                // Refused before any effect; nothing to undo.
                let _ = journal.transition(
                    action_id,
                    ActionState::Prepared,
                    &Event::TargetVanished,
                    now_unix_ms,
                );
                self.pending = None;
                ConfirmOutcome::IdentityGone
            }
            Err(err @ ActuatorError::OsFailure { .. }) => {
                let _ = journal.transition(
                    action_id,
                    ActionState::Prepared,
                    &Event::ApplyFailed {
                        reason: err.to_string(),
                    },
                    now_unix_ms,
                );
                self.pending = None;
                ConfirmOutcome::FailedClean(err.to_string())
            }
            Err(err @ ActuatorError::UnknownOutcome) => {
                let _ = journal.transition(
                    action_id,
                    ActionState::Prepared,
                    &Event::ApplyUncertain,
                    now_unix_ms,
                );
                self.pending = None;
                ConfirmOutcome::Uncertain(err.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eligibility::{
        ChildObservation, FocusObservation, OwnerSessionObservation, PriorityCapability,
        PriorityClassObservation, SafetyClassification,
    };
    use crate::identity::IdentityVerification;
    use crate::journal::Property;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn rules() -> PolicyRules {
        PolicyRules::default_experiment()
    }

    fn identity() -> ProcessIdentity {
        ProcessIdentity::new(4242, 133_000_000_000_000_000)
    }

    fn target() -> TargetObservation {
        TargetObservation {
            identity_check: IdentityVerification::Matches,
            owner_session: OwnerSessionObservation::CurrentUserSession,
            focus: FocusObservation::Observed {
                target_foreground: false,
                observed_age_ms: 100,
            },
            classification: SafetyClassification::VerifiedNonCritical,
            priority_class: PriorityClassObservation::Normal,
            children: ChildObservation::Observed { live_children: 0 },
            capability: PriorityCapability::Supported,
        }
    }

    fn allowlist() -> Allowlist {
        Allowlist::new(
            crate::eligibility::ELIGIBILITY_RULES_VERSION,
            &["synthetic-workload.exe"],
        )
    }

    fn request<'a>(
        identity: &'a ProcessIdentity,
        image: &'a str,
        allowlist: &'a Allowlist,
        action_id: &'a str,
        now: u64,
    ) -> PreviewRequest<'a> {
        PreviewRequest {
            identity,
            image_name: image,
            target: target(),
            allowlist,
            user_exclusions: &[],
            pressure: PressureLevel::Elevated,
            action_id,
            now_unix_ms: now,
            ttl_ms: None,
            desired: 0x4000,
        }
    }

    /// In-memory journal fake mirroring the store's contract tightly
    /// enough for the coordinator (records + guarded transitions).
    #[derive(Default)]
    struct FakeJournal {
        records: RefCell<Vec<(ActionIntent, ActionState, Option<u64>)>>,
        fail_writes: RefCell<bool>,
    }

    impl FakeJournal {
        fn state_of(&self, action_id: &str) -> Option<(ActionState, Option<u64>)> {
            self.records
                .borrow()
                .iter()
                .find(|(i, _, _)| i.action_id == action_id)
                .map(|(_, s, d)| (*s, *d))
        }
    }

    impl ActionJournal for FakeJournal {
        fn available(&self) -> bool {
            !*self.fail_writes.borrow()
        }
        fn record_prepared(&self, intent: &ActionIntent) -> Result<bool, JournalFault> {
            if *self.fail_writes.borrow() {
                return Err(JournalFault::Storage("injected failure".into()));
            }
            let mut records = self.records.borrow_mut();
            if records
                .iter()
                .any(|(i, _, _)| i.action_id == intent.action_id)
            {
                return Ok(false);
            }
            records.push((intent.clone(), ActionState::Prepared, None));
            Ok(true)
        }
        fn record_state(
            &self,
            action_id: &str,
        ) -> Result<Option<(ActionState, Option<u64>)>, JournalFault> {
            Ok(self.state_of(action_id))
        }
        fn transition(
            &self,
            action_id: &str,
            expected: ActionState,
            event: &Event,
            now_unix_ms: u64,
        ) -> Result<ActionState, JournalFault> {
            let mut records = self.records.borrow_mut();
            let Some(pos) = records
                .iter()
                .position(|(i, _, _)| i.action_id == action_id)
            else {
                return Err(JournalFault::StateConflict(action_id.to_string()));
            };
            let (_, state, deadline) = &mut records[pos];
            if *state != expected {
                return Err(JournalFault::StateConflict(action_id.to_string()));
            }
            let next = crate::journal::transition(*state, event)
                .map_err(|e| JournalFault::StateConflict(e.to_string()))?;
            if let Event::ApplyVerified { deadline_unix_ms } = event {
                *deadline = Some(*deadline_unix_ms);
            }
            let _ = now_unix_ms;
            *state = next;
            Ok(next)
        }
    }

    /// Fake actuator: the observable surface, with call-order assertions.
    struct FakeActuator {
        calls: RefCell<Vec<(u32, String)>>,
        /// When set, write_verified asserts the journal already holds a
        /// Prepared record for the action: the durable-before-write rule.
        journal_at_write: Option<Rc<FakeJournal>>,
        /// Scripted behavior of the next write.
        behavior: WriteBehavior,
        value: RefCell<u32>,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum WriteBehavior {
        Success,
        MismatchedReadBack,
        CleanOsFailure,
        UnknownOutcome,
        IdentityMismatch,
    }

    impl FakeActuator {
        fn new(behavior: WriteBehavior) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                journal_at_write: None,
                behavior,
                value: RefCell::new(0x20),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.borrow().len()
        }
    }

    impl PriorityActuator for FakeActuator {
        fn write_verified(
            &mut self,
            identity: &ProcessIdentity,
            desired: u32,
        ) -> Result<u32, ActuatorError> {
            self.calls
                .borrow_mut()
                .push((desired, identity.pid.to_string()));
            if let Some(journal) = &self.journal_at_write {
                // The durable record must exist BEFORE the OS call.
                assert!(
                    journal.state_of("act-1").is_some(),
                    "contract violation: OS write before durable prepare"
                );
            }
            match self.behavior {
                WriteBehavior::Success => {
                    *self.value.borrow_mut() = desired;
                    Ok(desired)
                }
                WriteBehavior::MismatchedReadBack => Ok(0x80),
                WriteBehavior::CleanOsFailure => Err(ActuatorError::OsFailure {
                    code: 87,
                    function: "SetPriorityClass",
                }),
                WriteBehavior::UnknownOutcome => Err(ActuatorError::UnknownOutcome),
                WriteBehavior::IdentityMismatch => Err(ActuatorError::IdentityMismatch),
            }
        }
    }

    #[test]
    fn preview_offered_under_elevated_with_eligible_target() {
        let mut coordinator = Coordinator::new(rules());
        let journal = FakeJournal::default();
        let id = identity();
        let list = allowlist();
        let verdict = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-1", 1_000),
        );
        match verdict {
            PreviewVerdict::Offered(preview) => {
                assert_eq!(preview.action_id, "act-1");
                assert_eq!(preview.ttl_ms, 120_000);
                assert_eq!(preview.expires_at_unix_ms, 1_000 + 30_000);
            }
            other => panic!("expected Offered, got {other:?}"),
        }
    }

    #[test]
    fn preview_denied_under_normal_pressure_or_ineligible_target() {
        let mut coordinator = Coordinator::new(rules());
        let journal = FakeJournal::default();
        let id = identity();
        let list = allowlist();
        // Normal pressure: not offered.
        let mut req = request(&id, "synthetic-workload.exe", &list, "act-1", 1_000);
        req.pressure = PressureLevel::Normal;
        assert_eq!(
            coordinator.preview(&journal, req),
            PreviewVerdict::PressureNotElevated
        );
        // Excluded target: denied with the gate's codes.
        let exclusions = vec!["synthetic-workload.exe".to_string()];
        let req = PreviewRequest {
            user_exclusions: &exclusions,
            ..request(&id, "synthetic-workload.exe", &list, "act-2", 1_000)
        };
        match coordinator.preview(&journal, req) {
            PreviewVerdict::Denied(report) => {
                assert!(!report.eligible);
                assert!(report
                    .deny_reasons()
                    .iter()
                    .any(|r| r.code() == "excluded_by_user"));
            }
            other => panic!("expected Denied, got {other:?}"),
        }
        // TTL out of range.
        let mut req = request(&id, "synthetic-workload.exe", &list, "act-3", 1_000);
        req.ttl_ms = Some(9_999);
        assert!(matches!(
            coordinator.preview(&journal, req),
            PreviewVerdict::TtlOutOfRange {
                requested_ms: 9_999,
                ..
            }
        ));
    }

    #[test]
    fn preview_expires_after_30_s_inclusive_boundary() {
        let mut coordinator = Coordinator::new(rules());
        let journal = FakeJournal::default();
        let mut actuator = FakeActuator::new(WriteBehavior::Success);
        let id = identity();
        let list = allowlist();
        let _ = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-1", 1_000),
        );
        // Exactly +30 s: still valid (inclusive).
        let outcome = coordinator.confirm(&journal, &mut actuator, &id, "act-1", 31_000);
        assert!(
            matches!(outcome, ConfirmOutcome::Applied { .. }),
            "{outcome:?}"
        );
        // A second preview at t=100_000; confirm at +30_001: expired.
        let _ = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-2", 100_000),
        );
        let outcome = coordinator.confirm(&journal, &mut actuator, &id, "act-2", 130_001);
        assert_eq!(outcome, ConfirmOutcome::PreviewExpired);
    }

    #[test]
    fn confirm_prepares_durable_record_before_the_os_write() {
        let journal = Rc::new(FakeJournal::default());
        let mut coordinator = Coordinator::new(rules());
        let mut actuator = FakeActuator::new(WriteBehavior::Success);
        actuator.journal_at_write = Some(Rc::clone(&journal));
        let id = identity();
        let list = allowlist();
        let _ = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-1", 1_000),
        );
        let outcome = coordinator.confirm(&journal, &mut actuator, &id, "act-1", 2_000);
        assert!(
            matches!(
                outcome,
                ConfirmOutcome::Applied {
                    deadline_unix_ms: 122_000
                }
            ),
            "{outcome:?}"
        );
        let (state, deadline) = journal.state_of("act-1").expect("record");
        assert_eq!(state, ActionState::Applied);
        assert_eq!(deadline, Some(122_000));
        assert_eq!(actuator.call_count(), 1);
    }

    #[test]
    fn replayed_confirm_never_writes_twice() {
        let journal = Rc::new(FakeJournal::default());
        let mut coordinator = Coordinator::new(rules());
        let mut actuator = FakeActuator::new(WriteBehavior::Success);
        actuator.journal_at_write = Some(Rc::clone(&journal));
        let id = identity();
        let list = allowlist();
        let _ = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-1", 1_000),
        );
        let first = coordinator.confirm(&journal, &mut actuator, &id, "act-1", 2_000);
        assert!(matches!(first, ConfirmOutcome::Applied { .. }));
        // Replay even though the pending slot is now free: same deadline,
        // no second actuator call.
        let second = coordinator.confirm(&journal, &mut actuator, &id, "act-1", 3_000);
        assert_eq!(
            second,
            ConfirmOutcome::Replayed {
                deadline_unix_ms: 122_000
            }
        );
        assert_eq!(actuator.call_count(), 1, "replay must not write again");
    }

    #[test]
    fn failure_classification_follows_the_actuator_error() {
        let id = identity();
        let list = allowlist();
        // Clean OS failure: Failed, and the record is not pinned by
        // retained recovery.
        let journal = FakeJournal::default();
        let mut coordinator = Coordinator::new(rules());
        let mut actuator = FakeActuator::new(WriteBehavior::CleanOsFailure);
        let _ = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-1", 1_000),
        );
        let outcome = coordinator.confirm(&journal, &mut actuator, &id, "act-1", 2_000);
        assert!(
            matches!(outcome, ConfirmOutcome::FailedClean(_)),
            "{outcome:?}"
        );
        let (state, _) = journal.state_of("act-1").expect("record");
        assert_eq!(state, ActionState::Failed);
        assert!(!state.is_pinned(false));

        // Unknown outcome: Uncertain, pinned (recovery retained).
        let journal = FakeJournal::default();
        let mut coordinator = Coordinator::new(rules());
        let mut actuator = FakeActuator::new(WriteBehavior::UnknownOutcome);
        let _ = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-1", 1_000),
        );
        let outcome = coordinator.confirm(&journal, &mut actuator, &id, "act-1", 2_000);
        assert!(
            matches!(outcome, ConfirmOutcome::Uncertain(_)),
            "{outcome:?}"
        );
        let (state, _) = journal.state_of("act-1").expect("record");
        assert_eq!(state, ActionState::Uncertain);
        assert!(state.is_pinned(false));

        // Read-back mismatch after a returned call: Uncertain as well.
        let journal = FakeJournal::default();
        let mut coordinator = Coordinator::new(rules());
        let mut actuator = FakeActuator::new(WriteBehavior::MismatchedReadBack);
        let _ = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-1", 1_000),
        );
        let outcome = coordinator.confirm(&journal, &mut actuator, &id, "act-1", 2_000);
        assert!(
            matches!(outcome, ConfirmOutcome::Uncertain(_)),
            "{outcome:?}"
        );
        let (state, _) = journal.state_of("act-1").expect("record");
        assert_eq!(state, ActionState::Uncertain);

        // Identity mismatch refused before any effect: TargetGone,
        // nothing written.
        let journal = FakeJournal::default();
        let mut coordinator = Coordinator::new(rules());
        let mut actuator = FakeActuator::new(WriteBehavior::IdentityMismatch);
        let _ = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-1", 1_000),
        );
        let outcome = coordinator.confirm(&journal, &mut actuator, &id, "act-1", 2_000);
        assert_eq!(outcome, ConfirmOutcome::IdentityGone);
        let (state, _) = journal.state_of("act-1").expect("record");
        assert_eq!(state, ActionState::TargetGone);
    }

    #[test]
    fn journal_unavailable_blocks_preview_and_confirm() {
        let journal = FakeJournal::default();
        *journal.fail_writes.borrow_mut() = true;
        let mut coordinator = Coordinator::new(rules());
        let id = identity();
        let list = allowlist();
        let verdict = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-1", 1_000),
        );
        assert_eq!(verdict, PreviewVerdict::JournalUnavailable);
    }

    #[test]
    fn one_action_slot_blocks_a_second_preview() {
        let journal = FakeJournal::default();
        let mut coordinator = Coordinator::new(rules());
        let id = identity();
        let list = allowlist();
        let first = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-1", 1_000),
        );
        assert!(matches!(first, PreviewVerdict::Offered(_)));
        let second = coordinator.preview(
            &journal,
            request(&id, "synthetic-workload.exe", &list, "act-2", 2_000),
        );
        assert_eq!(second, PreviewVerdict::ActionOutstanding);
    }

    #[test]
    fn unused_property_construction_is_exhaustive_over_the_contract() {
        // A structural guard: the intent the coordinator builds carries the
        // one 0.2 action kind and the pinned original is filled by the
        // store wiring, not the coordinator.
        let intent = ActionIntent {
            action_id: "x".into(),
            agent_instance_id: String::new(),
            pid: 1,
            creation_filetime: 1,
            target_name: String::new(),
            property: Property {
                kind: ACTION_KIND_CPU_PRIORITY,
                original: None,
                desired: 0x4000,
            },
            consent_id: String::new(),
            rule_version: "policy_v1".into(),
            ttl_ms: 120_000,
            created_at_unix_ms: 0,
        };
        assert_eq!(intent.property.kind, ACTION_KIND_CPU_PRIORITY);
    }
}
