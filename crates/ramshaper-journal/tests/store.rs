//! Journal store tests: durable prepare, guarded transitions, replay
//! idempotency, schema refusal, pinning and quota — with a fake actuator
//! driving the AT-12 flows. No OS process is touched: the fake actuator is
//! an in-memory property value plus an identity flag, exactly the surface
//! the contract's state machine reasons over.

#![deny(unsafe_code)]

use ramshaper_core::journal::{
    reconcile, ActionIntent, ActionState, Event, Observed, Property, ACTION_KIND_CPU_PRIORITY,
};
use ramshaper_journal::{JournalStore, Recorded};

fn intent(id: &str) -> ActionIntent {
    ActionIntent {
        action_id: id.to_string(),
        agent_instance_id: "agent-1".to_string(),
        pid: 4242,
        creation_filetime: 133_000_000_000_000_000,
        target_name: "app.exe".to_string(),
        property: Property {
            kind: ACTION_KIND_CPU_PRIORITY,
            original: Some(0x20), // Normal
            desired: 0x4000,      // Below Normal
        },
        consent_id: "consent-1".to_string(),
        rule_version: "policy_v1".to_string(),
        ttl_ms: 120_000,
        created_at_unix_ms: 1_798_000_000_000,
    }
}

/// Fake actuator: the observable surface of one target process.
#[derive(Default)]
struct FakeTarget {
    value: Option<u32>,
    identity_matches: bool,
    writes: Vec<u32>,
}

impl FakeTarget {
    fn new() -> Self {
        Self {
            value: Some(0x20),
            identity_matches: true,
            writes: Vec::new(),
        }
    }

    /// Startup reconciliation: decide from reality, apply the decision.
    fn reconcile_and_restore(
        &mut self,
        store: &JournalStore,
        record_id: &str,
        expected: ActionState,
        now: u64,
    ) -> Result<ActionState, ramshaper_journal::JournalError> {
        let record = store.get(record_id)?.expect("record exists");
        let decision = reconcile(
            &record.intent,
            &Observed {
                identity_matches: self.identity_matches,
                current: self.value,
            },
        );
        use ramshaper_core::journal::ReconcileDecision::*;
        match decision {
            WriteHappenedRestore => {
                // Move to RestorePending, write the ORIGINAL back (never a
                // re-apply of desired), verify, resolve.
                store.transition(
                    record_id,
                    expected,
                    &Event::ReconcileWriteHappened,
                    now,
                    serde_json::json!({"reconcile": "write-happened"}),
                )?;
                let original = record.intent.property.original;
                if let Some(original) = original {
                    self.value = Some(original);
                    self.writes.push(original);
                }
                store.transition(
                    record_id,
                    ActionState::RestorePending,
                    &Event::RestoreVerified,
                    now,
                    serde_json::json!({"restored": true, "writes": self.writes.len()}),
                )
            }
            NoWriteOccurred => {
                // No OS call at all: the record resolves by observation.
                store.transition(
                    record_id,
                    expected,
                    &Event::ReconcileNoWrite,
                    now,
                    serde_json::json!({"reconcile": "no-write"}),
                )
            }
            ExternalConflict => store.transition(
                record_id,
                expected,
                &Event::ExternalChange,
                now,
                serde_json::json!({"reconcile": "external"}),
            ),
            TargetGone => store.transition(
                record_id,
                expected,
                &Event::TargetVanished,
                now,
                serde_json::json!({"reconcile": "target-gone"}),
            ),
            Unreadable => store.transition(
                record_id,
                expected,
                &Event::ApplyUncertain,
                now,
                serde_json::json!({"reconcile": "unreadable"}),
            ),
        }
    }
}

