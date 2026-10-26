//! RS-026 boundary review: adversarial policy-command probes over a live
//! gated agent (in-process server, real pipe). The gate exists but stays
//! DISABLED (EOF consent = declined): the review pins what a same-user
//! client can and cannot make happen without the operator's typed
//! consent.

use std::sync::Arc;
use std::time::Duration;

use ramshaper_agent::consent;
use ramshaper_core::ipc::{
    RequestEnvelope, COMMAND_CONFIRM_ACTION, COMMAND_DISABLE_ACTIONS, COMMAND_LIST_ACTIONS,
    COMMAND_PREVIEW_ACTION, COMMAND_RESTORE_ACTION, PROTOCOL_VERSION,
};
use ramshaper_windows::pipe::{client_connect, read_frame, write_frame};

/// Serve once on a fresh pipe with a gate that is present but disabled
/// (the shipped consent path with stdin at EOF).
fn serve_disabled_gate(name: &str) -> Arc<ramshaper_windows::event::ManualResetEvent> {
    let dir = std::env::temp_dir().join(format!("rs026-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let journal = dir.join("journal.sqlite");
    let _ = std::fs::remove_file(&journal);
    let gate = consent::open_session_gate(&journal, Vec::new()).expect("gated journal");
    assert!(!gate.enabled(), "the gate must start disabled");
    let stop = Arc::new(ramshaper_windows::event::ManualResetEvent::new().expect("stop event"));
    let config_summary = ramshaper_agent::dispatch::ConfigSummary {
        mode: "observe_only".to_string(),
        profile: "balanced".to_string(),
        exclusions: Vec::new(),
    };
    ramshaper_agent::server::serve(
        Arc::clone(&stop),
        format!(r"\\.\pipe\{name}"),
        ramshaper_core::watch::WatchConfig::new(1000).expect("cadence"),
        config_summary,
        Some(Arc::new(gate)),
    )
    .expect("agent serves");
    std::thread::sleep(Duration::from_millis(200));
    stop
}

fn request(pipe: &str, command: &str, payload: Option<serde_json::Value>) -> serde_json::Value {
    let client = client_connect(&format!(r"\\.\pipe\{pipe}")).expect("connect");
    let envelope = RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: format!("probe-{command}"),
        agent_instance_id: None,
        command: command.to_string(),
        payload,
    };
    let payload = serde_json::to_vec(&envelope).expect("encode");
    write_frame(&client, &payload).expect("write");
    let frame = read_frame(&client).expect("read");
    serde_json::from_slice(&frame).expect("decode")
}

fn code_of(response: &serde_json::Value) -> String {
    response["error"]["code"]
        .as_str()
        .unwrap_or("no-error")
        .to_string()
}

#[test]
fn a_disabled_gate_refuses_every_action_command_with_explanations() {
    let stop = serve_disabled_gate("rs026-refusals");
    // Preview: refused Busy — the operator's typed consent is the only key.
    let preview = request(
        "rs026-refusals",
        COMMAND_PREVIEW_ACTION,
        Some(serde_json::json!({
            "action_id": "probe-1",
            "pid": 4242,
            "creation_filetime": "133000000000000000",
            "image_name": "leaf.exe"
        })),
    );
    assert_eq!(code_of(&preview), "Busy", "{preview}");
    assert!(
        preview["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("disabled"),
        "the refusal explains the disabled session: {preview}"
    );
    // Confirm and restore are refused (InvalidArgument — their binding/
    // record checks run first; nothing succeeds without the consent word).
    // DisableActions is ALLOWED to succeed on a disabled gate: it is an
    // idempotent no-op that can only REDUCE authority (disable the already
    // disabled session and drain a necessarily-empty outstanding set) —
    // never grant one. The response pins that reading.
    for (command, expected) in [
        (COMMAND_CONFIRM_ACTION, "InvalidArgument"),
        (COMMAND_RESTORE_ACTION, "InvalidArgument"),
    ] {
        let response = request(
            "rs026-refusals",
            command,
            Some(serde_json::json!({ "action_id": "probe-1" })),
        );
        assert_eq!(code_of(&response), expected, "{command}: {response}");
        assert_eq!(response["ok"], false, "{command}");
    }
    let disable = request(
        "rs026-refusals",
        COMMAND_DISABLE_ACTIONS,
        Some(serde_json::json!({})),
    );
    assert_eq!(disable["ok"], true, "{disable}");
    assert_eq!(disable["data"]["disabled"], true);
    assert_eq!(disable["data"]["restored"], 0);
    assert_eq!(disable["data"]["unresolved"], 0);
    // ListActions stays READABLE while disabled (observation, not action).
    let list = request("rs026-refusals", COMMAND_LIST_ACTIONS, None);
    assert_eq!(list["ok"], true, "{list}");
    stop.trigger();
}

#[test]
fn malformed_payloads_are_invalid_arguments_not_panics() {
    let stop = serve_disabled_gate("rs026-malformed");
    // Non-numeric creation FILETIME.
    let bad_filetime = request(
        "rs026-malformed",
        COMMAND_PREVIEW_ACTION,
        Some(serde_json::json!({
            "action_id": "m-1",
            "pid": 4242,
            "creation_filetime": "not-a-number",
            "image_name": "leaf.exe"
        })),
    );
    assert_eq!(code_of(&bad_filetime), "InvalidArgument", "{bad_filetime}");
    // Wrong payload shape entirely (missing fields).
    let wrong_shape = request(
        "rs026-malformed",
        COMMAND_PREVIEW_ACTION,
        Some(serde_json::json!({ "action_id": "m-2" })),
    );
    assert_eq!(code_of(&wrong_shape), "InvalidArgument", "{wrong_shape}");
    // Payload of the wrong JSON type.
    let wrong_type = request(
        "rs026-malformed",
        COMMAND_CONFIRM_ACTION,
        Some(serde_json::json!([1, 2, 3])),
    );
    assert_eq!(code_of(&wrong_type), "InvalidArgument", "{wrong_type}");
    stop.trigger();
}

#[test]
fn unknown_action_ids_never_create_journal_state() {
    let stop = serve_disabled_gate("rs026-unknown-ids");
    // Confirm for an ID that was never previewed.
    let confirm = request(
        "rs026-unknown-ids",
        COMMAND_CONFIRM_ACTION,
        Some(serde_json::json!({ "action_id": "never-previewed" })),
    );
    // Busy (disabled gate is checked first) — either way no record may
    // appear in ListActions.
    let list = request("rs026-unknown-ids", COMMAND_LIST_ACTIONS, None);
    assert_eq!(list["ok"], true);
    assert_eq!(
        list["data"]["actions"].as_array().map(Vec::len),
        Some(0),
        "no journal state may be created by probe requests: {list}"
    );
    let _ = confirm;
    stop.trigger();
}
