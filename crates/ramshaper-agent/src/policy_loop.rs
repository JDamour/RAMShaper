//! The agent's policy loop (G3 glue over RS-015–RS-018; contract 14 §6).
//!
//! Composes the pure stack on one ticker: startup recovery reconciles
//! unresolved journal records before anything else; safety observations
//! are evaluated through the RS-018 guardrails and trip the RS-017 restore
//! engine; ticks perform TTL-expiry and due-retry restores. The 2-second
//! verification target is **accounted, not assumed**: a trigger whose
//! verification is not terminal within 2 s counts one missed deadline —
//! never a fabricated success.
//!
//! Boundary: the loop only **restores**. It can act on records that exist
//! in the journal, and nothing in this build can create actions (no policy
//! IPC commands; the coordinator is not wired to the pipe). The loop is
//! additionally gated behind `PolicyLoopConfig::enabled` (default off);
//! startup recovery is the contract-mandated safety direction.

use std::collections::HashMap;

use ramshaper_core::coordinator::{
    ActionJournal, ActuatorError, Coordinator, JournalSnapshot, PriorityActuator, RestoreEngine,
    RestoreOutcome,
};
use ramshaper_core::guardrails::{evaluate, CompatibilityFinding, Decision, SafetyObservation};
use ramshaper_core::identity::ProcessIdentity;
use ramshaper_core::journal::{reconcile, ActionState, Event, Observed, ReconcileDecision};

/// Verification target for a restore trigger (contract 14 §6: "target
/// verification within 2 s of a restore trigger; record missed deadlines,
/// not fabricated success").
pub const VERIFICATION_TARGET_MS: u64 = 2_000;

/// Loop configuration. `enabled = false` (the default) makes every entry
/// point a no-op: the loop is wired but dormant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PolicyLoopConfig {
    /// Whether the loop acts (restores) at all. The agent gates this on an
    /// explicit startup flag; there is no IPC path that flips it.
    pub enabled: bool,
}

/// Counters surfaced for evidence (never fabricated: a missed deadline is
/// counted once per trigger, and restores are counted only from terminal
/// journal states).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PolicyLoopStats {
    /// Startup-recovery passes that resolved a record.
    pub startup_reconciled: u64,
    /// Safety trips performed.
    pub trips: u64,
    /// Terminal `Restored` records (any path).
    pub restored: u64,
    /// Terminal `Conflict` records (external change preserved).
    pub conflicts: u64,
    /// Terminal `TargetGone` records (zero writes to replacements).
    pub targets_gone: u64,
    /// Triggers whose verification exceeded [`VERIFICATION_TARGET_MS`],
    /// counted once each.
    pub missed_deadlines: u64,
}

/// One restore trigger awaiting terminal verification.
#[derive(Debug, Clone)]
struct PendingTrigger {
    action_id: String,
    fired_at_unix_ms: u64,
    /// Whether the >2 s miss has already been counted.
    miss_counted: bool,
}

/// The policy loop. Generic over the journal and actuator seams so tests
/// drive fakes; production wires the SQLite store and the Windows
/// actuator.
pub struct PolicyLoop<J: ActionJournal, A: PriorityActuator> {
    config: PolicyLoopConfig,
    journal: J,
    actuator: A,
    engine: RestoreEngine,
    #[allow(dead_code)] // mid-refactor: confirm moved to the gate; loop no longer reads it
    coordinator: Coordinator,
    stats: PolicyLoopStats,
    /// Trigger currently awaiting verification (one action at a time).
    pending: Option<PendingTrigger>,
    /// Due retries the engine scheduled (`RetryScheduled`): action id →
    /// (trigger to restate, when the attempt is allowed). A due time is
    /// inclusive (the schedule says when the attempt is allowed); entries
    /// clear on terminal outcomes, `ManualRetry` and `NeedsAttention`
    /// (those records wait for an external trigger, never a busy
    /// tick loop). TTL expiry has no in-memory mirror: the journal's
    /// `applied_deadline_unix_ms` is the single source of truth, so
    /// applies made through the gate need no bridge into the loop.
    retry_dues: HashMap<String, (ramshaper_core::coordinator::RestoreTrigger, u64)>,
    /// Compatibility findings recorded by child-spawned trips (AT-14:
    /// "incompatibility is recorded") — evidence for the RS-024 Stage B
    /// allowlist, never a verdict about the app.
    findings: Vec<CompatibilityFinding>,
}