#[test]
fn happy_path_prepare_apply_restore_round_trips_durably() {
    let store = JournalStore::open_in_memory().expect("journal");
    let mut target = FakeTarget::new();
    let now = 1_798_000_000_000u64;

    // Prepare: durable before the OS call.
    assert_eq!(
        store.record_prepared(&intent("act-1")).expect("prepare"),
        Recorded::Created
    );
    assert_eq!(
        store.get("act-1").expect("store").expect("record").state,
        ActionState::Prepared
    );

    // Replay: returns the existing record, never extends or duplicates.
    match store.record_prepared(&intent("act-1")).expect("replay") {
        Recorded::Existing(existing) => {
            assert_eq!(existing.intent.ttl_ms, 120_000, "TTL not extended");
            assert_eq!(existing.state, ActionState::Prepared);
        }
        Recorded::Created => panic!("replay must not create a second record"),
    }

    // Apply: write desired, read back, record.
    target.value = Some(0x4000);
    target.writes.push(0x4000);
    let deadline = now + 120_000;
    assert_eq!(
        store
            .transition(
                "act-1",
                ActionState::Prepared,
                &Event::ApplyVerified {
                    deadline_unix_ms: deadline
                },
                now,
                serde_json::json!({"read_back": 0x4000})
            )
            .expect("apply"),
        ActionState::Applied
    );
    let record = store.get("act-1").expect("store").expect("record");
    assert_eq!(record.applied_deadline_unix_ms, Some(deadline));

    // Restore trigger, then verified restoration of the ORIGINAL.
    store
        .transition(
            "act-1",
            ActionState::Applied,
            &Event::RestoreRequested,
            now + 5_000,
            serde_json::json!({"trigger": "ttl"}),
        )
        .expect("trigger");
    target.value = Some(0x20);
    target.writes.push(0x20);
    assert_eq!(
        store
            .transition(
                "act-1",
                ActionState::RestorePending,
                &Event::RestoreVerified,
                now + 6_000,
                serde_json::json!({"read_back": 0x20})
            )
            .expect("restore"),
        ActionState::Restored
    );
    // Terminal, unpinned, prunable after the retention window.
    let record = store.get("act-1").expect("store").expect("record");
    assert!(!record.is_pinned());
    assert_eq!(store.unresolved().expect("journal op").len(), 0);
    assert_eq!(
        store
            .prune(now + 6_000 + 31 * 24 * 3600 * 1000, 30 * 24 * 3600 * 1000)
            .expect("journal op"),
        1
    );
    assert!(store.get("act-1").expect("journal op").is_none());
}

/// AT-12 core: crash right after Prepared (write may or may not have
/// happened). Startup reconciles from the CURRENT value and never reapplies
/// the desired value.
#[test]
fn crash_after_prepared_reconciles_from_current_value_never_reapplies() {
    // Case A: the write happened before the crash.
    let store = JournalStore::open_in_memory().expect("journal");
    let mut target = FakeTarget::new();
    target.value = Some(0x4000); // write happened, then crash
    store.record_prepared(&intent("act-a")).expect("prepare");
    let next = target
        .reconcile_and_restore(&store, "act-a", ActionState::Prepared, 1)
        .expect("reconcile");
    assert_eq!(next, ActionState::Restored);
    assert_eq!(
        target.writes,
        vec![0x20],
        "only the ORIGINAL is written back"
    );
    assert_eq!(store.unresolved().expect("journal op").len(), 0);

    // Case B: the write never happened.
    let store = JournalStore::open_in_memory().expect("journal");
    let mut target = FakeTarget::new(); // still 0x20
    store.record_prepared(&intent("act-b")).expect("prepare");
    let next = target
        .reconcile_and_restore(&store, "act-b", ActionState::Prepared, 1)
        .expect("reconcile");
    assert_eq!(next, ActionState::Restored);
    assert!(
        target.writes.is_empty(),
        "no OS call at all when nothing changed"
    );

    // Case C: someone else changed the value during the crash window.
    let store = JournalStore::open_in_memory().expect("journal");
    let mut target = FakeTarget::new();
    target.value = Some(0x80); // external writer
    store.record_prepared(&intent("act-c")).expect("prepare");
    let next = target
        .reconcile_and_restore(&store, "act-c", ActionState::Prepared, 1)
        .expect("reconcile");
    assert_eq!(next, ActionState::Conflict);
    assert!(
        target.writes.is_empty(),
        "external change preserved untouched"
    );
    assert_eq!(
        store.unresolved().expect("journal op").len(),
        1,
        "Conflict stays pinned"
    );
}

