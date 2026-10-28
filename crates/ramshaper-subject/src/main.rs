//! `ramshaper-subject` — the Stage B trial subject (RS-024 Stage B;
//! design memo 2026-10-28, follow-up ticket 2).
//!
//! A real consented trial needs a subject process that is separate from
//! the driver: the bench process self-identifies as `protected_ramshaper`
//! (ineligible by construction), and measuring inside the treated process
//! is poor methodology. This binary is that subject: it runs the SAME
//! bounded-CPU task-latency workload as `ramshaper-bench::workload`
//! (spec fidelity by construction — it calls `run_workload` itself),
//! measures its own per-task latencies, and emits one JSON run record on
//! stdout for the orchestrator to aggregate.
//!
//! Safety shape: the subject is **inert**. It is always observe-only (the
//! treatment closure is a no-op — in a real trial the agent applies the
//! priority change externally, through its consent gate; the subject
//! never knows and never acts). It performs no IPC, no OS mutation, no
//! file writes beyond stdout, and its image name is deliberately NOT in
//! the agent's self-identification list (it is a workload simulator, the
//! *subject* of the experiment — the reasoning is recorded in the design
//! memo and lands in ADR-045 when the owner picks it). Nothing in the
//! shipped surface references this binary yet: it is built ahead of
//! ADR-045 so the decision, once made, has working machinery behind it.
//!
//! The spec mirror: `ramshaper-bench`'s types carry no serde derives and
//! the bench crate is frozen while the AT-06 soak holds its release
//! binary, so this crate declares its own `SubjectSpec` with identical
//! field names and the same doc-08 validation, then constructs the real
//! `WorkloadSpec` from it. The orchestrator writes the same JSON for
//! both.

#![deny(unsafe_code)]

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use ramshaper_bench::workload::{run_workload, Completion, WorkloadSpec};
use serde::Deserialize;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TOOL_NAME: &str = "ramshaper-subject";
/// The run-record schema tag.
pub const SUBJECT_RUN_SCHEMA: &str = "ramshaper.subject-run.v1";

const HELP: &str = "\
ramshaper-subject — Stage B trial subject (runs the declared workload, emits its own latencies)

USAGE:
    ramshaper-subject --spec <SPEC.json> [--run-id <ID>]

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
    --version, -V        Print version
    --help, -h           Print this help

EXIT CODES:
    0  the run completed its full schedule
    1  the run stopped early (recorded in the JSON completion field)
    2  usage or spec error (nothing ran)
";

/// The spec mirror of `ramshaper_bench::workload::WorkloadSpec` (identical
/// field names and minimums; see the module docs for why it is mirrored).
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectSpec {
    /// Measured tasks per run (analyzed series length).
    pub events_per_run: usize,
    /// Warmup tasks before the measured series (excluded by design).
    #[serde(default = "default_warmup")]
    pub warmup_events: usize,
    /// Per-task bounded compute effort: internal loop iterations.
    #[serde(default = "default_task_iters")]
    pub task_iters: u64,
    /// Recorded seed (block-order randomization belongs to the
    /// orchestrator; the subject echoes it for spec fidelity).
    #[serde(default = "default_seed")]
    pub seed: u64,
}

fn default_warmup() -> usize {
    ramshaper_bench::workload::DEFAULT_WARMUP_EVENTS
}

fn default_task_iters() -> u64 {
    200_000
}

fn default_seed() -> u64 {
    0xAB1D_0C0F_EEBE_EF00
}