impl<J: ActionJournal, A: PriorityActuator> PolicyLoop<J, A> {
    /// A loop under the given configuration.
    pub fn new(config: PolicyLoopConfig, journal: J, actuator: A) -> Self {
        Self {
            config,
            journal,
            actuator,
            engine: RestoreEngine::default(),
            coordinator: Coordinator::new(ramshaper_core::policy::PolicyRules::default_experiment()),
            stats: PolicyLoopStats::default(),
            pending: None,
            retry_dues: HashMap::new(),
            findings: Vec::new(),
        }
    }

    /// Counters so far.
    #[must_use]
    pub fn stats(&self) -> &PolicyLoopStats {
        &self.stats
    }

    /// Compatibility findings recorded by child-spawned trips (AT-14):
    /// evidence for the RS-024 Stage B allowlist, in record order.
    #[must_use]
    pub fn findings(&self) -> &[CompatibilityFinding] {
        &self.findings
    }

    /// The outstanding Applied record's exact identity, when one exists —
    /// the safety-feed seam: the agent observes children/focus of exactly
    /// this target and feeds the observations back through
    /// [`PolicyLoop::observe_safety`].
    #[must_use]
    pub fn outstanding_applied_identity(&self) -> Option<ProcessIdentity> {
        let action_id = self.outstanding_applied_action()?;
        self.identity_of(&action_id)
    }

    /// Whether the loop acts at all.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// Startup recovery (contract: "Restart restores outstanding records
    /// before enabling any new actions"). `Prepared`/`Uncertain` records
    /// reconcile from the current value (never reapply); `Applied` records
    /// restore; `RestorePending` records restore. Returns how many records
    /// were resolved.
    pub fn startup_recovery(&mut self, now_unix_ms: u64) -> usize {
        if !self.config.enabled {
            return 0;
        }
        let Ok(ids) = self.journal.unresolved_ids() else {
            return 0;
        };
        let mut resolved = 0;
        for action_id in ids {
            let Some(snapshot) = self.journal.record_snapshot(&action_id).ok().flatten() else {
                continue;
            };
            let Some(intent) = snapshot.intent else {
                continue;
            };
            let identity = ProcessIdentity::new(intent.pid, intent.creation_filetime);
            let outcome = match snapshot.state {
                ActionState::Applied | ActionState::RestorePending => self.engine.restore_once(
                    &self.journal,
                    &mut self.actuator,
                    &identity,
                    &action_id,
                    ramshaper_core::coordinator::RestoreTrigger::StartupRecovery,
                    now_unix_ms,
                ),
                ActionState::Prepared | ActionState::Uncertain => {
                    self.reconcile_unapplied(&identity, &action_id, snapshot.state, now_unix_ms)
                }
                _ => continue,
            };
            if self.account_terminal(
                &action_id,
                ramshaper_core::coordinator::RestoreTrigger::StartupRecovery,
                &outcome,
                now_unix_ms,
            ) {
                resolved += 1;
                self.stats.startup_reconciled += 1;
            }
        }
        resolved
    }