/// AT-03/AT-12: a reused PID is never written to.
#[test]
fn reused_pid_receives_zero_writes() {
    let store = JournalStore::open_in_memory().expect("journal");
    let mut target = FakeTarget::new();
    target.value = Some(0x4000); // write happened
    store.record_prepared(&intent("act-r")).expect("prepare");
    // The identity check fails: PID reused by a new creation.
    target.identity_matches = false;
    let next = target
        .reconcile_and_restore(&store, "act-r", ActionState::Prepared, 1)
        .expect("reconcile");
    assert_eq!(next, ActionState::TargetGone);
    assert!(target.writes.is_empty(), "replacement receives zero writes");
}

#[test]
fn optimistic_state_guard_rejects_moved_records() {
    let store = JournalStore::open_in_memory().expect("journal");
    store.record_prepared(&intent("act-g")).expect("prepare");
    // Caller assumes Prepared but the record already advanced.
    store
        .transition(
            "act-g",
            ActionState::Prepared,
            &Event::ApplyUncertain,
            1,
            serde_json::json!({}),
        )
        .expect("first");
    let err = store
        .transition(
            "act-g",
            ActionState::Prepared,
            &Event::ApplyVerified {
                deadline_unix_ms: 2,
            },
            2,
            serde_json::json!({}),
        )
        .expect_err("must fail");
    assert!(matches!(
        err,
        ramshaper_journal::JournalError::StateConflict { .. }
    ));
    // Missing records are explicit.
    assert!(store.get("nope").expect("journal op").is_none());
}

#[test]
fn pinned_records_survive_pruning_and_block_at_quota() {
    let store = JournalStore::open_in_memory().expect("journal");
    let now = 1_798_000_000_000u64;
    // One terminal record, ancient.
    store.record_prepared(&intent("old-1")).expect("prepare");
    store
        .transition(
            "old-1",
            ActionState::Prepared,
            &Event::ReconcileNoWrite,
            1,
            serde_json::json!({}),
        )
        .expect("terminal");
    // One pinned Conflict, equally ancient.
    store.record_prepared(&intent("pin-1")).expect("prepare");
    store
        .transition(
            "pin-1",
            ActionState::Prepared,
            &Event::ExternalChange,
            1,
            serde_json::json!({}),
        )
        .expect("conflict");
    // Prune: only the terminal one goes.
    assert_eq!(store.prune(now, 1).expect("journal op"), 1);
    assert!(store.get("old-1").expect("journal op").is_none());
    assert!(
        store.get("pin-1").expect("journal op").is_some(),
        "pinned never pruned"
    );
    // Quota: the pinned record blocks at cap via the pure decision.
    let (total, pinned) = store.counts().expect("journal op");
    assert_eq!((total, pinned), (1, 1));
    assert!(!ramshaper_core::journal::may_record_new(total, pinned, 1));
}

#[test]
fn schema_version_mismatch_refuses_and_leaves_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("journal.db");
    {
        let store = JournalStore::open(&path).expect("fresh");
        store.record_prepared(&intent("a")).expect("prepare");
    }
    // Simulate a future build's journal: bump user_version.
    {
        let conn = rusqlite::Connection::open(&path).expect("journal op");
        conn.execute_batch("PRAGMA user_version = 2;")
            .expect("journal op");
    }
    let err = JournalStore::open(&path).expect_err("must fail");
    assert!(matches!(
        err,
        ramshaper_journal::JournalError::SchemaMismatch { found: 2, .. }
    ));
    // Refused open must not have modified the file: record and version intact.
    {
        let conn = rusqlite::Connection::open(&path).expect("journal op");
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("journal op");
        assert_eq!(v, 2, "journal left untouched");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM actions", [], |r| r.get(0))
            .expect("journal op");
        assert_eq!(n, 1, "records left untouched");
    }
    // A non-database file is Corrupt, also untouched.
    let junk = dir.path().join("junk.db");
    std::fs::write(&junk, b"this is not a database").expect("journal op");
    assert!(matches!(
        JournalStore::open(&junk).expect_err("must fail"),
        ramshaper_journal::JournalError::Corrupt(_)
    ));
    assert_eq!(
        std::fs::read(&junk).expect("journal op"),
        b"this is not a database"
    );
}

