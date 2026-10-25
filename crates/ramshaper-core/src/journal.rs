//! Action journal: pure state machine, reconciliation and retention rules
//! (RS-031 first slice; contract 14 §6).
//!
//! This module is pure: records, legal transitions, startup reconciliation
//! and pinning/quota decisions. The SQLite-backed store lives in
//! `ramshaper-journal`; the OS actuator is faked in tests (no real policy
//! writes exist in this build — Observe Only).
//!
//! Contract rules implemented here:
//! - records begin at `Prepared`: identity, authorization context,
//!   original/desired values, consent, rule revision, TTL and action ID are
//!   durably committed **before** the OS call (`Proposed` previews are
//!   ephemeral and never journaled);
//! - a crash after `Prepared` is indistinguishable from write-then-crash:
//!   startup **reconciles from the current observed value and never
//!   reapplies**;
//! - restore only happens when the current value still equals RAMShaper's
//!   written value; anything else is `Conflict` (external change preserved);
//! - `TargetGone` never writes to a replacement process;
//! - active/unresolved records are pinned: they are exempt from pruning and
//!   a full record cap **blocks new actions** instead of evicting them;
//! - an unreadable original value is never inferred from defaults.

use serde::Serialize;

/// Journal schema version implemented by this build. A database written by
/// a newer build blocks new policy actions and is never touched.
pub const JOURNAL_SCHEMA_VERSION: i64 = 1;

/// The one action kind of the 0.2 policy stage (contract 14 §5): a single
/// manually confirmed Normal → Below Normal CPU priority class change.
pub const ACTION_KIND_CPU_PRIORITY: &str = "cpu_priority_class";

/// States of the action state machine (contract 14 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ActionState {
    /// Durable intent committed; the OS call has not been made (or its
    /// outcome is unknown after a crash — reconciliation decides).
    Prepared,
    /// Written through the validated handle and read back; actual result
    /// and live deadline recorded.
    Applied,
    /// A restore trigger fired; identity revalidated, current property
    /// being read; new applies blocked.
    RestorePending,
    /// Current property already equals original, or the restoration write
    /// and read-back matched original.
    Restored,
    /// Original process exited or identity no longer matches; no write to
    /// a replacement.
    TargetGone,
    /// Current property differs from both original and desired: external
    /// change preserved, attention required.
    Conflict,
    /// Recorded error. `recovery_retained` records whether a write may have
    /// occurred (intent stays pinned and reconciles).
    Failed,
    /// Unknown outcome (e.g. lost reply): a write may have occurred; the
    /// record stays pinned and reconciles.
    Uncertain,
}

impl ActionState {
    /// Whether the record must be pinned (exempt from pruning and
    /// clear-history). Terminal resolved states are not pinned.
    #[must_use]
    pub fn is_pinned(self, recovery_retained: bool) -> bool {
        match self {
            Self::Restored | Self::TargetGone => false,
            Self::Failed => recovery_retained,
            Self::Prepared
            | Self::Applied
            | Self::RestorePending
            | Self::Conflict
            | Self::Uncertain => true,
        }
    }

    /// Whether this state still needs recovery work before new actions may
    /// resume after a restart (contract: restart restores outstanding
    /// records before enabling new actions).
    #[must_use]
    pub fn is_unresolved(self, recovery_retained: bool) -> bool {
        self.is_pinned(recovery_retained)
    }
}

/// The one process property this journal can describe. Values are opaque
/// u32 OS values: the actuator owns their meaning. Serialization is
/// write-only (the store reconstitutes records from columns, so borrowed
/// `kind` needs no Deserialize).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Property {
    /// Action kind, [`ACTION_KIND_CPU_PRIORITY`] in this build.
    pub kind: &'static str,
    /// The pinned original value, when it was readable. `None` is honest
    /// "unreadable" and blocks any restoration write.
    pub original: Option<u32>,
    /// The value RAMShaper would write.
    pub desired: u32,
}

