//! End-to-end tests over the real `ramshaper-subject` binary: the
//! orchestrator's contract is spawn → spec JSON → one run record on
//! stdout, so that is exactly what these tests drive.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_ramshaper-subject")
}

fn write_spec(dir: &std::path::Path, name: &str, json: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, json).expect("write spec file");
    path
}

#[test]
fn a_run_emits_one_v1_record_matching_the_spec() {
    let dir = std::env::temp_dir().join(format!(
        "ramshaper-subject-{}-{}",
        line!(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = write_spec(
        &dir,
        "ok.json",
        r#"{"events_per_run": 120, "warmup_events": 2, "task_iters": 1000, "seed": 7}"#,
    );
    let out = Command::new(bin())
        .args([
            "--spec",
            spec.to_str().expect("utf-8 path"),
            "--run-id",
            "block2-B",
        ])
        .output()
        .expect("run the subject binary");
    assert_eq!(out.status.code(), Some(0), "{:?}", out.stderr);
    let record: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("one JSON object on stdout");
    assert_eq!(record["schema"], "ramshaper.subject-run.v1");
    assert_eq!(record["run_id"], "block2-B");
    assert_eq!(record["spec"]["events_per_run"], 120);
    assert_eq!(record["completion"]["status"], "complete");
    let latencies = record["latencies_ms"].as_array().expect("series");
    assert_eq!(
        latencies.len(),
        120,
        "warmup (2) excluded from the series by design"
    );
    assert!(
        latencies
            .iter()
            .all(|v| v.as_f64().is_some_and(|ms| ms >= 0.0)),
        "every measured latency is a non-negative number"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_below_minimum_spec_is_a_usage_error_and_runs_nothing() {
    let dir = std::env::temp_dir().join(format!(
        "ramshaper-subject-{}-{}",
        line!(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = write_spec(&dir, "small.json", r#"{"events_per_run": 50}"#);
    let out = Command::new(bin())
        .args(["--spec", spec.to_str().expect("utf-8 path")])
        .output()
        .expect("run the subject binary");
    assert_eq!(out.status.code(), Some(2), "doc-08 minimum violation");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("descriptive-p95 minimum"),
        "the bench validator's reason is carried verbatim: {:?}",
        out.stderr
    );
    assert!(
        out.stdout.is_empty(),
        "a refused spec must not emit a run record"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_closed_schema_refuses_unknown_fields() {
    let dir = std::env::temp_dir().join(format!(
        "ramshaper-subject-{}-{}",
        line!(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = write_spec(
        &dir,
        "bogus.json",
        r#"{"events_per_run": 100, "priority_boost": true}"#,
    );
    let out = Command::new(bin())
        .args(["--spec", spec.to_str().expect("utf-8 path")])
        .output()
        .expect("run the subject binary");
    assert_eq!(out.status.code(), Some(2));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_missing_spec_file_is_a_usage_error() {
    let out = Command::new(bin())
        .args(["--spec", "Z:/definitely/not/there.json"])
        .output()
        .expect("run the subject binary");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn the_record_survives_a_round_trip_through_the_analysis_event_shape() {
    // The orchestrator (ticket 3) will feed these latencies to
    // `analyze-ab`-style pairing; the subject's record must carry plain
    // numbers, not wrapped status objects, for every measured event.
    let dir = std::env::temp_dir().join(format!(
        "ramshaper-subject-{}-{}",
        line!(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = write_spec(&dir, "ok.json", r#"{"events_per_run": 100}"#);
    let out = Command::new(bin())
        .args(["--spec", spec.to_str().expect("utf-8 path")])
        .output()
        .expect("run the subject binary");
    assert_eq!(out.status.code(), Some(0));
    let record: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    let latencies = record["latencies_ms"].as_array().expect("series");
    assert_eq!(latencies.len(), 100);
    assert!(latencies.iter().all(|v| v.is_number()));
    std::fs::remove_dir_all(&dir).ok();
}