impl SubjectSpec {
    /// Construct the real `WorkloadSpec` (the workload itself stays
    /// bench-owned; the mirror only crosses the process boundary).
    #[must_use]
    pub fn to_workload_spec(&self) -> WorkloadSpec {
        WorkloadSpec {
            events_per_run: self.events_per_run,
            warmup_events: self.warmup_events,
            task_iters: self.task_iters,
            seed: self.seed,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub spec_path: PathBuf,
    pub run_id: String,
}

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut spec_path: Option<PathBuf> = None;
    let mut run_id = "subject".to_string();
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
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let Some(spec_path) = spec_path else {
        return Err("missing --spec <SPEC.json>".to_string());
    };
    Ok(Parsed { spec_path, run_id })
}

/// One measured latency, milliseconds. Missing values never occur on this
/// path (`run_workload` records environmental gaps as completion status,
/// not fabricated events) — mapped honestly anyway.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(untagged)]
pub enum Latency {
    /// A measured task latency, milliseconds.
    Measured(f64),
    /// A missing value with its preserved reason (never silently dropped).
    Missing {
        /// Why the event is missing.
        reason: String,
    },
}

/// The emitted run record (one JSON object on stdout).
#[derive(Debug, serde::Serialize)]
pub struct SubjectRun {
    /// Exact schema tag ([`SUBJECT_RUN_SCHEMA`]).
    pub schema: &'static str,
    /// Tool version.
    pub version: &'static str,
    /// The run identifier (echoed).
    pub run_id: String,
    /// The spec as executed.
    pub spec: SubjectSpec,
    /// The measured series (warmup excluded by design), milliseconds.
    pub latencies_ms: Vec<Latency>,
    /// How the run ended.
    pub completion: serde_json::Value,
}

fn run(parsed: &Parsed) -> Result<SubjectRun, String> {
    let text = std::fs::read_to_string(&parsed.spec_path)
        .map_err(|error| format!("cannot read spec {}: {error}", parsed.spec_path.display()))?;
    let spec: SubjectSpec = serde_json::from_str(&text).map_err(|error| {
        format!(
            "{} is not a valid subject spec: {error}",
            parsed.spec_path.display()
        )
    })?;
    // The doc-08 minimum lives in the bench validator (one authority).
    spec.to_workload_spec()
        .validate()
        .map_err(|error| format!("invalid spec: {error}"))?;

    let treatment = || {};
    let stop = AtomicBool::new(false);
    let executed = run_workload(&spec.to_workload_spec(), &parsed.run_id, &treatment, &stop);

    let latencies_ms = executed
        .events
        .events
        .into_iter()
        .map(|value| match value {
            ramshaper_bench::analysis::EventValue::Measured(ms) => Latency::Measured(ms),
            ramshaper_bench::analysis::EventValue::Missing { reason } => {
                Latency::Missing { reason }
            }
        })
        .collect();
    let completion = match executed.completion {
        Completion::Complete => serde_json::json!({ "status": "complete" }),
        Completion::StoppedEarly { executed } => {
            serde_json::json!({ "status": "stopped_early", "executed": executed })
        }
    };
    Ok(SubjectRun {
        schema: SUBJECT_RUN_SCHEMA,
        version: VERSION,
        run_id: executed.events.run_id,
        spec,
        latencies_ms,
        completion,
    })
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
    match run(&parsed) {
        Ok(record) => {
            match serde_json::to_string_pretty(&record) {
                Ok(out) => println!("{out}"),
                Err(error) => {
                    eprintln!("error: failed to serialize the run record: {error}");
                    return ExitCode::from(1);
                }
            }
            let _ = std::io::stdout().flush();
            // The exit code mirrors the completion status (doc 08: early
            // stops are recorded separately, never hidden).
            match record.completion.get("status").and_then(|s| s.as_str()) {
                Some("complete") => ExitCode::SUCCESS,
                _ => ExitCode::from(1),
            }
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_mirror_matches_bench_defaults_and_minimums() {
        let spec = SubjectSpec {
            events_per_run: 100,
            warmup_events: 10,
            task_iters: 1,
            seed: 1,
        };
        let workload = spec.to_workload_spec();
        assert_eq!(workload.events_per_run, 100);
        assert_eq!(workload.warmup_events, 10);
        // The doc-08 minimum (>= 100 events) is enforced by the bench
        // validator — the mirror delegates to it, never re-implements it.
        assert!(workload.validate().is_ok());
        let small = SubjectSpec {
            events_per_run: 50,
            ..spec
        };
        assert!(small.to_workload_spec().validate().is_err());
    }

    #[test]
    fn spec_parse_is_closed_schema_with_defaults() {
        let spec: SubjectSpec =
            serde_json::from_str(r#"{"events_per_run": 250, "task_iters": 1000}"#)
                .expect("parses with defaults");
        assert_eq!(
            spec.warmup_events,
            ramshaper_bench::workload::DEFAULT_WARMUP_EVENTS
        );
        assert_eq!(spec.seed, 0xAB1D_0C0F_EEBE_EF00);
        assert!(
            serde_json::from_str::<SubjectSpec>(r#"{"events_per_run": 250, "bogus": true}"#)
                .is_err(),
            "unknown fields are refused"
        );
    }

    #[test]
    fn parse_args_rules() {
        assert!(parse_args(vec![]).is_err(), "missing --spec is an error");
        assert!(parse_args(vec!["--spec".to_string()]).is_err());
        assert!(parse_args(vec!["bogus".to_string()]).is_err());
        let parsed = parse_args(vec!["--spec".to_string(), "s.json".to_string()]).expect("parses");
        assert_eq!(parsed.spec_path, PathBuf::from("s.json"));
        assert_eq!(parsed.run_id, "subject");
        let named = parse_args(vec![
            "--spec".to_string(),
            "s.json".to_string(),
            "--run-id".to_string(),
            "block3-B".to_string(),
        ])
        .expect("parses");
        assert_eq!(named.run_id, "block3-B");
    }

    #[test]
    fn a_full_run_record_carries_the_series_and_completion() {
        let spec = SubjectSpec {
            events_per_run: 120,
            warmup_events: 2,
            task_iters: 1,
            seed: 7,
        };
        let treatment = || {};
        let stop = AtomicBool::new(false);
        let executed = run_workload(&spec.to_workload_spec(), "t1", &treatment, &stop);
        assert_eq!(executed.completion, Completion::Complete);
        assert_eq!(executed.events.events.len(), 120, "warmup excluded");
    }
}