    /// Reconcile a `Prepared`/`Uncertain` record from reality (the AT-12
    /// rule): never reapply; restore only what the current value proves.
    fn reconcile_unapplied(
        &mut self,
        identity: &ProcessIdentity,
        action_id: &str,
        state: ActionState,
        now_unix_ms: u64,
    ) -> RestoreOutcome {
        let Some(snapshot) = self.journal.record_snapshot(action_id).ok().flatten() else {
            return RestoreOutcome::NotRestorable;
        };
        let Some(intent) = snapshot.intent.clone() else {
            return RestoreOutcome::StorageBlocked("record has no intent".to_string());
        };
        let observed = match self.actuator.read(identity) {
            Ok(value) => Observed {
                identity_matches: true,
                current: Some(value),
            },
            Err(ActuatorError::IdentityMismatch) => Observed {
                identity_matches: false,
                current: None,
            },
            Err(_) => Observed {
                identity_matches: true,
                current: None,
            },
        };
        match reconcile(&intent, &observed) {
            ReconcileDecision::WriteHappenedRestore => {
                let _ = self.journal.transition(
                    action_id,
                    state,
                    &Event::ReconcileWriteHappened,
                    now_unix_ms,
                );
                self.engine.restore_once(
                    &self.journal,
                    &mut self.actuator,
                    identity,
                    action_id,
                    ramshaper_core::coordinator::RestoreTrigger::StartupRecovery,
                    now_unix_ms,
                )
            }
            ReconcileDecision::NoWriteOccurred => {
                match self.journal.transition(
                    action_id,
                    state,
                    &Event::ReconcileNoWrite,
                    now_unix_ms,
                ) {
                    Ok(_) => RestoreOutcome::Restored { wrote_back: false },
                    Err(fault) => RestoreOutcome::StorageBlocked(fault.to_string()),
                }
            }
            ReconcileDecision::ExternalConflict => {
                match self
                    .journal
                    .transition(action_id, state, &Event::ExternalChange, now_unix_ms)
                {
                    Ok(_) => RestoreOutcome::Conflict,
                    Err(fault) => RestoreOutcome::StorageBlocked(fault.to_string()),
                }
            }
            ReconcileDecision::TargetGone => {
                match self
                    .journal
                    .transition(action_id, state, &Event::TargetVanished, now_unix_ms)
                {
                    Ok(_) => RestoreOutcome::TargetGone,
                    Err(fault) => RestoreOutcome::StorageBlocked(fault.to_string()),
                }
            }
            ReconcileDecision::Unreadable => RestoreOutcome::NotRestorable,
        }
    }

    /// Feed one safety observation (RS-018): trips restore the parent —
    /// the outstanding `Applied` record whose identity matches the
    /// observation's target, if any. Returns whether a trip was performed.
    pub fn observe_safety(&mut self, observation: &SafetyObservation, now_unix_ms: u64) -> bool {
        if !self.config.enabled {
            return false;
        }
        let decision = evaluate(observation, now_unix_ms);
        let Decision::Trip { trigger, finding } = decision else {
            return false;
        };
        // Map the trigger to the outstanding Applied record (the one-action
        // cap means at most one).
        let Some(action_id) = self.outstanding_applied_action() else {
            return false;
        };
        // AT-14: "incompatibility is recorded" — the finding is evidence
        // for the Stage B allowlist; the parent label comes from the
        // record's own intent (evaluate leaves it to the agent).
        if let Some(mut finding) = finding {
            if let Some(snapshot) = self.journal.record_snapshot(&action_id).ok().flatten() {
                finding.parent_image = snapshot
                    .intent
                    .map(|intent| intent.target_name)
                    .unwrap_or_default();
            }
            self.findings.push(finding);
        }
        let Some(identity) = self.identity_of(&action_id) else {
            return false;
        };
        self.stats.trips += 1;
        self.begin_verification(&action_id, now_unix_ms);
        let trigger = match trigger {
            ramshaper_core::guardrails::TripTrigger::ChildSpawned => {
                ramshaper_core::coordinator::RestoreTrigger::StaleSafetyInput
            }
            ramshaper_core::guardrails::TripTrigger::ForegroundReturned => {
                ramshaper_core::coordinator::RestoreTrigger::ForegroundReturned
            }
            ramshaper_core::guardrails::TripTrigger::SessionLock => {
                ramshaper_core::coordinator::RestoreTrigger::SessionLock
            }
            ramshaper_core::guardrails::TripTrigger::StaleSafetyInput => {
                ramshaper_core::coordinator::RestoreTrigger::StaleSafetyInput
            }
            ramshaper_core::guardrails::TripTrigger::SystemResumed => {
                // The agent can only observe the RESUME (it was not
                // scheduled during the sleep); the contract's restore
                // reason for the journal is the suspend that caused it.
                ramshaper_core::coordinator::RestoreTrigger::Suspend
            }
        };
        let outcome = self.engine.restore_once(
            &self.journal,
            &mut self.actuator,
            &identity,
            &action_id,
            trigger,
            now_unix_ms,
        );
        let _ = self.account_terminal(&action_id, trigger, &outcome, now_unix_ms);
        matches!(
            outcome,
            RestoreOutcome::Restored { .. }
                | RestoreOutcome::Conflict
                | RestoreOutcome::TargetGone
                | RestoreOutcome::RetryScheduled { .. }
                | RestoreOutcome::ManualRetry
                | RestoreOutcome::NeedsAttention
        )
    }

