//! `ramshaper-subject` library: the Stage B trial subject's shared types
//! and run logic. The binary (main.rs) owns the CLI; the trial driver
//! (`ramshaper-trial`) reuses the spec type and record schema from here
//! so the two processes speak exactly the same JSON.
//!
//! Safety shape (module docs in main.rs carry the full statement): the
//! subject is **inert** — always observe-only, no IPC, no OS mutation;
//! any treatment is applied externally by the consent-gated agent.

#![deny(unsafe_code)]

use ramshaper_bench::workload::{run_workload, Completion, WorkloadSpec};
use serde::Deserialize;

/// The run-record schema tag.
pub const SUBJECT_RUN_SCHEMA: &str = "ramshaper.subject-run.v1";
/// The line the binary prints (on stdout) once it is spawned and waiting
/// for the driver's go signal in `--hold-until-go` mode.
pub const SUBJECT_READY_MARKER: &str = "SUBJECT_READY";
/// The stdin line that releases a held subject.
pub const SUBJECT_GO_MARKER: &str = "GO";

/// The spec mirror of `ramshaper_bench::workload::WorkloadSpec` (identical
/// field names and minimums; the bench types carry no serde derives and
/// the bench crate was frozen during the AT-06 soak when this was
/// written).
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

    /// Validate against the doc-08 minimums (delegated to the bench
    /// validator — one authority).
    ///
    /// # Errors
    /// The bench validator's reason verbatim.
    pub fn validate(&self) -> Result<(), String> {
        self.to_workload_spec().validate()
    }
}

/// Read and parse a spec file (closed schema).
///
/// # Errors
/// I/O or parse failures carry the path and reason.
pub fn load_spec(path: &std::path::Path) -> Result<SubjectSpec, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read spec {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("{} is not a valid subject spec: {error}", path.display()))
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
    /// How the run ended (`{"status":"complete"}` or
    /// `{"status":"stopped_early","executed":N}`).
    pub completion: serde_json::Value,
}

/// Execute one run (observe-only by definition — the treatment closure is
/// a no-op; any real treatment is applied externally by the agent).
#[must_use]
pub fn execute(spec: &SubjectSpec, run_id: &str) -> SubjectRun {
    let treatment = || {};
    let stop = std::sync::atomic::AtomicBool::new(false);
    let executed = run_workload(&spec.to_workload_spec(), run_id, &treatment, &stop);
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
    SubjectRun {
        schema: SUBJECT_RUN_SCHEMA,
        version: env!("CARGO_PKG_VERSION"),
        run_id: executed.events.run_id,
        spec: spec.clone(),
        latencies_ms,
        completion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_mirror_matches_bench_minimums() {
        let spec = SubjectSpec {
            events_per_run: 100,
            warmup_events: 10,
            task_iters: 1,
            seed: 1,
        };
        assert!(spec.validate().is_ok());
        let small = SubjectSpec {
            events_per_run: 50,
            ..spec
        };
        assert!(small.validate().is_err());
    }

    #[test]
    fn spec_parse_is_closed_schema_with_defaults() {
        let spec: SubjectSpec =
            serde_json::from_str(r#"{"events_per_run": 250, "task_iters": 1000}"#)
                .expect("parses with defaults");
        assert_eq!(spec.warmup_events, default_warmup());
        assert!(
            serde_json::from_str::<SubjectSpec>(r#"{"events_per_run": 250, "bogus": true}"#)
                .is_err()
        );
    }

    #[test]
    fn a_full_run_record_carries_the_series_and_completion() {
        let spec = SubjectSpec {
            events_per_run: 120,
            warmup_events: 2,
            task_iters: 1,
            seed: 7,
        };
        let record = execute(&spec, "t1");
        assert_eq!(record.completion["status"], "complete");
        assert_eq!(record.latencies_ms.len(), 120, "warmup excluded");
        assert_eq!(record.run_id, "t1");
    }
}