/// Durability across "restarts": reopen the same file and reconcile.
#[test]
fn records_survive_reopen_for_startup_recovery() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("journal.db");
    {
        let store = JournalStore::open(&path).expect("open");
        store.record_prepared(&intent("act-d")).expect("prepare");
        // Crash: no further transition.
    }
    // Restart: the unresolved Prepared record is still there.
    let store = JournalStore::open(&path).expect("reopen");
    let unresolved = store.unresolved().expect("unresolved");
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].intent.action_id, "act-d");
    assert_eq!(unresolved[0].state, ActionState::Prepared);
    assert_eq!(
        unresolved[0].intent.creation_filetime,
        133_000_000_000_000_000
    );
}

/// AT-18: clear-history during an active action removes terminal records
/// but retains the recovery intent of unresolved ones.
#[test]
fn clear_history_retains_recovery_intent_during_active_action() {
    let store = JournalStore::open_in_memory().expect("journal");
    // Terminal, unpinned: clearable.
    store.record_prepared(&intent("done-1")).expect("prepare");
    store
        .transition(
            "done-1",
            ActionState::Prepared,
            &Event::ReconcileNoWrite,
            1,
            serde_json::json!({}),
        )
        .expect("terminal");
    // Active action (Applied) and a pinned Conflict: must survive.
    store.record_prepared(&intent("live-1")).expect("prepare");
    store
        .transition(
            "live-1",
            ActionState::Prepared,
            &Event::ApplyVerified {
                deadline_unix_ms: 9,
            },
            1,
            serde_json::json!({}),
        )
        .expect("applied");
    store.record_prepared(&intent("pin-2")).expect("prepare");
    store
        .transition(
            "pin-2",
            ActionState::Prepared,
            &Event::ExternalChange,
            1,
            serde_json::json!({}),
        )
        .expect("conflict");

    let (removed, retained) = store.clear_history().expect("clear");
    assert_eq!((removed, retained), (1, 2));
    assert!(store.get("done-1").expect("store").is_none());
    assert!(
        store.get("live-1").expect("store").is_some(),
        "active action survives clear-history"
    );
    assert!(
        store.get("pin-2").expect("store").is_some(),
        "pinned conflict survives clear-history"
    );
    // The recovery set is still served afterwards.
    assert_eq!(store.unresolved().expect("unresolved").len(), 2);
}

/// AT-18 disk-full: SQLITE_FULL (induced deterministically via the
/// connection page cap — the same error path as real exhaustion) surfaces
/// as DiskFull, leaves prior records intact and readable, and lifts when
/// space returns: recovery of recorded intents proceeds.
#[test]
fn disk_full_blocks_new_records_but_preserves_and_recovers() {
    let store = JournalStore::open_in_memory().expect("journal");
    // One committed record we must never lose.
    store.record_prepared(&intent("precious")).expect("prepare");

    // Enforceable cap: max_page_count takes effect at or above the current
    // size (lower values clamp away). Cap at 50 pages and grow past it with
    // ~8 KB rows; each row spans several overflow pages, so the budget is
    // exhausted after a bounded number of inserts.
    store.set_fault_page_cap(Some(50)).expect("cap");
    let mut saw_disk_full = false;
    for i in 0..64u32 {
        let mut big = intent(&format!("grow-{i}"));
        big.target_name = "D".repeat(8192);
        match store.record_prepared(&big) {
            Ok(_) => {}
            Err(e) => {
                assert!(
                    matches!(e, ramshaper_journal::JournalError::DiskFull),
                    "expected DiskFull, got {e:?}"
                );
                saw_disk_full = true;
                break;
            }
        }
    }
    assert!(saw_disk_full, "the page cap must induce SQLITE_FULL");

    // Prior records are intact and readable while "full" (the grow rows
    // that fit before the failure are readable too — the journal is
    // consistent, not half-written).
    assert!(store.get("precious").expect("store").is_some());
    assert!(store.get("grow-0").expect("store").is_some());

    // Storage failure blocks new applies, not recovery: transitions of the
    // recorded intent still work (a small UPDATE fits in existing pages).
    store
        .transition(
            "precious",
            ActionState::Prepared,
            &Event::ReconcileNoWrite,
            2,
            serde_json::json!({}),
        )
        .expect("recovery transition while full");

    // Space returns: the cap lifts and new records are accepted again.
    store.set_fault_page_cap(None).expect("lift");
    store
        .record_prepared(&intent("after-recovery"))
        .expect("insert after cap lifted");
}

