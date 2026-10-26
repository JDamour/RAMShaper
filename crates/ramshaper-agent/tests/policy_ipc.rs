//! RS-016 completion e2e: the policy gate over a real journal, real
//! actuator and a spawned child process. The only live write target is the
//! test's own `ping` child (Normal → Below Normal → Normal), restored
//! inside the test. The injected allowlist entry and classification
//! override stand in for RS-024 Stage B evidence — production ships
//! neither.

use std::process::{Child, Command};

use ramshaper_agent::policy::PolicyGate;
use ramshaper_core::eligibility::{Allowlist, SafetyClassification, ELIGIBILITY_RULES_VERSION};
use ramshaper_core::ipc::{
    ConfirmActionRequest, DisableActionsRequest, ErrorCode, PreviewActionRequest,
    RestoreActionRequest,
};
use ramshaper_core::pressure::PressureLevel;
use ramshaper_journal::JournalStore;
use ramshaper_windows::eligibility;
use ramshaper_windows::identity::resolve_identity;

/// A quiet, alive child: `ping -n <seconds> 127.0.0.1`.
fn spawn_quiet_child(seconds: u32) -> Child {
    Command::new("ping")
        .args(["-n", &seconds.to_string(), "127.0.0.1"])
        .spawn()
        .expect("spawn ping child")
}

/// A gate wired like the tested matrix would: the child's image name is on
/// the allowlist and classification is verified noncritical.
fn test_gate() -> PolicyGate {
    let store = JournalStore::open_in_memory().expect("in-memory journal");
    PolicyGate::open_with_classification(
        store,
        Allowlist::new(ELIGIBILITY_RULES_VERSION, &["PING.EXE"]),
        Vec::new(),
        Box::new(|_pid, _image| SafetyClassification::VerifiedNonCritical),
    )
}

const NOW: u64 = 1_800_000_000_000;