/// One durable action record (as persisted at `Prepared`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionIntent {
    /// Durable action ID (replay returns the existing record).
    pub action_id: String,
    /// Agent instance that owns the action.
    pub agent_instance_id: String,
    /// Exact process identity (PID + creation FILETIME) — never a name.
    pub pid: u32,
    pub creation_filetime: u64,
    /// Image name, label only.
    pub target_name: String,
    /// Property (kind + pinned original + desired).
    pub property: Property,
    /// Consent/authorization context binding (agent instance + consent id).
    pub consent_id: String,
    /// Policy rule revision in force at confirmation.
    pub rule_version: String,
    /// Time to live, milliseconds from the applied deadline.
    pub ttl_ms: u64,
    /// Wall-clock creation time (Unix ms, UTC) — deadlines survive restarts.
    pub created_at_unix_ms: u64,
}

/// An event applied to a persisted record. The store refuses a transition
/// whose current state does not match `expected` (optimistic guard).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The OS write succeeded and the read-back matched; carries the live
    /// deadline (Unix ms).
    ApplyVerified { deadline_unix_ms: u64 },
    /// The OS write failed in a way that cannot have had an effect.
    ApplyFailed { reason: String },
    /// The outcome is unknown (lost reply, crash window): a write may have
    /// occurred.
    ApplyUncertain,
    /// A restore trigger fired.
    RestoreRequested,
    /// Restoration verified: current equals original.
    RestoreVerified,
    /// Identity no longer matches or the process exited.
    TargetVanished,
    /// Current value differs from both original and desired.
    ExternalChange,
    /// Startup reconciliation observed the current value equal to
    /// `desired`: the write happened (crash window). The record proceeds to
    /// restoration — never a re-apply of `desired`.
    ReconcileWriteHappened,
    /// Startup reconciliation observed the current value equal to the
    /// pinned `original`: nothing to undo; the record resolves with no OS
    /// call at all.
    ReconcileNoWrite,
}

/// Why a transition was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransitionError {
    #[error("illegal transition {from:?} --{event:?}--> is not allowed")]
    Illegal {
        from: ActionState,
        event: &'static str,
    },
}

/// The legal transition table (contract 14 §6). Pure; the store enforces it
/// atomically per record. (No `#[must_use]`: the `Result` return type already
/// carries it — a redundant attribute trips `clippy::double_must_use`.)
pub fn transition(state: ActionState, event: &Event) -> Result<ActionState, TransitionError> {
    let event_name = event_name(event);
    let next = match (state, event) {
        (ActionState::Prepared, Event::ApplyVerified { .. }) => ActionState::Applied,
        (ActionState::Prepared, Event::ApplyFailed { .. }) => ActionState::Failed,
        (ActionState::Prepared, Event::ApplyUncertain) => ActionState::Uncertain,
        // A restore trigger can arrive while still Prepared (the write never
        // happened): nothing to undo — the record resolves as Restored-by-
        // nothing-to-undo only via reconciliation, so keep it actionable:
        // Prepared stays Prepared until the coordinator applies or resolves.
        (ActionState::Applied, Event::RestoreRequested) => ActionState::RestorePending,
        (ActionState::RestorePending, Event::RestoreVerified) => ActionState::Restored,
        (ActionState::RestorePending, Event::TargetVanished) => ActionState::TargetGone,
        (ActionState::RestorePending, Event::ExternalChange) => ActionState::Conflict,
        // Retained recovery intent on failed restores: the record stays
        // RestorePending (manual retry), per contract — retries do not
        // invent a terminal state.
        (ActionState::RestorePending, Event::ApplyFailed { .. }) => ActionState::RestorePending,
        (ActionState::RestorePending, Event::ApplyUncertain) => ActionState::RestorePending,
        // Reconciliation may confirm the original apply long after the fact.
        (ActionState::Prepared, Event::RestoreRequested) => ActionState::RestorePending,
        // Startup reconciliation (AT-12): Prepared is indistinguishable
        // from write-then-crash — decide from the current value, never
        // reapply.
        (ActionState::Prepared, Event::ReconcileWriteHappened) => ActionState::RestorePending,
        (ActionState::Prepared, Event::ReconcileNoWrite) => ActionState::Restored,
        // Reconciliation of a never-applied record whose target vanished or
        // whose value changed externally resolves terminally the same way.
        (ActionState::Prepared, Event::TargetVanished) => ActionState::TargetGone,
        (ActionState::Prepared, Event::ExternalChange) => ActionState::Conflict,
        (ActionState::Uncertain, Event::ReconcileWriteHappened) => ActionState::RestorePending,
        (ActionState::Uncertain, Event::ReconcileNoWrite) => ActionState::Restored,
        (ActionState::Uncertain, Event::RestoreRequested) => ActionState::RestorePending,
        (ActionState::Uncertain, Event::TargetVanished) => ActionState::TargetGone,
        (ActionState::Uncertain, Event::ExternalChange) => ActionState::Conflict,
        _ => {
            return Err(TransitionError::Illegal {
                from: state,
                event: event_name,
            })
        }
    };
    Ok(next)
}

