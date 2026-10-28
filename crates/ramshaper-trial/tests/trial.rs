//! End-to-end tests for the Stage B trial driver.
//!
//! Happy path: `run_trial`/`run_once` at the library boundary with a
//! scripted [`ramshaper_trial::AgentRpc`] — the REAL subject binary is
//! spawned, held, released and collected, and the exact wire verb
//! sequence (Preview → Confirm → Restore, nothing for the baseline run)
//! is pinned. The apply/restore MECHANICS behind the fake's answers are
//! proven live in the agent crate's own tests (which inject Elevated
//! pressure through the in-process gate API — the real wire gate would
//! honestly refuse previews on this unpressured machine, and fabricating
//! memory pressure during the AT-06 soak is off-limits).
//!
//! Refusal paths: a refused preview fails the trial fast (no blocks file,
//! the abandoned held subject never runs untreated), and a production
//! gateless agent (no `--policy-journal`) answers `Unsupported` over a
//! real pipe — the real driver binary must stop honestly.
//!
//! The shipped trial runbook (post-ADR-045) covers the live
//! agent-with-gate configuration; its remaining precondition is real
//! Elevated memory-pressure evidence at trial time.

use std::io::{BufRead as _, Read as _, Write as _};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use ramshaper_agent::dispatch::ConfigSummary;
use ramshaper_agent::server::serve;
use ramshaper_core::watch::WatchConfig;
use ramshaper_windows::event::ManualResetEvent;