    /// One ticker pass: TTL expiry, due retries and missed-deadline
    /// accounting. Cheap when nothing is pending.
    pub fn tick(&mut self, now_unix_ms: u64) {
        if !self.config.enabled {
            return;
        }
        // TTL expiry: every Applied record past its deadline restores. The
        // journal is the source of truth — applies made through the gate's
        // own coordinator (the production path) are seen here without any
        // bridge, exactly like startup recovery sees them.
        let expired: Vec<(String, ProcessIdentity)> = match self.journal.unresolved_ids() {
            Ok(ids) => ids
                .into_iter()
                .filter_map(|action_id| {
                    let snapshot = self.journal.record_snapshot(&action_id).ok().flatten()?;
                    if snapshot.state != ActionState::Applied {
                        return None;
                    }
                    let deadline = snapshot.applied_deadline_unix_ms?;
                    if deadline >= now_unix_ms {
                        return None;
                    }
                    let intent = snapshot.intent?;
                    Some((
                        action_id,
                        ProcessIdentity::new(intent.pid, intent.creation_filetime),
                    ))
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        for (action_id, identity) in expired {
            self.begin_verification(&action_id, now_unix_ms);
            let outcome = self.engine.restore_once(
                &self.journal,
                &mut self.actuator,
                &identity,
                &action_id,
                ramshaper_core::coordinator::RestoreTrigger::TtlExpired,
                now_unix_ms,
            );
            let _ = self.account_terminal(
                &action_id,
                ramshaper_core::coordinator::RestoreTrigger::TtlExpired,
                &outcome,
                now_unix_ms,
            );
        }
        // Due retries: re-attempt the engine's bounded schedule (1 s / 5 s
        // / 30 s) at exactly the due time — inclusive, because the due
        // value is when the attempt becomes allowed; an earlier call
        // restates the schedule instead of writing.
        let due: Vec<(
            String,
            ramshaper_core::coordinator::RestoreTrigger,
            ProcessIdentity,
        )> = self
            .retry_dues
            .iter()
            .filter(|(_, (_, due))| *due <= now_unix_ms)
            .map(|(id, (trigger, _))| (id.clone(), *trigger))
            .filter_map(|(id, trigger)| {
                self.identity_of(&id)
                    .map(|identity| (id, trigger, identity))
            })
            .collect();
        for (action_id, trigger, identity) in due {
            self.retry_dues.remove(&action_id);
            let outcome = self.engine.restore_once(
                &self.journal,
                &mut self.actuator,
                &identity,
                &action_id,
                trigger,
                now_unix_ms,
            );
            let _ = self.account_terminal(&action_id, trigger, &outcome, now_unix_ms);
        }
        // Missed-deadline accounting: one count per trigger, never per
        // tick, and never in place of a real outcome.
        if let Some(pending) = &mut self.pending {
            if !pending.miss_counted
                && now_unix_ms.saturating_sub(pending.fired_at_unix_ms) > VERIFICATION_TARGET_MS
            {
                self.stats.missed_deadlines += 1;
                pending.miss_counted = true;
            }
        }
    }

    fn outstanding_applied_action(&self) -> Option<String> {
        let ids = self.journal.unresolved_ids().ok()?;
        ids.into_iter().find(|action_id| {
            matches!(
                self.journal.record_snapshot(action_id).ok().flatten(),
                Some(snapshot) if snapshot.state == ActionState::Applied
            )
        })
    }

    fn identity_of(&self, action_id: &str) -> Option<ProcessIdentity> {
        let snapshot: JournalSnapshot = self.journal.record_snapshot(action_id).ok()??;
        let intent = snapshot.intent?;
        Some(ProcessIdentity::new(intent.pid, intent.creation_filetime))
    }

    fn begin_verification(&mut self, action_id: &str, now_unix_ms: u64) {
        self.pending.get_or_insert(PendingTrigger {
            action_id: action_id.to_string(),
            fired_at_unix_ms: now_unix_ms,
            miss_counted: false,
        });
    }

    /// Record an outcome: stats + verification and retry-schedule
    /// accounting. Returns true when the outcome was terminal for the
    /// record. `RetryScheduled` records the next allowed attempt;
    /// `ManualRetry`/`NeedsAttention` clear any schedule (the record waits
    /// for an external trigger, never a busy tick loop).
    fn account_terminal(
        &mut self,
        action_id: &str,
        trigger: ramshaper_core::coordinator::RestoreTrigger,
        outcome: &RestoreOutcome,
        now_unix_ms: u64,
    ) -> bool {
        let terminal = matches!(
            outcome,
            RestoreOutcome::Restored { .. } | RestoreOutcome::Conflict | RestoreOutcome::TargetGone
        );
        match outcome {
            RestoreOutcome::Restored { .. } => self.stats.restored += 1,
            RestoreOutcome::Conflict => self.stats.conflicts += 1,
            RestoreOutcome::TargetGone => self.stats.targets_gone += 1,
            RestoreOutcome::RetryScheduled { due_unix_ms } => {
                self.retry_dues
                    .insert(action_id.to_string(), (trigger, *due_unix_ms));
            }
            RestoreOutcome::ManualRetry | RestoreOutcome::NeedsAttention => {
                self.retry_dues.remove(action_id);
            }
            _ => {}
        }
        if terminal {
            self.retry_dues.remove(action_id);
            if let Some(pending) = self.pending.take() {
                if pending.action_id == action_id
                    && !pending.miss_counted
                    && now_unix_ms.saturating_sub(pending.fired_at_unix_ms) > VERIFICATION_TARGET_MS
                {
                    // Counted at closure if it was missed and not yet
                    // counted by a tick.
                    self.stats.missed_deadlines += 1;
                }
            }
            self.retry_dues.remove(action_id);
        }
        terminal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramshaper_core::coordinator::JournalFault;
    use ramshaper_core::journal::{ActionIntent, Property, ACTION_KIND_CPU_PRIORITY};
    use std::cell::RefCell;

    fn identity(pid: u32) -> ProcessIdentity {
        ProcessIdentity::new(pid, 133_000_000_000_000_000 + u64::from(pid))
    }

    fn intent_for(pid: u32) -> ActionIntent {
        ActionIntent {
            action_id: format!("act-{pid}"),
            agent_instance_id: "agent".into(),
            pid,
            creation_filetime: 133_000_000_000_000_000 + u64::from(pid),
            target_name: "app.exe".into(),
            property: Property {
                kind: ACTION_KIND_CPU_PRIORITY,
                original: Some(0x20),
                desired: 0x4000,
            },
            consent_id: "c".into(),
            rule_version: "policy_v1".into(),
            ttl_ms: 120_000,
            created_at_unix_ms: 1_000,
        }
    }

    #[derive(Default)]
    struct FakeJournal {
        records: RefCell<Vec<(ActionIntent, ActionState, Option<u64>)>>,
    }

    impl FakeJournal {
        fn seed_applied(&self, pid: u32, deadline: u64) {
            self.records
                .borrow_mut()
                .push((intent_for(pid), ActionState::Applied, Some(deadline)));
        }
    }

    impl ActionJournal for FakeJournal {
        fn available(&self) -> bool {
            true
        }
        fn record_prepared(&self, intent: &ActionIntent) -> Result<bool, JournalFault> {
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
        fn record_original(&self, action_id: &str, value: u32) -> Result<(), JournalFault> {
            let mut records = self.records.borrow_mut();
            let record = records
                .iter_mut()
                .find(|(i, state, _)| i.action_id == action_id && *state == ActionState::Prepared)
                .ok_or_else(|| JournalFault::StateConflict(action_id.to_string()))?;
            record.0.property.original = Some(value);
            Ok(())
        }
        fn record_snapshot(
            &self,
            action_id: &str,
        ) -> Result<Option<JournalSnapshot>, JournalFault> {
            Ok(self
                .records
                .borrow()
                .iter()
                .find(|(i, _, _)| i.action_id == action_id)
                .map(|(intent, state, deadline)| JournalSnapshot {
                    state: *state,
                    applied_deadline_unix_ms: *deadline,
                    intent: Some(intent.clone()),
                }))
        }
        fn unresolved_ids(&self) -> Result<Vec<String>, JournalFault> {
            Ok(self
                .records
                .borrow()
                .iter()
                .filter(|(_, state, _)| state.is_unresolved(false))
                .map(|(i, _, _)| i.action_id.clone())
                .collect())
        }
        fn transition(
            &self,
            action_id: &str,
            expected: ActionState,
            event: &Event,
            _now: u64,
        ) -> Result<ActionState, JournalFault> {
            let mut records = self.records.borrow_mut();
            let Some(pos) = records
                .iter()
                .position(|(i, _, _)| i.action_id == action_id)
            else {
                return Err(JournalFault::StateConflict(action_id.into()));
            };
            let (_, state, deadline) = &mut records[pos];
            if *state != expected {
                return Err(JournalFault::StateConflict(action_id.into()));
            }
            let next = ramshaper_core::journal::transition(*state, event)
                .map_err(|e| JournalFault::StateConflict(e.to_string()))?;
            if let Event::ApplyVerified { deadline_unix_ms } = event {
                *deadline = Some(*deadline_unix_ms);
            }
            *state = next;
            Ok(next)
        }
    }

    /// Value-holding actuator with a scripted read-failure mode.
    struct FakeActuator {
        value: RefCell<u32>,
        transient_reads: RefCell<u32>,
    }

    impl FakeActuator {
        fn new(value: u32) -> Self {
            Self {
                value: RefCell::new(value),
                transient_reads: RefCell::new(0),
            }
        }
    }

    impl PriorityActuator for FakeActuator {
        fn write_verified(
            &mut self,
            _identity: &ProcessIdentity,
            desired: u32,
        ) -> Result<u32, ActuatorError> {
            *self.value.borrow_mut() = desired;
            Ok(desired)
        }
        fn read(&self, _identity: &ProcessIdentity) -> Result<u32, ActuatorError> {
            let pending = *self.transient_reads.borrow();
            if pending > 0 {
                *self.transient_reads.borrow_mut() -= 1;
                return Err(ActuatorError::OsFailure {
                    code: 87,
                    function: "GetPriorityClass",
                });
            }
            Ok(*self.value.borrow())
        }
    }

    #[test]
    fn disabled_loop_is_a_noop_everywhere() {
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500);
        let mut loop_ = PolicyLoop::new(
            PolicyLoopConfig { enabled: false },
            journal,
            FakeActuator::new(0x4000),
        );
        assert!(!loop_.observe_safety(&SafetyObservation::ForegroundReturned, 1_000,));
        loop_.tick(1_000_000);
        assert_eq!(loop_.startup_recovery(1_000_000), 0);
        assert_eq!(loop_.stats().restored, 0);
    }

    #[test]
    fn startup_recovery_reconciles_without_reapplying() {
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500);
        // The value is already back at the original: nothing to undo.
        let mut loop_ = PolicyLoop::new(
            PolicyLoopConfig { enabled: true },
            journal,
            FakeActuator::new(0x20),
        );
        assert_eq!(loop_.startup_recovery(1_000), 1);
        assert_eq!(loop_.stats().restored, 1);
        assert_eq!(loop_.stats().startup_reconciled, 1);
    }

    #[test]
    fn startup_recovery_restores_a_live_write() {
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500);
        // The write is live (0x4000): startup must put 0x20 back.
        let mut loop_ = PolicyLoop::new(
            PolicyLoopConfig { enabled: true },
            journal,
            FakeActuator::new(0x4000),
        );
        assert_eq!(loop_.startup_recovery(1_000), 1);
        assert_eq!(loop_.stats().restored, 1);
    }

    #[test]
    fn safety_trip_restores_the_parent() {
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500_000);
        let mut loop_ = PolicyLoop::new(
            PolicyLoopConfig { enabled: true },
            journal,
            FakeActuator::new(0x4000),
        );
        let tripped = loop_.observe_safety(
            &SafetyObservation::ChildSpawned {
                parent: identity(4242),
                child: identity(5000),
                child_image: "updater.exe".into(),
            },
            1_000,
        );
        assert!(tripped);
        assert_eq!(loop_.stats().trips, 1);
        assert_eq!(loop_.stats().restored, 1);
        assert!(
            loop_.stats().missed_deadlines == 0,
            "a same-tick terminal restore is within the 2 s target"
        );
    }

    #[test]
    fn ttl_tick_restores_an_expired_action() {
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500);
        let mut loop_ = PolicyLoop::new(
            PolicyLoopConfig { enabled: true },
            journal,
            FakeActuator::new(0x4000),
        );
        // No note_applied bridge: the deadline comes from the journal
        // snapshot (the gate's applies are seen the same way).
        loop_.tick(501);
        assert_eq!(loop_.stats().restored, 1);
    }