fn event_name(event: &Event) -> &'static str {
    match event {
        Event::ApplyVerified { .. } => "ApplyVerified",
        Event::ApplyFailed { .. } => "ApplyFailed",
        Event::ApplyUncertain => "ApplyUncertain",
        Event::RestoreRequested => "RestoreRequested",
        Event::RestoreVerified => "RestoreVerified",
        Event::TargetVanished => "TargetVanished",
        Event::ExternalChange => "ExternalChange",
        Event::ReconcileWriteHappened => "ReconcileWriteHappened",
        Event::ReconcileNoWrite => "ReconcileNoWrite",
    }
}

/// What the coordinator observes about a record's target on startup
/// reconciliation (or any revalidation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Observed {
    /// The identity still resolves to the same live process.
    pub identity_matches: bool,
    /// The current property value, when readable. `None` = unreadable —
    /// never inferred from defaults.
    pub current: Option<u32>,
}

/// The startup reconciliation decision (contract: crash after `Prepared` is
/// indistinguishable from write-then-crash; reconcile from the current
/// value, never reapply).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileDecision {
    /// The current value equals `desired`: the write happened. Transition to
    /// Applied-equivalent and continue with restoration.
    WriteHappenedRestore,
    /// The current value equals `original`: no effective write. The record
    /// resolves with nothing to undo (no OS call is made).
    NoWriteOccurred,
    /// The current value differs from both original and desired: external
    /// change, record `Conflict`, preserve it.
    ExternalConflict,
    /// The identity no longer matches: record `TargetGone`, never write to
    /// a replacement.
    TargetGone,
    /// The current value is unreadable: record `Uncertain`, retain pinned
    /// recovery intent and revalidate later.
    Unreadable,
}

/// Decide reconciliation for one unresolved record against the observed
/// facts. Pure; the coordinator performs whatever writes the decision
/// implies (only `WriteHappenedRestore` leads to an OS write — of the
/// *original* value, restoring — never a re-apply of `desired`).
#[must_use]
pub fn reconcile(intent: &ActionIntent, observed: &Observed) -> ReconcileDecision {
    if !observed.identity_matches {
        return ReconcileDecision::TargetGone;
    }
    match observed.current {
        None => ReconcileDecision::Unreadable,
        Some(current) => {
            if current == intent.property.desired {
                ReconcileDecision::WriteHappenedRestore
            } else if Some(current) == intent.property.original {
                ReconcileDecision::NoWriteOccurred
            } else {
                ReconcileDecision::ExternalConflict
            }
        }
    }
}

/// Retention decision for one record at pruning time (contract: active and
/// unresolved records are pinned; terminal records older than the window
/// may go; a full cap blocks new actions rather than evicting).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// Terminal and past the retention window: prunable.
    Prunable,
    /// Kept (terminal within the window, or pinned).
    Keep,
}

/// Decide whether one record may be pruned.
#[must_use]
pub fn retention(
    state: ActionState,
    recovery_retained: bool,
    terminal_at_unix_ms: Option<u64>,
    now_unix_ms: u64,
    retention_window_ms: u64,
) -> Retention {
    if state.is_pinned(recovery_retained) {
        return Retention::Keep;
    }
    match terminal_at_unix_ms {
        Some(at) if now_unix_ms.saturating_sub(at) > retention_window_ms => Retention::Prunable,
        _ => Retention::Keep,
    }
}

