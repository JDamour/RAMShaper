//! End-to-end tests for the CLI `restore` command (contract 14 §8 CLI
//! surface). Two paths: the wire refusal against a real GATELESS
//! production agent (Unsupported is the outcome), and the happy path
//! through a test-constructed gate with a real applied action on a real
//! ping child (the agent crate's own live-test pattern; the injected
//! seam carries the Elevated pressure level the wire gate would honestly
//! demand).

use std::process::Command;
use std::sync::Arc;

use ramshaper_agent::dispatch::ConfigSummary;
use ramshaper_agent::policy::PolicyGate;
use ramshaper_agent::server::serve;
use ramshaper_core::clock::WallClock as _;
use ramshaper_core::eligibility::{Allowlist, SafetyClassification, ELIGIBILITY_RULES_VERSION};
use ramshaper_core::ipc::{ConfirmActionRequest, PreviewActionRequest};
use ramshaper_core::pressure::PressureLevel;
use ramshaper_core::watch::WatchConfig;
use ramshaper_windows::event::ManualResetEvent;

fn cli() -> &'static str {
    env!("CARGO_BIN_EXE_ramshaper-cli")
}

fn serve_gateless(pipe_name: &str) -> ramshaper_agent::server::AgentHandle {
    let stop = Arc::new(ManualResetEvent::new().expect("stop event"));
    serve(
        stop,
        pipe_name.to_string(),
        WatchConfig::new(500).expect("cadence"),
        ConfigSummary {
            mode: "observe_only".to_string(),
            profile: "balanced".to_string(),
            exclusions: Vec::new(),
        },
        None,
    )
    .expect("agent serves")
}

/// Build the test gate (ping fixture allowed, classification override),
/// enable it, serve it, and return the gate Arc (for the in-process
/// preview/confirm that carry the pressure seam) plus the handle.
fn serve_test_gate(
    pipe_name: &str,
) -> (
    Arc<PolicyGate>,
    ramshaper_agent::server::AgentHandle,
    Arc<ManualResetEvent>,
) {
    let gate = Arc::new(PolicyGate::open_with_classification(
        ramshaper_journal::JournalStore::open_in_memory().expect("in-memory journal"),
        Allowlist::new(ELIGIBILITY_RULES_VERSION, &["PING.EXE"]),
        Vec::new(),
        Box::new(|_pid, image| {
            if image.eq_ignore_ascii_case("ping.exe") {
                SafetyClassification::VerifiedNonCritical
            } else {
                SafetyClassification::Unknown {
                    reason: "test gate: only the ping fixture is covered".to_string(),
                }
            }
        }),
    ));
    gate.enable();
    let stop = Arc::new(ManualResetEvent::new().expect("stop event"));
    let handle = serve(
        Arc::clone(&stop),
        pipe_name.to_string(),
        WatchConfig::new(500).expect("cadence"),
        ConfigSummary {
            mode: "observe_only".to_string(),
            profile: "balanced".to_string(),
            exclusions: Vec::new(),
        },
        Some(Arc::clone(&gate)),
    )
    .expect("agent serves");
    (gate, handle, stop)
}

fn apply_to_child(gate: &PolicyGate, action_id: &str, pid: u32) {
    let identity = ramshaper_windows::identity::resolve_identity(pid).expect("child identity");
    let now = ramshaper_core::SystemClock.unix_epoch_ms();
    let preview = gate.preview(
        &PreviewActionRequest {
            action_id: action_id.to_string(),
            pid,
            creation_filetime: identity.creation_filetime_100ns.to_string(),
            image_name: "PING.EXE".to_string(),
            ttl_ms: None,
        },
        PressureLevel::Elevated,
        now,
    );
    assert!(
        matches!(preview, ramshaper_agent::policy::ResponseData::Ok(_)),
        "preview must be offered: {preview:?}"
    );
    let confirm = gate.confirm(
        &ConfirmActionRequest {
            action_id: action_id.to_string(),
        },
        now + 1_000,
    );
    match confirm {
        ramshaper_agent::policy::ResponseData::Ok(value) => {
            assert_eq!(value["outcome"], "applied", "{value:?}");
        }
        other => panic!("confirm failed: {other:?}"),
    }
}

#[test]
fn restore_against_a_gateless_agent_refuses_with_exit_1() {
    let pipe = "\\\\.\\pipe\\restore-ev-gateless-1";
    let handle = serve_gateless(pipe);
    std::thread::sleep(std::time::Duration::from_millis(300));

    let out = Command::new(cli())
        .args(["restore", "--pipe-name", pipe, "--action", "no-such-1"])
        .output()
        .expect("run the cli");
    assert_eq!(
        out.status.code(),
        Some(1),
        "Unsupported is a refusal, exit 1: {:?}",
        out.stderr
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Unsupported"),
        "the typed refusal is carried: {stderr}"
    );
    assert!(
        stderr.contains("nothing was changed"),
        "the no-change note is printed: {stderr}"
    );
    assert!(out.stdout.is_empty(), "refusals go to stderr only");

    handle.stop();
}

#[test]
fn restore_restores_a_live_applied_action_end_to_end() {
    let pipe = "\\\\.\\pipe\\restore-ev-happy-1";
    let (gate, handle, _stop) = serve_test_gate(pipe);
    std::thread::sleep(std::time::Duration::from_millis(300));

    let mut child = std::process::Command::new("ping")
        .args(["-n", "9", "127.0.0.1"])
        .spawn()
        .expect("spawn ping child");
    apply_to_child(&gate, "restore-e2e-1", child.id());

    // The child is at Below Normal now (the gate's real actuator applied).
    assert_eq!(
        ramshaper_windows::eligibility::observe_priority_class(child.id()),
        ramshaper_core::eligibility::PriorityClassObservation::Other {
            detail: "BELOW_NORMAL_PRIORITY_CLASS".to_string(),
        },
        "the applied class must be live before the restore"
    );

    let out = Command::new(cli())
        .args([
            "restore",
            "--pipe-name",
            pipe,
            "--action",
            "restore-e2e-1",
            "--json",
        ])
        .output()
        .expect("run the cli");
    assert_eq!(out.status.code(), Some(0), "{:?}", out.stderr);
    let record: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("JSON record on stdout");
    assert_eq!(record["schema"], "ramshaper.restore.v1");
    assert_eq!(record["verb"], "RestoreAction");
    assert_eq!(record["outcome"]["outcome"], "restored");
    assert_eq!(record["outcome"]["action_id"], "restore-e2e-1");

    // The real proof: the child's priority is back at Normal.
    assert_eq!(
        ramshaper_windows::eligibility::observe_priority_class(child.id()),
        ramshaper_core::eligibility::PriorityClassObservation::Normal,
        "the restore must have written the original class back"
    );

    handle.stop();
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn disable_all_against_a_gateless_agent_refuses_the_same_way() {
    // The verb exists on the wire; against a gateless agent the refusal
    // is the same explained Unsupported (no special casing).
    let pipe = "\\\\.\\pipe\\restore-ev-disable-1";
    let handle = serve_gateless(pipe);
    std::thread::sleep(std::time::Duration::from_millis(300));

    let out = Command::new(cli())
        .args(["restore", "--pipe-name", pipe, "--disable-all"])
        .output()
        .expect("run the cli");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Unsupported"), "{stderr}");

    handle.stop();
}