    #[test]
    fn missed_deadline_counts_once_and_never_fakes_success() {
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500_000);
        let actuator = FakeActuator::new(0x4000);
        // Two transient read failures spread over >2 s of loop time.
        *actuator.transient_reads.borrow_mut() = 2;
        let mut loop_ = PolicyLoop::new(PolicyLoopConfig { enabled: true }, journal, actuator);
        loop_.observe_safety(&SafetyObservation::ForegroundReturned, 1_000);
        assert_eq!(loop_.stats().missed_deadlines, 0, "not yet past 2 s");
        loop_.tick(2_000); // +1 s: still inside the target
        assert_eq!(loop_.stats().missed_deadlines, 0);
        loop_.tick(3_500); // +2.5 s: past the target; one miss
        assert_eq!(loop_.stats().missed_deadlines, 1);
        loop_.tick(5_000); // repeats must not double-count
        assert_eq!(loop_.stats().missed_deadlines, 1);
        // The schedule's third attempt (due at first+1s+5s = 7_000) lands:
        // reads work again, restore completes honestly.
        loop_.tick(7_000);
        assert_eq!(loop_.stats().restored, 1);
        assert_eq!(loop_.stats().missed_deadlines, 1, "a miss stays counted");
    }

    #[test]
    fn child_spawned_trip_restores_and_records_the_finding() {
        // AT-14: the parent restores and the incompatibility is recorded —
        // the finding carries the child identity plus the parent label
        // from the record's intent (evaluate leaves it to the agent).
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500_000);
        let mut loop_ = PolicyLoop::new(
            PolicyLoopConfig { enabled: true },
            journal,
            FakeActuator::new(0x4000),
        );
        let parent = ProcessIdentity::new(4242, 133_000_000_000_000_000 + 4242);
        let child = ProcessIdentity::new(5151, 133_000_000_000_000_000 + 5151);
        let tripped = loop_.observe_safety(
            &SafetyObservation::ChildSpawned {
                parent,
                child,
                child_image: "helper.exe".to_string(),
            },
            1_000,
        );
        assert!(tripped);
        assert_eq!(loop_.stats().restored, 1);
        let findings = loop_.findings();
        assert_eq!(findings.len(), 1, "exactly one finding: {findings:?}");
        assert_eq!(findings[0].parent_image, "app.exe", "the record's label");
        assert_eq!(findings[0].child_image, "helper.exe");
        assert_eq!(findings[0].child_pid, 5151);
        assert_eq!(
            findings[0].child_creation_filetime,
            child.creation_filetime_100ns
        );
        assert_eq!(findings[0].recorded_at_unix_ms, 1_000);
        // The no-descendant-write rule holds by construction: the loop's
        // vocabulary has no path that writes to the child.
    }

    #[test]
    fn focus_stale_beyond_the_freshness_window_trips() {
        // RS-018's stale-input trip: focus evidence older than the apply
        // freshness window (2 s) invalidates the experiment.
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500_000);
        let mut loop_ = PolicyLoop::new(
            PolicyLoopConfig { enabled: true },
            journal,
            FakeActuator::new(0x4000),
        );
        let tripped = loop_.observe_safety(&SafetyObservation::FocusStale { age_ms: 3_000 }, 1_000);
        assert!(tripped);
        assert_eq!(loop_.stats().restored, 1);
        assert!(
            loop_.findings().is_empty(),
            "stale focus is a safety trip, not a compatibility finding"
        );
    }

    #[test]
    fn session_lock_trips_the_restore() {
        // Contract 14 §6: a session lock initiates restoration (the user
        // is walking away). The adapter feeds SessionLocked only on a
        // positive Locked WTS state; this pins the loop side of that path.
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500_000);
        let mut loop_ = PolicyLoop::new(
            PolicyLoopConfig { enabled: true },
            journal,
            FakeActuator::new(0x4000),
        );
        let tripped = loop_.observe_safety(&SafetyObservation::SessionLocked, 1_000);
        assert!(tripped);
        assert_eq!(loop_.stats().restored, 1);
        assert!(loop_.findings().is_empty());
    }

    #[test]
    fn system_resume_trips_the_suspend_restore() {
        // Contract 14 §6: a suspend notification initiates restoration.
        // The agent observes the resume (interrupt-time divergence in the
        // ticker) and the journal records the Suspend reason.
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500_000);
        let mut loop_ = PolicyLoop::new(
            PolicyLoopConfig { enabled: true },
            journal,
            FakeActuator::new(0x4000),
        );
        let tripped = loop_.observe_safety(
            &SafetyObservation::SystemResumed { sleep_ms: 45_678 },
            1_000,
        );
        assert!(tripped);
        assert_eq!(loop_.stats().restored, 1);
        assert!(loop_.findings().is_empty());
        // And with nothing outstanding, a resume observation is refused
        // honestly (no restore fabricated).
        let retripped =
            loop_.observe_safety(&SafetyObservation::SystemResumed { sleep_ms: 12 }, 2_000);
        assert!(!retripped);
    }

    #[test]
    fn a_second_restore_engine_on_a_terminal_record_writes_nothing() {
        // RS-026 round-2 probe (gate-disable vs ticker interleave): two
        // engines over one journal are the shipped shape (the gate's
        // clean-exit disable + the ticker's pass). The first restore
        // transitions the record to a terminal state, so the second finds
        // NotRestorable — no second write, no error.
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500_000);
        let mut actuator = FakeActuator::new(0x4000);
        let identity = ProcessIdentity::new(4242, 133_000_000_000_000_000 + 4242);
        let mut first = RestoreEngine::default();
        let outcome = first.restore_once(
            &journal,
            &mut actuator,
            &identity,
            "act-4242",
            ramshaper_core::coordinator::RestoreTrigger::Disable,
            1_000,
        );
        assert!(
            matches!(outcome, RestoreOutcome::Restored { .. }),
            "{outcome:?}"
        );
        let mut second = RestoreEngine::default();
        let outcome = second.restore_once(
            &journal,
            &mut actuator,
            &identity,
            "act-4242",
            ramshaper_core::coordinator::RestoreTrigger::ForegroundReturned,
            2_000,
        );
        assert_eq!(outcome, RestoreOutcome::NotRestorable);
    }

    #[test]
    fn an_applied_record_cannot_survive_startup_recovery() {
        // RS-026 round-2 probe (FocusStale-at-startup reachability): the
        // feed fires staleness only when an APPLIED record is outstanding;
        // a restore that fails during startup recovery must land the
        // record in RestorePending (manual retry), never leave it Applied
        // — so the stale trip cannot misfire on recovery residue.
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500_000);
        let actuator = FakeActuator::new(0x4000);
        // Reads never succeed during recovery: every retry path is taken.
        *actuator.transient_reads.borrow_mut() = u32::MAX;
        let mut loop_ = PolicyLoop::new(PolicyLoopConfig { enabled: true }, journal, actuator);
        let resolved = loop_.startup_recovery(1_000);
        assert_eq!(resolved, 0, "an unreadable target cannot be resolved");
        assert_eq!(
            loop_.outstanding_applied_identity(),
            None,
            "no Applied record may survive startup recovery"
        );
    }

    #[test]
    fn outstanding_applied_identity_serves_the_safety_feed() {
        let journal = FakeJournal::default();
        journal.seed_applied(4242, 500_000);
        let mut loop_ = PolicyLoop::new(
            PolicyLoopConfig { enabled: true },
            journal,
            FakeActuator::new(0x4000),
        );
        assert_eq!(
            loop_.outstanding_applied_identity(),
            Some(ProcessIdentity::new(4242, 133_000_000_000_000_000 + 4242))
        );
        // After a terminal restore there is nothing outstanding to feed.
        loop_.observe_safety(&SafetyObservation::ForegroundReturned, 1_000);
        assert_eq!(loop_.outstanding_applied_identity(), None);
    }
}
