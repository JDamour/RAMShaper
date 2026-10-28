//! `ramshaper-subject` — the Stage B trial subject (RS-024 Stage B;
//! design memo 2026-10-28, follow-up ticket 2; hold-mode is ticket 3).
//!
//! A real consented trial needs a subject process that is separate from
//! the driver: the bench process self-identifies as `protected_ramshaper`
//! (ineligible by construction), and measuring inside the treated process
//! is poor methodology. This binary is that subject: it runs the SAME
//! bounded-CPU task-latency workload as `ramshaper-bench::workload`
//! (spec fidelity by construction — it calls `run_workload` itself,
//! through the [`ramshaper_subject`] library), measures its own per-task
//! latencies, and emits one JSON run record on stdout for the orchestrator
//! to aggregate.
//!
//! Safety shape: the subject is **inert**. It is always observe-only (the
//! treatment closure is a no-op — in a real trial the agent applies the
//! priority change externally, through its consent gate; the subject
//! never knows and never acts). It performs no IPC, no OS mutation, no
//! file writes beyond stdout, and its image name is deliberately NOT in
//! the agent's self-identification list (it is a workload simulator, the
//! *subject* of the experiment — the reasoning is recorded in the design
//! memo and lands in ADR-045 when the owner picks it).
//!
//! Trial sequencing (ticket 3): `--hold-until-go` makes the spawned
//! subject announce `SUBJECT_READY` and wait for a `GO` line on stdin, so
//! the driver can Preview/Confirm the priority change BEFORE the measured
//! loop runs (a mid-run apply would straddle the latencies). EOF on
//! stdin exits 1 — an orphaned held subject never hangs forever, and
//! never runs untreated under a dead driver. `--exit-delay-ms` keeps the
//! process alive after the record is printed so the driver's restore
//! lands while the subject still runs (journal `Restored`, not
//! `TargetGone`).

#![deny(unsafe_code)]

use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

use ramshaper_subject::{execute, load_spec, SubjectSpec, SUBJECT_GO_MARKER, SUBJECT_READY_MARKER};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TOOL_NAME: &str = "ramshaper-subject";

const HELP: &str = "\
ramshaper-subject — Stage B trial subject (runs the declared workload, emits its own latencies)

USAGE:
    ramshaper-subject --spec <SPEC.json> [--run-id <ID>]
                      [--hold-until-go] [--exit-delay-ms <MS>]

    Runs one bounded-CPU workload run exactly as specified (the same
    workload and doc-08 minimums as ramshaper-bench workload-ab) and
    prints one JSON run record (schema ramshaper.subject-run.v1) with the
    per-task latency series in milliseconds. Warmup tasks execute but are
    excluded from the series by design (doc 08 §3).

    The subject is observe-only BY ITSELF: any treatment (a priority
    change) is applied externally by the consent-gated agent; this binary
    performs no IPC and no OS mutation.

OPTIONS:
    --spec <SPEC.json>   The workload spec (JSON; fields events_per_run,
                         warmup_events, task_iters, seed — same shape and
                         minimums as ramshaper-bench's WorkloadSpec)
    --run-id <ID>        Run identifier echoed in the record (default:
                         \"subject\")
    --hold-until-go      Print SUBJECT_READY and wait for a GO line on
                         stdin before running (the trial driver applies
                         the treatment between READY and GO); stdin EOF
                         exits 1 without running
    --exit-delay-ms <MS> Stay alive this long AFTER printing the record
                         (default 0) so the driver's restore lands while
                         the process still runs; range 0..=60000
    --version, -V        Print version
    --help, -h           Print this help

EXIT CODES:
    0  the run completed its full schedule
    1  the run stopped early, or a held subject saw stdin EOF before GO
    2  usage or spec error (nothing ran)
";

#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub spec_path: PathBuf,
    pub run_id: String,
    pub hold_until_go: bool,
    pub exit_delay_ms: u64,
}

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut spec_path: Option<PathBuf> = None;
    let mut run_id = "subject".to_string();
    let mut hold_until_go = false;
    let mut exit_delay_ms = 0u64;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("{TOOL_NAME} {VERSION}");
                std::process::exit(0);
            }
            "--help" | "-h" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--spec" => match iter.next() {
                Some(value) => spec_path = Some(PathBuf::from(value)),
                None => return Err("--spec requires a path".to_string()),
            },
            "--run-id" => match iter.next() {
                Some(value) => run_id = value,
                None => return Err("--run-id requires a value".to_string()),
            },
            "--hold-until-go" => hold_until_go = true,
            "--exit-delay-ms" => match iter.next() {
                Some(value) => match value.parse::<u64>() {
                    Ok(ms) if ms <= 60_000 => exit_delay_ms = ms,
                    Ok(ms) => return Err(format!("--exit-delay-ms {ms} out of range (0..=60000)")),
                    Err(_) => return Err(format!("--exit-delay-ms expects a number, got {value}")),
                },
                None => return Err("--exit-delay-ms requires a value".to_string()),
            },
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let Some(spec_path) = spec_path else {
        return Err("missing --spec <SPEC.json>".to_string());
    };
    Ok(Parsed {
        spec_path,
        run_id,
        hold_until_go,
        exit_delay_ms,
    })
}

/// Wait for the driver's go signal. `true` = run; `false` = stdin closed
/// before GO (an orphaned subject must not hang, and must not run
/// untreated under a driver that died — the driver would have to redo
/// the whole run).
fn wait_for_go() -> bool {
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => false,
        Ok(_) => line.trim() == SUBJECT_GO_MARKER,
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match parse_args(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("run '{TOOL_NAME} --help' for details");
            return ExitCode::from(2);
        }
    };
    let spec: SubjectSpec = match load_spec(&parsed.spec_path) {
        Ok(spec) => spec,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };
    if let Err(message) = spec.validate() {
        eprintln!("error: invalid spec: {message}");
        return ExitCode::from(2);
    }

    if parsed.hold_until_go {
        println!("{SUBJECT_READY_MARKER}");
        let _ = std::io::stdout().flush();
        if !wait_for_go() {
            eprintln!("error: stdin closed before the go signal; nothing ran");
            return ExitCode::from(1);
        }
    }

    let record = execute(&spec, &parsed.run_id);
    match serde_json::to_string_pretty(&record) {
        Ok(out) => println!("{out}"),
        Err(error) => {
            eprintln!("error: failed to serialize the run record: {error}");
            return ExitCode::from(1);
        }
    }
    let _ = std::io::stdout().flush();
    if parsed.exit_delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(parsed.exit_delay_ms));
    }
    // The exit code mirrors the completion status (doc 08: early stops
    // are recorded separately, never hidden).
    match record.completion.get("status").and_then(|s| s.as_str()) {
        Some("complete") => ExitCode::SUCCESS,
        _ => ExitCode::from(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_rules() {
        assert!(parse_args(vec![]).is_err(), "missing --spec is an error");
        assert!(parse_args(vec!["--spec".to_string()]).is_err());
        assert!(parse_args(vec!["bogus".to_string()]).is_err());
        assert!(parse_args(vec!["--exit-delay-ms".to_string()]).is_err());
        assert!(parse_args(vec!["--exit-delay-ms".to_string(), "60001".to_string()]).is_err());
        let parsed = parse_args(vec![
            "--spec".to_string(),
            "s.json".to_string(),
            "--hold-until-go".to_string(),
            "--exit-delay-ms".to_string(),
            "800".to_string(),
        ])
        .expect("parses");
        assert!(parsed.hold_until_go);
        assert_eq!(parsed.exit_delay_ms, 800);
        assert_eq!(parsed.run_id, "subject");
    }
}
