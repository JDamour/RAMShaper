//! Binary-level CLI tests: run the real `ramshaper-cli` executable and check
//! stdout/stderr separation, JSON shape and exit codes (contract 14 §8).

#![cfg(windows)]

use std::process::{Command, Stdio};

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_ramshaper-cli")
}

fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(exe())
        .args(args)
        .output()
        .expect("run ramshaper-cli");
    (
        output.status.code().expect("exit code"),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn version_prints_and_exits_zero() {
    let (code, stdout, stderr) = run(&["--version"]);
    assert_eq!(code, 0);
    assert!(stdout.starts_with("ramshaper-cli "), "stdout: {stdout}");
    assert!(!stdout.contains('\n') || stdout.ends_with('\n'));
    assert!(stderr.is_empty(), "stderr must stay empty: {stderr}");
}

#[test]
fn help_lists_commands_and_exits_zero() {
    let (code, stdout, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("info"));
    assert!(stdout.contains("low-memory-watch"));
    assert!(stdout.contains("EXIT CODES"));
}

#[test]
fn usage_error_writes_stderr_only_and_exits_two() {
    let (code, stdout, stderr) = run(&["not-a-command"]);
    assert_eq!(code, 2);
    assert!(
        stdout.is_empty(),
        "usage errors must not write stdout: {stdout}"
    );
    assert!(stderr.contains("unknown command"), "stderr: {stderr}");
}

#[test]
fn bad_interval_is_usage_error() {
    let (code, stdout, stderr) = run(&["low-memory-watch", "--interval-ms", "1"]);
    assert_eq!(code, 2);
    assert!(stdout.is_empty());
    assert!(stderr.contains("out of range"), "stderr: {stderr}");
}

#[test]
fn info_text_reports_metrics_with_units() {
    let (code, stdout, stderr) = run(&["info"]);
    assert!(code == 0 || code == 3, "exit {code}, stderr: {stderr}");
    assert!(stdout.contains("Physical memory"), "{stdout}");
    assert!(stdout.contains("System commit"), "{stdout}");
    assert!(stdout.contains("Low-memory notification"), "{stdout}");
    assert!(stdout.contains("GiB"), "{stdout}");
    assert!(stdout.contains("bytes)"), "{stdout}");
    assert!(stdout.contains("does not by itself prove"), "{stdout}");
}

#[test]
fn info_json_is_one_versioned_object_with_contract_fields() {
    let (code, stdout, stderr) = run(&["info", "--json"]);
    assert!(code == 0 || code == 3, "exit {code}, stderr: {stderr}");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(value["schema"], "ramshaper.snapshot.v1");
    assert!(value["generated_at_utc"]
        .as_str()
        .is_some_and(|t| t.ends_with('Z')));
    assert!(value["agent_instance_id"]
        .as_str()
        .is_some_and(|t| t.starts_with("cli-")));
    assert!(value["sample_sequence"].is_u64());
    assert!(value["os"]["version"].as_str().is_some());
    assert!(value["tool"]["name"] == "ramshaper-cli");
    for key in [
        "physical_total_bytes",
        "physical_available_bytes",
        "physical_in_use_bytes",
        "commit_used_bytes",
        "commit_limit_bytes",
        "commit_headroom_bytes",
    ] {
        let metric = &value["metrics"][key];
        assert_eq!(metric["status"], "ok", "{key}: {metric}");
        assert!(
            metric["value"].is_u64(),
            "{key} must be an integer: {metric}"
        );
        assert!(metric["source"] == "GetPerformanceInfo");
        assert!(metric["observed_at_utc"]
            .as_str()
            .is_some_and(|t| t.ends_with('Z')));
        assert!(metric["sample_sequence"].is_u64());
        assert!(metric["age_ms"].is_u64());
    }
    let low = &value["metrics"]["low_memory"];
    assert_eq!(low["status"], "ok", "low_memory: {low}");
    assert!(
        low["state"] == "not_signaled" || low["state"] == "signaled",
        "low_memory state: {low}"
    );
}

#[test]
fn watch_json_emits_ndjson_lines_before_kill() {
    // Start the real watch, read one JSON line, then terminate the process.
    // This proves line emission and flushing; clean Ctrl+C exit 0 is covered
    // by the in-process integration tests in ramshaper-windows.
    use std::io::{BufRead, BufReader};
    let mut child = Command::new(exe())
        .args(["low-memory-watch", "--json", "--interval-ms", "200"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn watch");
    let mut reader = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("read first NDJSON line");
    let _ = child.kill();
    let _ = child.wait();
    assert!(!line.is_empty(), "watch must emit a first line immediately");
    let value: serde_json::Value = serde_json::from_str(line.trim()).expect("line is JSON");
    assert_eq!(value["schema"], "ramshaper.snapshot.v1");
    assert!(value["metrics"]["low_memory"]["status"].is_string());
}

#[test]
fn info_json_physical_total_is_plausible_for_this_machine() {
    let (code, stdout, _) = run(&["info", "--json"]);
    assert!(code == 0 || code == 3);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let total = value["metrics"]["physical_total_bytes"]["value"]
        .as_u64()
        .expect("total bytes");
    // Between 1 GiB and 4 TiB on any realistic desktop machine.
    assert!((1024 * 1024 * 1024..=4 * 1024 * 1024 * 1024 * 1024).contains(&total));
}

#[test]
fn processes_text_lists_processes_with_headers() {
    let (code, stdout, stderr) = run(&["processes", "--interval-ms", "200"]);
    assert!(code == 0 || code == 3, "exit {code}, stderr: {stderr}");
    assert!(stdout.contains("Processes ("), "{stdout}");
    assert!(stdout.contains("pid"), "{stdout}");
    assert!(stdout.contains("workingset"), "{stdout}");
    assert!(stdout.contains("process I/O is process"), "{stdout}");
    // The rate window is reported.
    assert!(stdout.contains("rate window"), "{stdout}");
}

#[test]
fn processes_json_has_contract_shape() {
    let (code, stdout, stderr) = run(&["processes", "--json", "--interval-ms", "200"]);
    assert!(code == 0 || code == 3, "exit {code}, stderr: {stderr}");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(value["schema"], "ramshaper.processes.v1");
    assert!(value["logical_cpu_count"].as_u64().expect("cpus") > 0);
    assert!(value["rate_interval_ms"].as_u64().expect("window") >= 200);
    let list = value["processes"].as_array().expect("process array");
    assert!(list.len() > 20, "only {} processes", list.len());
    // Every entry carries an exact identity with a decimal-string FILETIME.
    for entry in list {
        assert!(entry["identity"]["pid"].is_u64());
        assert!(entry["identity"]["creation_filetime_100ns"]
            .as_str()
            .is_some_and(|s| { s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() }));
    }
    // The self process appears with real byte counters somewhere in the list.
    assert!(list.iter().any(|entry| {
        entry["working_set"]["status"] == "ok"
            && entry["working_set"]["value"]
                .as_u64()
                .is_some_and(|v| v > 0)
    }));
}

#[test]
fn invalid_config_file_is_exit_two() {
    // Write a config with an unknown field; the CLI must reject it (AT-08).
    let dir = std::env::temp_dir().join(format!("ramshaper-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let bad = dir.join("bad.toml");
    std::fs::write(&bad, "schema_version = 1\nno_such_field = 3\n").expect("write");
    let (code, stdout, stderr) = run(&["--config", bad.to_string_lossy().as_ref(), "info"]);
    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(stdout.is_empty());
    assert!(stderr.contains("unknown field"), "stderr: {stderr}");

    let future = dir.join("future.toml");
    std::fs::write(&future, "schema_version = 2\n").expect("write");
    let (code, _, stderr) = run(&["--config", future.to_string_lossy().as_ref(), "info"]);
    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(
        stderr.contains("unsupported schema_version"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn watch_interval_flag_reaches_the_watch_banner() {
    // Regression (M1 increment): --interval-ms was parsed, validated and then
    // dropped for low-memory-watch, so the banner always reported the config
    // default (1000 ms) no matter what the flag said. The flag must win over
    // the built-in default; the config file variant is covered by
    // valid_config_sets_watch_interval below.
    use std::process::{Command as OsCommand, Stdio};
    let mut child = OsCommand::new(exe())
        .args(["low-memory-watch", "--interval-ms", "300", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watch");
    std::thread::sleep(std::time::Duration::from_millis(900));
    let _ = child.kill();
    let output = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("every 300 ms"), "stderr: {stderr}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // ~3 samples at 300 ms in 900 ms; at least 2 emitted lines.
    assert!(stdout.lines().count() >= 2, "stdout: {stdout}");
}

#[test]
fn valid_config_sets_watch_interval() {
    let dir = std::env::temp_dir().join(format!("ramshaper-test-cfg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let config = dir.join("watch.toml");
    std::fs::write(
        &config,
        "schema_version = 1\nmode = \"observe_only\"\n[watch]\ninterval_ms = 300\nretention_ms = 600000\n",
    )
    .expect("write");
    // Watch prints its interval on stderr; verify and cancel via kill.

    use std::process::{Command as OsCommand, Stdio};
    let mut child = OsCommand::new(exe())
        .args([
            "--config",
            config.to_string_lossy().as_ref(),
            "low-memory-watch",
            "--json",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watch");
    std::thread::sleep(std::time::Duration::from_millis(900));
    let _ = child.kill();
    let output = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("every 300 ms"), "stderr: {stderr}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // ~3 samples at 300 ms in 900 ms; at least 2 emitted lines.
    assert!(stdout.lines().count() >= 2, "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn info_json_includes_pressure_section() {
    let (code, stdout, stderr) = run(&["info", "--json"]);
    assert!(code == 0 || code == 3, "exit {code}, stderr: {stderr}");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(value["pressure"]["rule_version"], "pressure_v1");
    // A single observation cannot classify; state must be unknown.
    assert_eq!(value["pressure"]["level"], "unknown");
}

/// RS-012: config export/import are inert, schema-validated file operations.
#[cfg(test)]
mod config_tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("rs012-{}-{name}", std::process::id()));
        p
    }

    fn cleanup(paths: &[PathBuf]) {
        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn export_then_import_roundtrip_is_lossless() {
        let a = temp_path("export.toml");
        let b = temp_path("imported.toml");
        cleanup(&[a.clone(), b.clone()]);
        let (code, stdout, stderr) = run(&["config", "export", a.to_str().expect("rs012 test")]);
        assert_eq!(code, 0, "stderr: {stderr}");
        assert!(stdout.contains("exported validated configuration"));
        let (code, stdout, stderr) = run(&[
            "config",
            "import",
            a.to_str().expect("rs012 test"),
            "--into",
            b.to_str().expect("rs012 test"),
        ]);
        assert_eq!(code, 0, "stderr: {stderr}");
        assert!(stdout.contains("imported validated configuration"));
        assert!(stdout.contains("profile: balanced"));
        // Canonical serialization is deterministic: identical bytes.
        let ca = std::fs::read_to_string(&a).expect("rs012 test");
        let cb = std::fs::read_to_string(&b).expect("rs012 test");
        assert_eq!(ca, cb);
        cleanup(&[a, b]);
    }

    #[test]
    fn export_roundtrips_a_custom_source() {
        let src = temp_path("custom-src.toml");
        let dest = temp_path("custom-out.toml");
        cleanup(&[src.clone(), dest.clone()]);
        std::fs::write(
            &src,
            "schema_version = 1\n[watch]\ninterval_ms = 2500\n[profiles]\nactive = \"low_ram\"\n[exclusions]\nprocesses = [\"obs64.exe\"]\n",
        )
        .expect("rs012 test");
        let (code, _, stderr) = run(&[
            "config",
            "export",
            "--config",
            src.to_str().expect("rs012 test"),
            dest.to_str().expect("rs012 test"),
        ]);
        assert_eq!(code, 0, "stderr: {stderr}");
        let text = std::fs::read_to_string(&dest).expect("rs012 test");
        assert!(text.contains("interval_ms = 2500"), "text: {text}");
        assert!(text.contains("low_ram"), "text: {text}");
        assert!(text.contains("obs64.exe"), "text: {text}");
        // The exported file itself validates as an import source.
        let (code, _, stderr) = run(&[
            "config",
            "import",
            dest.to_str().expect("rs012 test"),
            "--into",
            src.to_str().expect("rs012 test"),
        ]);
        assert_eq!(code, 0, "stderr: {stderr}");
        cleanup(&[src, dest]);
    }

    #[test]
    fn import_rejects_invalid_source_and_keeps_dest() {
        let dest = temp_path("keepme.toml");
        cleanup(std::slice::from_ref(&dest));
        std::fs::write(&dest, "schema_version = 1\n").expect("rs012 test");
        let src = temp_path("bad.toml");
        std::fs::write(&src, "schema_version = 1\n[profiles]\nactive = \"turbo\"\n")
            .expect("rs012 test");
        let (code, _, stderr) = run(&[
            "config",
            "import",
            src.to_str().expect("rs012 test"),
            "--into",
            dest.to_str().expect("rs012 test"),
        ]);
        assert_eq!(code, 2);
        assert!(
            stderr.contains("unsupported profile 'turbo'"),
            "stderr: {stderr}"
        );
        // Retain the last valid configuration: DEST is untouched.
        assert_eq!(
            std::fs::read_to_string(&dest).expect("rs012 test"),
            "schema_version = 1\n"
        );
        cleanup(&[dest, src]);
    }

    #[test]
    fn import_rejects_missing_into_and_policy_mode() {
        let src = temp_path("ok.toml");
        std::fs::write(&src, "schema_version = 1\n").expect("rs012 test");
        let (code, _, stderr) = run(&["config", "import", src.to_str().expect("rs012 test")]);
        assert_eq!(code, 2);
        assert!(stderr.contains("--into"), "stderr: {stderr}");
        let policy = temp_path("policy.toml");
        std::fs::write(&policy, "schema_version = 1\nmode = \"policy\"\n").expect("rs012 test");
        let dest = temp_path("out.toml");
        let (code, _, stderr) = run(&[
            "config",
            "import",
            policy.to_str().expect("rs012 test"),
            "--into",
            dest.to_str().expect("rs012 test"),
        ]);
        assert_eq!(code, 2);
        assert!(stderr.contains("unsupported mode"), "stderr: {stderr}");
        assert!(!dest.exists(), "a rejected import must not create DEST");
        cleanup(&[src, policy, dest]);
    }

    #[test]
    fn config_usage_errors_and_help() {
        let (code, _, stderr) = run(&["config"]);
        assert_eq!(code, 2);
        assert!(stderr.contains("action"), "stderr: {stderr}");
        let (code, _, stderr) = run(&["config", "bogus"]);
        assert_eq!(code, 2);
        assert!(stderr.contains("unknown config action"), "stderr: {stderr}");
        let (code, _, _) = run(&["config", "export"]);
        assert_eq!(code, 2);
        let (code, _, stderr) = run(&["config", "export", "x", "--into", "y"]);
        assert_eq!(code, 2);
        assert!(stderr.contains("--into"), "stderr: {stderr}");
        // The help documents the subcommand and both actions.
        let (code, stdout, _) = run(&["--help"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("config export"), "stdout: {stdout}");
        assert!(stdout.contains("config import"), "stdout: {stdout}");
    }
}

#[test]
fn policies_dry_run_json_is_schema_shaped_and_deterministic() {
    let (code, stdout, stderr) = run(&["policies", "--dry-run", "--json"]);
    assert_eq!(code, 0, "the dry run cannot fail at runtime");
    assert!(stderr.is_empty(), "stdout carries data only: {stderr}");
    assert!(
        stdout.contains("ramshaper.policy-dryrun.v1"),
        "versioned schema"
    );
    assert!(stdout.contains("\"previews_offered\""));
    assert!(stdout.contains("dry-1"), "the deterministic action ids");
    // Byte-identical on a second run (no clock, no OS input).
    let (code2, stdout2, _) = run(&["policies", "--dry-run", "--json"]);
    assert_eq!(code2, 0);
    assert_eq!(
        stdout, stdout2,
        "the same trace must print byte-identical output"
    );
}

#[test]
fn policies_dry_run_text_carries_the_synthetic_banner_and_summary() {
    let (code, stdout, stderr) = run(&["policies", "--dry-run"]);
    assert_eq!(code, 0);
    assert!(stderr.is_empty(), "stdout carries data only: {stderr}");
    assert!(
        stdout.to_uppercase().contains("SYNTHETIC"),
        "the synthetic marking must be in the output: {stdout}"
    );
    assert!(stdout.contains("schema ramshaper.policy-dryrun.v1"));
    assert!(stdout.contains("no_action pressure_unknown"), "a deny step");
    assert!(stdout.contains("preview dry-1"), "an offered preview");
    assert!(stdout.contains("summary:"), "the aggregate counts");
}

#[test]
fn policies_usage_errors_exit_two() {
    let (code, stdout, stderr) = run(&["policies"]);
    assert_eq!(code, 2, "missing --dry-run is a usage error");
    assert!(stdout.is_empty(), "errors never go to stdout: {stdout}");
    assert!(stderr.contains("--dry-run"), "{stderr}");
    let (code, _, stderr) = run(&["--dry-run", "info"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("--dry-run"), "{stderr}");
}