#[test]
fn policy_gate_full_loop_applies_and_restores_a_spawned_child() {
    let mut child = spawn_quiet_child(8);
    let pid = child.id();
    let identity = resolve_identity(pid).expect("fresh child identity");
    let gate = test_gate();
    gate.enable();
    assert!(gate.enabled());

    // Preview: offered under fresh Elevated pressure with the tested
    // allowlist entry and verified classification.
    let preview = gate.preview(
        &PreviewActionRequest {
            action_id: "e2e-1".to_string(),
            pid,
            creation_filetime: identity.creation_filetime_100ns.to_string(),
            image_name: "ping.exe".to_string(),
            ttl_ms: None,
        },
        PressureLevel::Elevated,
        NOW,
    );
    let offered = match preview {
        ramshaper_agent::policy::ResponseData::Ok(value) => {
            assert_eq!(value["verdict"], "offered", "got {value}");
            value
        }
        ramshaper_agent::policy::ResponseData::Err { code, message, .. } => {
            panic!("preview denied: {code:?} {message}")
        }
    };
    assert_eq!(offered["ttl_ms"], 120_000, "the policy default TTL");
    assert_eq!(offered["desired"], 0x4000, "Below Normal is the only write");

    // Confirm: durable prepare, verified write, journaled Applied.
    let confirm = gate.confirm(
        &ConfirmActionRequest {
            action_id: "e2e-1".to_string(),
        },
        NOW + 1_000,
    );
    match confirm {
        ramshaper_agent::policy::ResponseData::Ok(value) => {
            assert_eq!(value["outcome"], "applied", "got {value}");
            assert_eq!(value["deadline_unix_ms"], NOW + 1_000 + 120_000);
        }
        ramshaper_agent::policy::ResponseData::Err { code, message, .. } => {
            panic!("confirm failed: {code:?} {message}");
        }
    }
    assert_eq!(
        eligibility::observe_priority_class(pid),
        ramshaper_core::eligibility::PriorityClassObservation::Other {
            detail: "BELOW_NORMAL_PRIORITY_CLASS".to_string(),
        },
        "the live child must be at Below Normal after the confirmed apply"
    );

    // Lost-reply replay: a second confirm replays the recorded outcome —
    // never a second write.
    let replay = gate.confirm(
        &ConfirmActionRequest {
            action_id: "e2e-1".to_string(),
        },
        NOW + 2_000,
    );
    match replay {
        ramshaper_agent::policy::ResponseData::Ok(value) => {
            assert_eq!(value["outcome"], "replayed", "got {value}");
            assert_eq!(value["deadline_unix_ms"], NOW + 1_000 + 120_000);
        }
        ramshaper_agent::policy::ResponseData::Err { code, message, .. } => {
            panic!("replay failed: {code:?} {message}");
        }
    }

    // Manual restore over IPC: back to Normal, verified by an independent
    // adapter read.
    let restore = gate.restore(
        &RestoreActionRequest {
            action_id: "e2e-1".to_string(),
        },
        NOW + 3_000,
    );
    match restore {
        ramshaper_agent::policy::ResponseData::Ok(value) => {
            assert_eq!(value["outcome"], "restored", "got {value}");
            assert_eq!(value["wrote_back"], true);
        }
        ramshaper_agent::policy::ResponseData::Err { code, message, .. } => {
            panic!("restore failed: {code:?} {message}");
        }
    }
    assert_eq!(
        eligibility::observe_priority_class(pid),
        ramshaper_core::eligibility::PriorityClassObservation::Normal,
    );

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn untested_third_party_targets_are_denied_with_the_full_report() {
    let mut child = spawn_quiet_child(4);
    let pid = child.id();
    let identity = resolve_identity(pid).expect("fresh child identity");
    // Production shape: EMPTY allowlist, self-only classification.
    let store = JournalStore::open_in_memory().expect("in-memory journal");
    let gate = PolicyGate::open_with_classification(
        store,
        Allowlist::empty(),
        Vec::new(),
        Box::new(|_pid, _image| {
            // No tested matrix exists: every third-party target is unknown.
            SafetyClassification::Unknown {
                reason: "no tested classification signal covers this target".to_string(),
            }
        }),
    );
    gate.enable();

    let preview = gate.preview(
        &PreviewActionRequest {
            action_id: "deny-1".to_string(),
            pid,
            creation_filetime: identity.creation_filetime_100ns.to_string(),
            image_name: "ping.exe".to_string(),
            ttl_ms: None,
        },
        PressureLevel::Elevated,
        NOW,
    );
    match preview {
        ramshaper_agent::policy::ResponseData::Err {
            code,
            detail: Some(detail),
            ..
        } => {
            assert_eq!(code, ErrorCode::Ineligible);
            let checks = detail["checks"].as_array().expect("explain-all report");
            assert!(
                checks.iter().any(|c| c["outcome"].get("Deny").is_some()),
                "at least one deny in the report: {detail}"
            );
        }
        other => panic!("expected an Ineligible error with the report, got {other:?}"),
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn disabled_gate_rejects_previews_until_explicitly_enabled() {
    let store = JournalStore::open_in_memory().expect("in-memory journal");
    let gate = PolicyGate::open_with_classification(
        store,
        Allowlist::empty(),
        Vec::new(),
        Box::new(|_pid, _image| SafetyClassification::VerifiedNonCritical),
    );
    // Startup is Observe Only: the gate is disabled.
    assert!(!gate.enabled());
    let preview = gate.preview(
        &PreviewActionRequest {
            action_id: "blocked-1".to_string(),
            pid: 123_456,
            creation_filetime: "1000".to_string(),
            image_name: "ping.exe".to_string(),
            ttl_ms: None,
        },
        PressureLevel::Elevated,
        NOW,
    );
    match preview {
        ramshaper_agent::policy::ResponseData::Err { code, message, .. } => {
            assert_eq!(code, ErrorCode::Busy, "got {message}");
            assert!(message.contains("disabled"));
        }
        other => panic!("expected Busy for a disabled gate, got {other:?}"),
    }
}

#[test]
fn disable_drains_outstanding_actions_and_blocks_new_previews() {
    let mut child = spawn_quiet_child(8);
    let pid = child.id();
    let identity = resolve_identity(pid).expect("fresh child identity");
    let gate = test_gate();
    gate.enable();

    gate.preview(
        &PreviewActionRequest {
            action_id: "drain-1".to_string(),
            pid,
            creation_filetime: identity.creation_filetime_100ns.to_string(),
            image_name: "ping.exe".to_string(),
            ttl_ms: Some(10_000),
        },
        PressureLevel::Elevated,
        NOW,
    );
    gate.confirm(
        &ConfirmActionRequest {
            action_id: "drain-1".to_string(),
        },
        NOW + 500,
    );
    assert_eq!(
        eligibility::observe_priority_class(pid),
        ramshaper_core::eligibility::PriorityClassObservation::Other {
            detail: "BELOW_NORMAL_PRIORITY_CLASS".to_string(),
        },
    );

    // Disable: everything outstanding is restored immediately.
    let disabled = gate.disable(&DisableActionsRequest {}, NOW + 1_000);
    match disabled {
        ramshaper_agent::policy::ResponseData::Ok(value) => {
            assert_eq!(value["disabled"], true);
            assert_eq!(value["restored"], 1, "got {value}");
            assert_eq!(value["unresolved"], 0);
        }
        other => panic!("expected the disable summary, got {other:?}"),
    }
    assert_eq!(
        eligibility::observe_priority_class(pid),
        ramshaper_core::eligibility::PriorityClassObservation::Normal,
        "the drain must put the child back to Normal"
    );
    assert!(!gate.enabled());

    // New previews are refused after disable.
    let after = gate.preview(
        &PreviewActionRequest {
            action_id: "after-1".to_string(),
            pid,
            creation_filetime: identity.creation_filetime_100ns.to_string(),
            image_name: "ping.exe".to_string(),
            ttl_ms: None,
        },
        PressureLevel::Elevated,
        NOW + 2_000,
    );
    match after {
        ramshaper_agent::policy::ResponseData::Err { code, .. } => {
            assert_eq!(code, ErrorCode::Busy);
        }
        other => panic!("expected Busy after disable, got {other:?}"),
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn restore_of_an_unknown_id_is_an_explained_refusal() {
    let gate = test_gate();
    gate.enable();
    let restore = gate.restore(
        &RestoreActionRequest {
            action_id: "no-such-action".to_string(),
        },
        NOW,
    );
    match restore {
        ramshaper_agent::policy::ResponseData::Err { code, message, .. } => {
            assert_eq!(code, ErrorCode::InvalidArgument, "got {message}");
        }
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn stopping_the_agent_runs_the_clean_exit_trigger() {
    // Contract 14 §6 lists "clean agent exit" among the restore triggers:
    // AgentHandle::stop disables the gate (no new applies) and restores
    // every outstanding record through the engine BEFORE the workers shut
    // down — the child must be back at Normal after the stop, with the
    // journal record terminal.
    use ramshaper_agent::dispatch::ConfigSummary;
    use ramshaper_agent::server::serve;
    use ramshaper_core::coordinator::ActionJournal as _;
    use ramshaper_core::journal::ActionState;
    use ramshaper_core::watch::WatchConfig;
    use ramshaper_windows::event::ManualResetEvent;
    use std::sync::Arc;
    use std::time::Duration;

    let mut child = spawn_quiet_child(10);
    let pid = child.id();
    let identity = resolve_identity(pid).expect("fresh child identity");
    let gate = std::sync::Arc::new(test_gate());
    gate.enable();

    let stop = Arc::new(ManualResetEvent::new().expect("stop event"));
    let handle = serve(
        Arc::clone(&stop),
        format!(r"\.\pipe\ramshaper-test-cleanexit-{}", std::process::id()),
        WatchConfig::new(500).expect("cadence"),
        ConfigSummary {
            mode: "observe_only".to_string(),
            profile: "balanced".to_string(),
            exclusions: Vec::new(),
        },
        Some(Arc::clone(&gate)),
    )
    .expect("agent serves");
    std::thread::sleep(Duration::from_millis(200));

    // Apply through the gate (the same injected-Elevated seam as the full
    // loop test; the pipe's real collector is not the pressure source).
    let preview = gate.preview(
        &PreviewActionRequest {
            action_id: "clean-exit-1".to_string(),
            pid,
            creation_filetime: identity.creation_filetime_100ns.to_string(),
            image_name: "ping.exe".to_string(),
            ttl_ms: None,
        },
        PressureLevel::Elevated,
        NOW,
    );
    match preview {
        ramshaper_agent::policy::ResponseData::Ok(value) => {
            assert_eq!(value["verdict"], "offered", "got {value}");
        }
        ramshaper_agent::policy::ResponseData::Err { code, message, .. } => {
            panic!("preview denied: {code:?} {message}")
        }
    }
    let confirm = gate.confirm(
        &ConfirmActionRequest {
            action_id: "clean-exit-1".to_string(),
        },
        NOW + 1_000,
    );
    match confirm {
        ramshaper_agent::policy::ResponseData::Ok(value) => {
            assert_eq!(value["outcome"], "applied", "got {value}");
        }
        ramshaper_agent::policy::ResponseData::Err { code, message, .. } => {
            panic!("confirm failed: {code:?} {message}");
        }
    }
    assert_eq!(
        eligibility::observe_priority_class(pid),
        ramshaper_core::eligibility::PriorityClassObservation::Other {
            detail: "BELOW_NORMAL_PRIORITY_CLASS".to_string(),
        },
        "the live child must be at Below Normal before the stop"
    );

    // The clean-exit trigger: stop() restores first, then joins.
    handle.stop();

    assert_eq!(
        eligibility::observe_priority_class(pid),
        ramshaper_core::eligibility::PriorityClassObservation::Normal,
        "the clean exit must have restored the applied setting"
    );
    assert!(!gate.enabled(), "the gate ends the session disabled");
    let snapshot = gate
        .journal_clone()
        .record_snapshot("clean-exit-1")
        .expect("journal readable")
        .expect("record exists");
    assert_eq!(
        snapshot.state,
        ActionState::Restored,
        "the journal shows the terminal restore"
    );

    let _ = child.kill();
    let _ = child.wait();
}
