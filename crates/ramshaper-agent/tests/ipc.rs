//! Agent integration tests: real named pipe, real protocol, standard user.
//!
//! An in-process agent serves on a unique per-test pipe name; the tests
//! connect as real clients from the same session. Cross-session and
//! foreign-user peers cannot be produced on a single interactive machine —
//! those verdicts are covered by the pure `validate_peer` unit tests and
//! recorded as Not run in the evidence file.

#![cfg(windows)]

use std::sync::Arc;
use std::time::Duration;

use ramshaper_agent::server::{serve, AgentHandle};
use ramshaper_core::ipc::{parse_request, RequestEnvelope, PROTOCOL_VERSION};
use ramshaper_core::watch::WatchConfig;
use ramshaper_windows::event::ManualResetEvent;
use ramshaper_windows::pipe::{client_connect, read_frame, write_frame};

struct TestAgent {
    handle: Option<AgentHandle>,
}

impl TestAgent {
    /// Serve on a unique pipe at a fast cadence. Panics if the accept loop
    /// reports an early failure (surfaces real pipe errors).
    fn start(name: &str, interval_ms: u64) -> TestAgent {
        let stop = Arc::new(ManualResetEvent::new().expect("stop event"));
        let config_summary = ramshaper_agent::dispatch::ConfigSummary {
            mode: "observe_only".to_string(),
            profile: "balanced".to_string(),
            exclusions: Vec::new(),
        };
        let handle = serve(
            Arc::clone(&stop),
            format!(r"\\.\pipe\{name}"),
            WatchConfig::new(interval_ms).expect("cadence"),
            config_summary,
            None,
        )
        .expect("agent serves");
        // Give the accept thread a moment; fail loudly on early errors.
        std::thread::sleep(Duration::from_millis(150));
        if let Ok(slot) = handle.accept_error.lock() {
            if let Some(message) = slot.as_ref() {
                panic!("accept loop failed early: {message}");
            }
        }
        TestAgent {
            handle: Some(handle),
        }
    }
}

impl Drop for TestAgent {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.stop();
        }
    }
}

/// Full pipe path for a bare test name.
fn full_pipe(name: &str) -> String {
    format!(r"\\.\pipe\{name}")
}

/// Send one request on a connected client and read one response frame.
fn request(pipe: &str, envelope: &RequestEnvelope) -> serde_json::Value {
    let client = client_connect(pipe).expect("connect");
    let payload = serde_json::to_vec(envelope).expect("ser");
    write_frame(&client, &payload).expect("write");
    let frame = read_frame(&client).expect("read");
    serde_json::from_slice(&frame).expect("json")
}

fn envelope(version: u32, command: &str) -> RequestEnvelope {
    RequestEnvelope {
        protocol_version: version,
        request_id: format!("test-{command}"),
        agent_instance_id: None,
        command: command.to_string(),
        payload: None,
    }
}

#[test]
fn capabilities_over_real_pipe() {
    let pipe = format!("ramshaper-test-caps-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    // Give the first instance a moment to enter the accept wait.
    std::thread::sleep(Duration::from_millis(200));
    let response = request(&full_pipe(&pipe), &envelope(1, "GetCapabilities"));
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["request_id"], "test-GetCapabilities");
    assert_eq!(response["data"]["protocol_version"], PROTOCOL_VERSION);
    let caps = response["data"]["capabilities"].as_array().expect("caps");
    assert!(caps
        .iter()
        .any(|c| c["name"] == "system_memory_counters" && c["supported"] == true));
}