/// The subject binary's path: `cargo` only exposes CARGO_BIN_EXE_* for
/// bins of the SAME package, so resolve the sibling bin from the
/// workspace layout (current_exe = target/<profile>/deps/... ->
/// target/<profile>/ramshaper-subject.exe). Build it first
/// (`cargo build -p ramshaper-subject`).
fn subject_exe() -> PathBuf {
    let exe = std::env::current_exe().expect("current test exe");
    let profile_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/<profile> directory");
    let path = profile_dir.join(format!("ramshaper-subject{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.exists(),
        "the subject binary is not built (run `cargo build -p ramshaper-subject`): {}",
        path.display()
    );
    path
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ramshaper-trial-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn write_spec(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("spec.json");
    std::fs::write(
        &path,
        r#"{"events_per_run": 100, "warmup_events": 2, "task_iters": 1000, "seed": 9}"#,
    )
    .expect("write spec");
    path
}

/// A scripted [`ramshaper_trial::AgentRpc`]: answers the three policy
/// verbs the way an enabled, accepting gate would, and records every call
/// with its payload. The apply/restore MECHANICS behind these answers are
/// proven live in the agent crate's own tests (which inject Elevated
/// pressure through the in-process gate API — the wire gate here would
/// honestly refuse on this unpressured machine, and fabricating memory
/// pressure during the AT-06 soak is off-limits).
struct FakeRpc {
    calls: std::sync::Mutex<Vec<(String, String)>>,
}

impl FakeRpc {
    fn new() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn commands(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("no poisoning")
            .iter()
            .map(|(command, _)| command.clone())
            .collect()
    }
}

impl ramshaper_trial::AgentRpc for FakeRpc {
    fn call(
        &self,
        request_id: &str,
        command: &str,
        _payload: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        self.calls
            .lock()
            .expect("no poisoning")
            .push((command.to_string(), request_id.to_string()));
        match command {
            ramshaper_core::ipc::COMMAND_PREVIEW_ACTION => Ok(serde_json::json!({
                "verdict": "offered", "action_id": "fake", "ttl_ms": 30000
            })),
            ramshaper_core::ipc::COMMAND_CONFIRM_ACTION => {
                Ok(serde_json::json!({ "outcome": "applied", "deadline_ms": 1 }))
            }
            ramshaper_core::ipc::COMMAND_RESTORE_ACTION => {
                Ok(serde_json::json!({ "outcome": "restored", "wrote_back": true }))
            }
            other => Err(format!("fake rpc: unexpected command {other}")),
        }
    }
}

#[test]
fn a_treated_block_applies_runs_and_restores_end_to_end() {
    let dir = temp_dir("happy");
    let spec_path = write_spec(&dir);
    let spec = ramshaper_subject::load_spec(&spec_path).expect("valid spec");
    let parsed = ramshaper_trial::Parsed {
        pipe_name: "unused-by-the-fake".to_string(),
        subject: subject_exe(),
        spec: spec_path.clone(),
        blocks: 1,
        seed: 11,
        out: dir.join("blocks.json"),
        run_prefix: "trial".to_string(),
        exit_delay_ms: 1200,
    };
    let rpc = FakeRpc::new();

    let blocks = ramshaper_trial::run_trial(&rpc, &parsed).expect("the trial completes");
    assert_eq!(blocks.len(), 1);
    let block = &blocks[0];
    assert_eq!(block.block_id, 1);
    assert_eq!(block.baseline.events.len(), 100);
    assert_eq!(block.treatment.events.len(), 100);
    assert_eq!(block.baseline.run_id, "trialb1-A");
    assert_eq!(block.treatment.run_id, "trialb1-B");

    // The treatment ran through all three policy verbs, in order, with
    // the deterministic action id bound into the request ids; the
    // baseline run issued NOTHING (observe stays observe).
    let commands = rpc.commands();
    assert_eq!(
        commands,
        vec![
            "PreviewAction".to_string(),
            "ConfirmAction".to_string(),
            "RestoreAction".to_string(),
        ],
        "exactly the treatment's three verbs: {commands:?}"
    );

    // The wire shape is analyze-ab compatible.
    let wire = ramshaper_trial::blocks_to_wire(&blocks);
    assert_eq!(wire[0]["block_id"], 1);
    assert!(wire[0]["baseline"]["events"]
        .as_array()
        .expect("series")
        .iter()
        .all(|v| v.is_number()));
    let _ = spec;

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_refused_treatment_fails_the_trial_and_runs_nothing_after() {
    // The gate refuses the preview: the trial must stop at that run —
    // the treated run reports the refusal, no blocks come back, and the
    // driver never restores what it did not apply.
    struct RefusingRpc;
    impl ramshaper_trial::AgentRpc for RefusingRpc {
        fn call(
            &self,
            _request_id: &str,
            command: &str,
            _payload: Option<serde_json::Value>,
        ) -> Result<serde_json::Value, String> {
            assert_eq!(command, ramshaper_core::ipc::COMMAND_PREVIEW_ACTION);
            Err("policy_disabled: the policy is disabled for this session".to_string())
        }
    }
    let dir = temp_dir("refused");
    let spec_path = write_spec(&dir);
    let parsed = ramshaper_trial::Parsed {
        pipe_name: "unused".to_string(),
        subject: subject_exe(),
        spec: spec_path,
        blocks: 1,
        seed: 3,
        out: dir.join("blocks.json"),
        run_prefix: "trial".to_string(),
        exit_delay_ms: 0,
    };
    let error = ramshaper_trial::run_trial(&RefusingRpc, &parsed)
        .expect_err("a refused preview fails the trial");
    assert!(
        error.contains("policy_disabled"),
        "the typed refusal is carried: {error}"
    );
    assert!(
        !parsed.out.exists(),
        "no blocks file for a half-executed experiment"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_gateless_agent_refuses_the_trial_without_a_partial_blocks_file() {
    let dir = temp_dir("gateless");
    let spec = write_spec(&dir);
    let pipe_name = format!(r"\\.\pipe\ramshaper-trial-gateless-{}", std::process::id());
    // A PRODUCTION agent: no --policy-journal => no gate => PreviewAction
    // answers Unsupported.
    let stop = Arc::new(ManualResetEvent::new().expect("stop event"));
    let handle = serve(
        Arc::clone(&stop),
        pipe_name.clone(),
        WatchConfig::new(500).expect("cadence"),
        ConfigSummary {
            mode: "observe_only".to_string(),
            profile: "balanced".to_string(),
            exclusions: Vec::new(),
        },
        None,
    )
    .expect("agent serves");
    std::thread::sleep(std::time::Duration::from_millis(300));

    let out_path = dir.join("blocks.json");
    let out = Command::new(env!("CARGO_BIN_EXE_ramshaper-trial"))
        .args([
            "--pipe-name",
            &pipe_name,
            "--subject",
            subject_exe().to_str().expect("utf-8 path"),
            "--spec",
            spec.to_str().expect("utf-8 path"),
            "--blocks",
            "1",
            "--out",
            out_path.to_str().expect("utf-8 path"),
        ])
        .output()
        .expect("run the trial driver");
    assert_eq!(
        out.status.code(),
        Some(1),
        "the trial must stop honestly: {:?}",
        out.stderr
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Unsupported") || stderr.contains("trial stopped"),
        "the refusal is explained: {stderr}"
    );
    assert!(
        !out_path.exists(),
        "no blocks file for a half-executed experiment"
    );

    handle.stop();
    std::fs::remove_dir_all(&dir).ok();
}

/// Sanity on the subject's hold protocol itself (no agent involved): the
/// driver's ready-line wait must work against the real binary.
#[test]
fn the_subject_holds_until_go() {
    let dir = temp_dir("hold");
    let spec = write_spec(&dir);
    let mut child = Command::new(subject_exe().to_str().expect("utf-8 path"))
        .args([
            "--spec",
            spec.to_str().expect("utf-8 path"),
            "--run-id",
            "hold-check",
            "--hold-until-go",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn held subject");
    let mut stdout = child.stdout.take().expect("piped");
    {
        let mut line = String::new();
        std::io::BufReader::new(&mut stdout)
            .read_line(&mut line)
            .expect("read ready line");
        assert_eq!(line.trim(), "SUBJECT_READY");
    }
    // Keep `stdout` alive until the child finishes: dropping the read end
    // early would break the subject's record println (a real bug class —
    // the driver itself holds the pipe open through run_once).
    let mut stdin = child.stdin.take().expect("piped stdin");
    stdin.write_all(b"GO\n").expect("release");
    drop(stdin);
    let mut record_text = String::new();
    stdout
        .read_to_string(&mut record_text)
        .expect("read the record");
    let status = child.wait().expect("finish");
    assert_eq!(status.code(), Some(0));
    let record: serde_json::Value = serde_json::from_str(record_text.trim()).expect("valid record");
    assert_eq!(record["completion"]["status"], "complete");
    std::fs::remove_dir_all(&dir).ok();
}