/// Whether a new action may be recorded given the current load (contract:
/// record caps block new actions instead of evicting recovery data).
#[must_use]
pub fn may_record_new(total_records: usize, pinned_records: usize, cap: usize) -> bool {
    if total_records < cap {
        return true;
    }
    // At cap: only replaceable (prunable) records make room; pinned records
    // never count as replaceable.
    total_records - pinned_records > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(original: Option<u32>, desired: u32) -> ActionIntent {
        ActionIntent {
            action_id: "act-1".to_string(),
            agent_instance_id: "agent-1".to_string(),
            pid: 4242,
            creation_filetime: 133_000_000_000_000_000,
            target_name: "app.exe".to_string(),
            property: Property {
                kind: ACTION_KIND_CPU_PRIORITY,
                original,
                desired,
            },
            consent_id: "consent-1".to_string(),
            rule_version: "policy_v1".to_string(),
            ttl_ms: 120_000,
            created_at_unix_ms: 1_798_000_000_000,
        }
    }

    #[test]
    fn legal_transitions_follow_the_contract_table() {
        assert_eq!(
            transition(
                ActionState::Prepared,
                &Event::ApplyVerified {
                    deadline_unix_ms: 1
                }
            ),
            Ok(ActionState::Applied)
        );
        assert_eq!(
            transition(
                ActionState::Prepared,
                &Event::ApplyFailed { reason: "e".into() }
            ),
            Ok(ActionState::Failed)
        );
        assert_eq!(
            transition(ActionState::Prepared, &Event::ApplyUncertain),
            Ok(ActionState::Uncertain)
        );
        assert_eq!(
            transition(ActionState::Applied, &Event::RestoreRequested),
            Ok(ActionState::RestorePending)
        );
        assert_eq!(
            transition(ActionState::RestorePending, &Event::RestoreVerified),
            Ok(ActionState::Restored)
        );
        assert_eq!(
            transition(ActionState::RestorePending, &Event::TargetVanished),
            Ok(ActionState::TargetGone)
        );
        assert_eq!(
            transition(ActionState::RestorePending, &Event::ExternalChange),
            Ok(ActionState::Conflict)
        );
        // Failed restores keep the pinned RestorePending (manual retry).
        assert_eq!(
            transition(
                ActionState::RestorePending,
                &Event::ApplyFailed { reason: "e".into() }
            ),
            Ok(ActionState::RestorePending)
        );
    }

    #[test]
    fn illegal_transitions_are_refused() {
        // Nothing may jump backward or skip the read-back.
        assert!(transition(
            ActionState::Applied,
            &Event::ApplyVerified {
                deadline_unix_ms: 1
            }
        )
        .is_err());
        assert!(transition(ActionState::Restored, &Event::RestoreRequested).is_err());
        assert!(transition(ActionState::Prepared, &Event::RestoreVerified).is_err());
        assert!(transition(ActionState::Conflict, &Event::RestoreVerified).is_err());
    }

    /// The core AT-12 rule: crash after Prepared is indistinguishable from
    /// write-then-crash — reconcile from the current value, never reapply.
    #[test]
    fn startup_reconciliation_reads_reality_not_intentions() {
        let record = intent(Some(0x20), 0x4000);
        // The write happened: restore the original — never re-apply desired.
        assert_eq!(
            reconcile(
                &record,
                &Observed {
                    identity_matches: true,
                    current: Some(0x4000)
                }
            ),
            ReconcileDecision::WriteHappenedRestore
        );
        // The write never happened: resolve without any OS call.
        assert_eq!(
            reconcile(
                &record,
                &Observed {
                    identity_matches: true,
                    current: Some(0x20)
                }
            ),
            ReconcileDecision::NoWriteOccurred
        );
        // An external writer left a third value: preserve it as Conflict.
        assert_eq!(
            reconcile(
                &record,
                &Observed {
                    identity_matches: true,
                    current: Some(0x80)
                }
            ),
            ReconcileDecision::ExternalConflict
        );
        // Replaced PID: TargetGone, never write to the replacement.
        assert_eq!(
            reconcile(
                &record,
                &Observed {
                    identity_matches: false,
                    current: Some(0x4000)
                }
            ),
            ReconcileDecision::TargetGone
        );
        // Unreadable: Uncertain with pinned intent, nothing inferred.
        assert_eq!(
            reconcile(
                &record,
                &Observed {
                    identity_matches: true,
                    current: None
                }
            ),
            ReconcileDecision::Unreadable
        );
    }

    #[test]
    fn reconciliation_events_carry_the_decision_into_the_state_machine() {
        // Crash after Prepared, current == desired: proceed to restoration.
        assert_eq!(
            transition(ActionState::Prepared, &Event::ReconcileWriteHappened),
            Ok(ActionState::RestorePending)
        );
        // Crash after Prepared, current == original: resolved, no OS call.
        assert_eq!(
            transition(ActionState::Prepared, &Event::ReconcileNoWrite),
            Ok(ActionState::Restored)
        );
        // Uncertain outcomes reconcile the same way.
        assert_eq!(
            transition(ActionState::Uncertain, &Event::ReconcileWriteHappened),
            Ok(ActionState::RestorePending)
        );
        assert_eq!(
            transition(ActionState::Uncertain, &Event::ReconcileNoWrite),
            Ok(ActionState::Restored)
        );
        assert_eq!(
            transition(ActionState::Uncertain, &Event::ExternalChange),
            Ok(ActionState::Conflict)
        );
        assert_eq!(
            transition(ActionState::Uncertain, &Event::TargetVanished),
            Ok(ActionState::TargetGone)
        );
    }

    #[test]
    fn unreadable_original_has_no_restore_path() {
        // An original that was never readable has no restore path either:
        // NoWriteOccurred when the value equals desired-for-original... the
        // decision only compares against what the record knows.
        let unreadable_original = intent(None, 0x4000);
        assert_eq!(
            reconcile(
                &unreadable_original,
                &Observed {
                    identity_matches: true,
                    current: Some(0x20)
                }
            ),
            ReconcileDecision::ExternalConflict
        );
    }

    #[test]
    fn pinning_follows_recovery_needs() {
        for state in [
            ActionState::Prepared,
            ActionState::Applied,
            ActionState::RestorePending,
            ActionState::Conflict,
            ActionState::Uncertain,
        ] {
            assert!(state.is_pinned(false), "{state:?} must be pinned");
            assert!(state.is_unresolved(false), "{state:?} must be unresolved");
        }
        assert!(!ActionState::Restored.is_pinned(false));
        assert!(!ActionState::TargetGone.is_pinned(false));
        // Failed without retained recovery is resolved; with retention it
        // is pinned.
        assert!(!ActionState::Failed.is_pinned(false));
        assert!(ActionState::Failed.is_pinned(true));
    }

    #[test]
    fn retention_prunes_only_expired_terminal_records() {
        let now = 1_800_000_000_000;
        let window = 30 * 24 * 3600 * 1000;
        assert_eq!(
            retention(
                ActionState::Restored,
                false,
                Some(now - window - 1),
                now,
                window
            ),
            Retention::Prunable
        );
        assert_eq!(
            retention(
                ActionState::Restored,
                false,
                Some(now - window + 1),
                now,
                window
            ),
            Retention::Keep
        );
        // Pinned records never prune, however old.
        assert_eq!(
            retention(ActionState::RestorePending, false, Some(0), now, window),
            Retention::Keep
        );
        assert_eq!(
            retention(ActionState::Failed, true, Some(0), now, window),
            Retention::Keep
        );
        // A terminal record with no timestamp cannot be proven old.
        assert_eq!(
            retention(ActionState::TargetGone, false, None, now, window),
            Retention::Keep
        );
    }

    #[test]
    fn record_cap_blocks_instead_of_evicting() {
        // Under the cap: allowed.
        assert!(may_record_new(9, 3, 10));
        // At cap with only pinned records: blocked, never evicted.
        assert!(!may_record_new(10, 10, 10));
        // At cap with prunable terminal records: allowed (they make room).
        assert!(may_record_new(10, 7, 10));
        // Over cap (should not happen): still blocked.
        assert!(!may_record_new(11, 11, 10));
    }
}