/// RS-016: the store implements the coordinator's ActionJournal trait and
/// an end-to-end preview→confirm→replay round trip works over SQLite.
#[test]
fn store_plugs_into_the_coordinator_trait() {
    use ramshaper_core::coordinator::{
        ConfirmOutcome, Coordinator, PreviewRequest, PreviewVerdict, PriorityActuator,
    };
    use ramshaper_core::eligibility::{
        ChildObservation, FocusObservation, OwnerSessionObservation, PriorityCapability,
        PriorityClassObservation, SafetyClassification,
    };
    use ramshaper_core::identity::IdentityVerification;
    use ramshaper_core::policy::PolicyRules;
    use ramshaper_core::pressure::PressureLevel;
    use std::cell::RefCell;

    let dir = tempfile::tempdir().expect("tempdir");
    let store = JournalStore::open(&dir.path().join("j.db")).expect("open");

    let identity = ramshaper_core::ProcessIdentity::new(4242, 133_000_000_000_000_000);
    let allowlist = ramshaper_core::eligibility::Allowlist::new(
        ramshaper_core::eligibility::ELIGIBILITY_RULES_VERSION,
        &["synthetic-workload.exe"],
    );
    let mut coordinator = Coordinator::new(PolicyRules::default_experiment());

    struct WinActuator {
        written: RefCell<Vec<u32>>,
    }
    impl PriorityActuator for WinActuator {
        fn write_verified(
            &mut self,
            _identity: &ramshaper_core::ProcessIdentity,
            desired: u32,
        ) -> Result<u32, ramshaper_core::coordinator::ActuatorError> {
            self.written.borrow_mut().push(desired);
            Ok(desired)
        }

        fn read(
            &self,
            _identity: &ramshaper_core::ProcessIdentity,
        ) -> Result<u32, ramshaper_core::coordinator::ActuatorError> {
            // The fake apply is live: reads see the last written value.
            Ok(*self.written.borrow().last().unwrap_or(&0x20))
        }
    }

    let verdict = coordinator.preview(
        &store,
        PreviewRequest {
            identity: &identity,
            image_name: "synthetic-workload.exe",
            target: ramshaper_core::policy::TargetObservation {
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
            },
            allowlist: &allowlist,
            user_exclusions: &[],
            pressure: PressureLevel::Elevated,
            action_id: "act-store-1",
            now_unix_ms: 1_000,
            ttl_ms: None,
            desired: 0x4000,
        },
    );
    assert!(matches!(verdict, PreviewVerdict::Offered(_)), "{verdict:?}");

    let mut actuator = WinActuator {
        written: RefCell::new(Vec::new()),
    };
    let outcome = coordinator.confirm(&store, &mut actuator, &identity, "act-store-1", 2_000);
    assert!(
        matches!(
            outcome,
            ConfirmOutcome::Applied {
                deadline_unix_ms: 122_000
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(actuator.written.borrow().len(), 1);

    // Replay over SQLite: same deadline, no second write.
    let replay = coordinator.confirm(&store, &mut actuator, &identity, "act-store-1", 3_000);
    assert_eq!(
        replay,
        ConfirmOutcome::Replayed {
            deadline_unix_ms: 122_000
        }
    );
    assert_eq!(actuator.written.borrow().len(), 1);
    // The record is durably Applied in the file-backed store.
    let record = store.get("act-store-1").expect("get").expect("record");
    assert_eq!(record.state, ActionState::Applied);
    assert_eq!(record.applied_deadline_unix_ms, Some(122_000));
}