#[test]
fn snapshot_over_real_pipe_carries_metrics_and_pressure() {
    let pipe = format!("ramshaper-test-snap-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 300);
    std::thread::sleep(Duration::from_millis(700)); // first snapshot collected
    let response = request(&full_pipe(&pipe), &envelope(1, "GetSnapshot"));
    assert_eq!(response["ok"], true, "{response}");
    assert!(response["data"]["schema"]
        .as_str()
        .is_some_and(|s| s.starts_with("ramshaper.snapshot")));
    assert!(response["data"]["metrics"]["physical_total_bytes"]["value"].is_u64());
    assert!(response["data"]["pressure"]["rule_version"]
        .as_str()
        .is_some_and(|r| r == "pressure_v1"));
    assert!(response["data"]["age_ms"].is_u64());
}

#[test]
fn version_mismatch_is_rejected_before_dispatch() {
    let pipe = format!("ramshaper-test-ver-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    std::thread::sleep(Duration::from_millis(200));
    let response = request(&full_pipe(&pipe), &envelope(2, "GetSnapshot"));
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "VersionMismatch");
    assert_eq!(response["error"]["retryable"], false);
}

#[test]
fn unknown_command_is_unsupported() {
    let pipe = format!("ramshaper-test-unk-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    std::thread::sleep(Duration::from_millis(200));
    let response = request(&full_pipe(&pipe), &envelope(1, "SetPriorityClass"));
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "Unsupported");
}

#[test]
fn malformed_frame_gets_one_error_then_close() {
    let pipe = format!("ramshaper-test-mal-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    std::thread::sleep(Duration::from_millis(200));
    let client = client_connect(&full_pipe(&pipe)).expect("connect");
    // A frame whose payload is not a request envelope.
    write_frame(&client, b"garbage not json").expect("write");
    let frame = read_frame(&client).expect("error response");
    let value: serde_json::Value = serde_json::from_slice(&frame).expect("json");
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "Unsupported");
    // Connection is closed after the malformed frame: the next read fails.
    assert!(read_frame(&client).is_err());
}

#[test]
fn malformed_framing_prefix_ends_the_connection_silently() {
    let pipe = format!("ramshaper-test-frame-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    std::thread::sleep(Duration::from_millis(200));
    let client = client_connect(&full_pipe(&pipe)).expect("connect");
    // A zero-length prefix is a framing violation (not a parseable request):
    // the server cannot correlate it to any request id, so it answers no
    // error frame at all and simply ends the connection.
    ramshaper_windows::pipe::write_bytes(&client, &[0u8; 4]).expect("write prefix");
    let outcome = read_frame(&client);
    assert!(
        outcome.is_err(),
        "zero-length prefix must terminate the connection without a response"
    );
}

#[test]
fn replayed_request_is_served_identically_on_the_same_connection() {
    // Protocol v1 carries no replay cache: request_id is a correlation id,
    // not an anti-replay token. This is an accepted v1 limitation because
    // every command is read-only and idempotent, so a replayed request is
    // benign by construction. This test pins that property: the identical
    // frame served twice yields two identical successful responses with the
    // same correlation, and nothing mutates between them.
    let pipe = format!("ramshaper-test-replay-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    std::thread::sleep(Duration::from_millis(200));
    let client = client_connect(&full_pipe(&pipe)).expect("connect");
    let payload = serde_json::to_vec(&envelope(1, "GetCapabilities")).expect("ser");
    let mut values = Vec::new();
    for _ in 0..2 {
        write_frame(&client, &payload).expect("write replayed frame");
        let frame = read_frame(&client).expect("response");
        values.push(serde_json::from_slice::<serde_json::Value>(&frame).expect("json"));
    }
    for value in &values {
        assert_eq!(value["ok"], true, "{value}");
        assert_eq!(value["request_id"], "test-GetCapabilities");
    }
    assert_eq!(
        values[0]["data"], values[1]["data"],
        "a replayed read-only request must be served identically"
    );
}

#[test]
fn oversized_frame_prefix_is_rejected_without_allocation() {
    let pipe = format!("ramshaper-test-big-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    std::thread::sleep(Duration::from_millis(200));
    let client = client_connect(&full_pipe(&pipe)).expect("connect");
    // A raw length prefix far beyond the 1 MiB cap (256 MiB), with no
    // payload. The reader must reject the prefix and close the connection
    // instead of allocating the buffer.
    ramshaper_windows::pipe::write_bytes(&client, &(256u32 * 1024 * 1024).to_le_bytes())
        .expect("write prefix");
    // The connection must end (read error) rather than block or accept.
    let outcome = read_frame(&client);
    assert!(
        outcome.is_err(),
        "oversized frame must terminate the connection"
    );
}

#[test]
fn subscribe_streams_snapshots_until_peer_leaves() {
    let pipe = format!("ramshaper-test-sub-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 250);
    std::thread::sleep(Duration::from_millis(200));
    let client = client_connect(&full_pipe(&pipe)).expect("connect");
    let payload = serde_json::to_vec(&envelope(1, "SubscribeSnapshots")).expect("ser");
    write_frame(&client, &payload).expect("write subscribe");
    // The subscription is acknowledged before the stream starts.
    let ack = read_frame(&client).expect("subscribe ack");
    let ack_value: serde_json::Value = serde_json::from_slice(&ack).expect("ack json");
    assert_eq!(ack_value["ok"], true, "{ack_value}");
    assert_eq!(ack_value["data"]["subscribed"], true);

    let deadline = std::time::Instant::now() + Duration::from_millis(2_000);
    let mut deliveries = 0u64;
    while std::time::Instant::now() < deadline && deliveries < 4 {
        match read_frame(&client) {
            Ok(frame) => {
                let value: serde_json::Value = serde_json::from_slice(&frame).expect("json");
                assert_eq!(value["ok"], true, "{value}");
                let data = &value["data"];
                assert!(data["snapshot"]["schema"].is_string());
                assert!(data["gap_dropped"].is_u64());
                deliveries += 1;
            }
            Err(_) => panic!("stream ended early after {deliveries} deliveries"),
        }
    }
    assert!(
        deliveries >= 3,
        "only {deliveries} deliveries in 2 s at 250 ms cadence"
    );
    drop(client); // peer leaves; server must clean up (no leak assertions here)
}

#[test]
fn request_envelope_roundtrip_via_ipc_module() {
    // The core codec and the envelope types must agree.
    let payload = serde_json::to_vec(&envelope(1, "ListActions")).expect("ser");
    let parsed = parse_request(&payload).expect("parse");
    assert_eq!(parsed.command, "ListActions");
    assert_eq!(parsed.protocol_version, PROTOCOL_VERSION);
}

#[test]
fn list_actions_is_empty_and_honest() {
    let pipe = format!("ramshaper-test-acts-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    std::thread::sleep(Duration::from_millis(200));
    let response = request(&full_pipe(&pipe), &envelope(1, "ListActions"));
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(
        response["data"]["actions"].as_array().map(Vec::len),
        Some(0)
    );
}

#[test]
fn get_processes_serves_a_real_inventory_with_warming_then_real_rates() {
    let pipe = format!("ramshaper-test-procs-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    std::thread::sleep(Duration::from_millis(200));

    // First request: one fresh scan; instantaneous values present, rates
    // honestly warming (no previous scan to diff against).
    let first = request(&full_pipe(&pipe), &envelope(1, "GetProcesses"));
    assert_eq!(first["ok"], true, "{first}");
    let data = &first["data"];
    assert_eq!(data["schema"], "ramshaper.processes.v1", "{data}");
    let rows = data["processes"].as_array().expect("rows");
    assert!(!rows.is_empty(), "a live machine has processes");
    assert!(rows.len() <= 1024, "row cap must hold: {}", rows.len());
    let self_row = rows.iter().find(|r| r["working_set"]["status"] == "ok");
    assert!(self_row.is_some(), "at least one row with real bytes");

    // Second request after a real gap: rates computed over that gap.
    std::thread::sleep(Duration::from_millis(700));
    let second = request(&full_pipe(&pipe), &envelope(1, "GetProcesses"));
    assert_eq!(second["ok"], true, "{second}");
    let data2 = &second["data"];
    assert!(
        data2["rate_interval_ms"].as_u64().unwrap_or(0) >= 500,
        "rate interval must span the real gap: {data2}"
    );
    // Every row carries the identity contract (pid + decimal FILETIME).
    let rows2 = data2["processes"].as_array().expect("rows");
    assert!(rows2.iter().all(|r| r["identity"]["pid"].is_u64()
        && r["identity"]["creation_filetime_100ns"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))));
}

#[test]
fn get_processes_rejects_wrong_version_before_any_scan() {
    let pipe = format!("ramshaper-test-procv-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    std::thread::sleep(Duration::from_millis(200));
    let response = request(&full_pipe(&pipe), &envelope(2, "GetProcesses"));
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "VersionMismatch");
}

#[test]
fn get_config_over_real_pipe_serves_the_inert_summary() {
    let pipe = format!("ramshaper-test-cfg-{}", std::process::id());
    let _agent = TestAgent::start(&pipe, 500);
    std::thread::sleep(Duration::from_millis(200));
    let response = request(&full_pipe(&pipe), &envelope(1, "GetConfig"));
    assert_eq!(response["ok"], true, "{response}");
    let data = &response["data"];
    assert_eq!(data["mode"], "observe_only");
    assert_eq!(data["profile"], "balanced");
    assert_eq!(data["exclusions"], serde_json::json!([]));
    assert!(data["note"].as_str().is_some_and(|n| n.contains("inert")));
    // Wrong version is still rejected before dispatch.
    let bad = request(&full_pipe(&pipe), &envelope(2, "GetConfig"));
    assert_eq!(bad["ok"], false, "{bad}");
    assert_eq!(bad["error"]["code"], "VersionMismatch");
}
