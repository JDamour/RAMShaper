//! Synthetic task-latency workload and the counterbalanced A/B driver
//! (RS-024 Stage B runner half; doc 08 §3).
//!
//! The primary endpoint's data source: a bounded, deterministic-schedule
//! CPU workload whose per-task latencies form the runs analyzed by
//! [`crate::analysis`]. Doc 08 §3 rules implemented here:
//!
//! - a **fixed warmup excluded by design** (its events are executed and
//!   then dropped — never part of the analyzed series);
//! - a **fixed event schedule** (`events_per_run`, default 100 — the
//!   descriptive-p95 minimum);
//! - **counterbalanced AB/BA block order** from a seeded deterministic
//!   PRNG (the same SplitMix64 family as the analysis bootstrap), so the
//!   order effect is controlled and reproducible;
//! - **stop control** (an atomic cancel flag): a stopped run records its
//!   completion status separately — the captured events stay analyzable;
//! - the **treatment is an injected closure** invoked before each task.
//!   The only production treatment today is `none` (observe-only): the
//!   policy action's injection lands here when the owner-gated consent
//!   flow enables it. A pilot with `none` therefore honestly shows ~0%
//!   delta — the machinery is exercised, no benefit is claimed.
//!
//! The workload is **CPU-only and bounded** — no memory pressure is ever
//! induced on a live workspace (doc 08 §3: synthetic *pressure* workloads
//! run only in a disposable environment; this harness measures task
//! scheduling latency, which is safe on any machine).

use crate::analysis::{Block, EventValue, RunEvents};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Default events per run (doc 08 §3: at least 100 valid repeated task
/// events for a descriptive p95).
pub const DEFAULT_EVENTS_PER_RUN: usize = 100;
/// Default warmup events, excluded from the analyzed series by design.
pub const DEFAULT_WARMUP_EVENTS: usize = 10;

/// The workload schedule and effort knobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadSpec {
    /// Measured tasks per run (analyzed series length).
    pub events_per_run: usize,
    /// Warmup tasks before the measured series (excluded by design).
    pub warmup_events: usize,
    /// Per-task bounded compute effort: internal loop iterations. Kept
    /// small (default ≈ a millisecond on a modern core) so a pilot is
    /// quick and the machine stays responsive.
    pub task_iters: u64,
    /// The recorded seed for block-order randomization.
    pub seed: u64,
}

impl Default for WorkloadSpec {
    fn default() -> Self {
        Self {
            events_per_run: DEFAULT_EVENTS_PER_RUN,
            warmup_events: DEFAULT_WARMUP_EVENTS,
            task_iters: 200_000,
            seed: 0xAB1D_0C0F_EEBE_EF00,
        }
    }
}

impl WorkloadSpec {
    /// Validate the doc 08 minimums.
    pub fn validate(&self) -> Result<(), String> {
        if self.events_per_run < 100 {
            return Err(format!(
                "events_per_run {} is below the descriptive-p95 minimum of 100 (doc 08 §3); \
                 fewer runs must be labelled exploratory by the analysis, not silently accepted",
                self.events_per_run
            ));
        }
        Ok(())
    }
}

/// How a run ended. Doc 08 §3: observe-only and treatment runs have the
/// same capture schedule; early stops are recorded separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Completion {
    /// The full event schedule executed.
    Complete,
    /// Stop control fired after this many of the scheduled tasks.
    StoppedEarly {
        /// Tasks (warmup + measured) completed before the stop.
        executed: usize,
    },
}

/// One executed run: the analyzed series plus its completion record.
#[derive(Debug, Clone)]
pub struct ExecutedRun {
    /// Run identifier.
    pub run_id: String,
    /// The measured event series (all `Measured`; the workload itself has
    /// no missing values — missing events come from the *environment*,
    /// which the runner records as completion status, not as fabricated
    /// gaps).
    pub events: RunEvents,
    /// How the run ended.
    pub completion: Completion,
}

/// Execute one run of the workload. `treatment` runs before each measured
/// task (the injection point for the owner-gated policy action). The
/// warmup tasks run first and are dropped by design. Honors `stop`.
pub fn run_workload(
    spec: &WorkloadSpec,
    run_id: &str,
    treatment: &dyn Fn(),
    stop: &AtomicBool,
) -> ExecutedRun {
    // Warmup: executed, excluded from the series by design.
    for _ in 0..spec.warmup_events {
        if stop.load(Ordering::SeqCst) {
            return ExecutedRun {
                run_id: run_id.to_string(),
                events: RunEvents {
                    run_id: run_id.to_string(),
                    events: Vec::new(),
                },
                completion: Completion::StoppedEarly { executed: 0 },
            };
        }
        task_chunk(spec.task_iters);
    }
    let mut events = Vec::with_capacity(spec.events_per_run);
    for index in 0..spec.events_per_run {
        if stop.load(Ordering::SeqCst) {
            return ExecutedRun {
                run_id: run_id.to_string(),
                events: RunEvents {
                    run_id: run_id.to_string(),
                    events,
                },
                completion: Completion::StoppedEarly {
                    executed: spec.warmup_events + index,
                },
            };
        }
        treatment();
        let started = Instant::now();
        task_chunk(spec.task_iters);
        events.push(EventValue::Measured(
            started.elapsed().as_secs_f64() * 1000.0,
        ));
    }
    ExecutedRun {
        run_id: run_id.to_string(),
        events: RunEvents {
            run_id: run_id.to_string(),
            events,
        },
        completion: Completion::Complete,
    }
}

/// The bounded per-task compute chunk: a fixed-iteration integer mix over
/// a small state. CPU-only, allocation-free, and side-effect-free — a
/// proxy "task" whose latency reflects scheduling and machine load.
fn task_chunk(iters: u64) {
    let mut state = iters ^ 0xDA7A_5EED;
    let mut i = 0;
    while i < iters {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state ^= state >> 33;
        i += 1;
    }
    // Defeat constant-folding: fold the state into an observable-but-
    // discarded side effect via a black-box barrier.
    std::hint::black_box(state);
}

/// One A/B block's raw runs, pre-analysis.
#[derive(Debug, Clone)]
pub struct ExecutedBlock {
    /// Block id (1-based).
    pub block_id: u32,
    /// The order actually executed ("AB" = baseline first, "BA" =
    /// treatment first) — recorded for the counterbalancing evidence.
    pub order: &'static str,
    pub baseline: ExecutedRun,
    pub treatment: ExecutedRun,
}

/// The A/B driver: counterbalanced AB/BA block order from the spec's
/// seed, identical event schedules per block, one shared stop control.
/// The `treatment` closure is invoked for every treatment-run task
/// (`|_|` no-op today).
pub fn run_ab_pilot(
    spec: &WorkloadSpec,
    blocks: usize,
    treatment: &dyn Fn(),
    stop: &AtomicBool,
) -> Vec<ExecutedBlock> {
    // Deterministic block-order randomization: a fresh SplitMix64-style
    // generator seeded from the spec (kept local so the workload and the
    // analysis randomize independently).
    let mut order_state = spec.seed;
    let mut next_order = move || {
        order_state = order_state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = order_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };

    let mut executed = Vec::with_capacity(blocks);
    for (index, block_id) in (1..=blocks).enumerate() {
        let block_id = block_id as u32;
        let _ = index;
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let treatment_first = next_order() & 1 == 1;
        let order = if treatment_first { "BA" } else { "AB" };
        let noop = || {};
        let (baseline, treatment_run) = if treatment_first {
            let t = run_workload(spec, &format!("b-{block_id}"), treatment, stop);
            let b = run_workload(spec, &format!("a-{block_id}"), &noop, stop);
            (b, t)
        } else {
            let b = run_workload(spec, &format!("a-{block_id}"), &noop, stop);
            let t = run_workload(spec, &format!("b-{block_id}"), treatment, stop);
            (b, t)
        };
        executed.push(ExecutedBlock {
            block_id,
            order,
            baseline,
            treatment: treatment_run,
        });
    }
    executed
}

/// Convert executed blocks into the analysis input (complete runs only —
/// a stopped run's partial series is retained in the `ExecutedBlock` for
/// the record, but excluded from pairing: incomplete capture duration is
/// a completion record, not a comparable sample, per doc 08 §3).
#[must_use]
pub fn into_analysis_blocks(executed: &[ExecutedBlock]) -> Vec<Block> {
    executed
        .iter()
        .filter(|block| {
            block.baseline.completion == Completion::Complete
                && block.treatment.completion == Completion::Complete
        })
        .map(|block| Block {
            block_id: block.block_id,
            baseline: block.baseline.events.clone(),
            treatment: block.treatment.events.clone(),
        })
        .collect()
}

/// Shared stop control constructor (the CLI wires Ctrl+C-style cancel
/// here later; tests flip it directly).
#[must_use]
pub fn stop_control() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny spec: 100 measured events is the doc minimum but the task
    /// effort is minimal so the pilot tests run fast.
    fn tiny_spec(seed: u64, events: usize) -> WorkloadSpec {
        WorkloadSpec {
            events_per_run: events,
            warmup_events: 3,
            task_iters: 1_000,
            seed,
        }
    }

    #[test]
    fn spec_validates_the_doc08_minimum() {
        assert!(tiny_spec(1, 100).validate().is_ok());
        assert!(tiny_spec(1, 200).validate().is_ok());
        assert!(tiny_spec(1, 99).validate().is_err());
    }

    #[test]
    fn run_produces_the_full_schedule_with_warmup_excluded() {
        let spec = tiny_spec(1, 100);
        let stop = stop_control();
        let run = run_workload(&spec, "r", &|| {}, &stop);
        assert_eq!(run.completion, Completion::Complete);
        assert_eq!(run.events.events.len(), 100, "warmup excluded by design");
        // Every event is a measured positive latency.
        assert!(run.events.events.iter().all(|event| matches!(
            event,
            EventValue::Measured(value) if *value > 0.0
        )));
    }

    #[test]
    fn stop_control_stops_early_and_records_completion() {
        let spec = tiny_spec(1, 100);
        let stop = stop_control();
        stop.store(true, Ordering::SeqCst);
        let run = run_workload(&spec, "r", &|| {}, &stop);
        assert_eq!(
            run.completion,
            Completion::StoppedEarly { executed: 0 },
            "a pre-fired stop executes nothing"
        );
        assert!(run.events.events.is_empty());

        // Mid-run stop: flip the flag from within the treatment (the
        // injection point doubles as the test hook).
        let stop2 = stop_control();
        let flip = Arc::clone(&stop2);
        let flips = std::cell::Cell::new(0u32);
        let treatment = || {
            flips.set(flips.get() + 1);
            if flips.get() == 5 {
                flip.store(true, Ordering::SeqCst);
            }
        };
        let run = run_workload(&spec, "r", &treatment, &stop2);
        match run.completion {
            Completion::StoppedEarly { executed } => {
                assert!(
                    executed < spec.warmup_events + spec.events_per_run,
                    "stopped runs must be early: {executed}"
                );
            }
            other => panic!("expected StoppedEarly, got {other:?}"),
        }
    }

    #[test]
    fn treatment_closure_runs_before_each_measured_task() {
        let spec = tiny_spec(1, 100);
        let stop = stop_control();
        let calls = std::cell::RefCell::new(0u32);
        let treatment = || {
            *calls.borrow_mut() += 1;
        };
        let run = run_workload(&spec, "r", &treatment, &stop);
        // Once per measured task — warmup gets no treatment (it is not
        // part of the experiment).
        assert_eq!(*calls.borrow(), 100);
        assert_eq!(run.events.events.len(), 100);
    }

    #[test]
    fn ab_pilot_counterbalances_order_and_pairs_by_block() {
        let spec = tiny_spec(7, 100);
        let stop = stop_control();
        let executed = run_ab_pilot(&spec, 6, &|| {}, &stop);
        assert_eq!(executed.len(), 6);
        // Orders recorded and both directions appear across 6 blocks
        // (deterministic seed → stable pattern; assert the counter-
        // balancing property, not a specific sequence).
        let orders: Vec<&str> = executed.iter().map(|b| b.order).collect();
        assert!(orders.contains(&"AB") && orders.contains(&"BA"));
        for (index, block) in executed.iter().enumerate() {
            assert_eq!(block.block_id, index as u32 + 1);
            assert_eq!(block.baseline.completion, Completion::Complete);
            assert_eq!(block.treatment.completion, Completion::Complete);
            assert_eq!(block.baseline.events.events.len(), 100);
        }
        // Conversion keeps only complete pairs, with ids intact.
        let blocks = into_analysis_blocks(&executed);
        assert_eq!(blocks.len(), 6);
        assert_eq!(blocks[0].block_id, 1);
        assert_eq!(blocks[5].block_id, 6);
    }

    #[test]
    fn incomplete_pairs_are_excluded_from_analysis_but_retained() {
        let spec = tiny_spec(1, 100);
        let stop = stop_control();
        let mut executed = run_ab_pilot(&spec, 2, &|| {}, &stop);
        // Corrupt block 2's baseline into a stopped run (simulating an
        // environmental stop between blocks).
        executed[1].baseline.completion = Completion::StoppedEarly { executed: 7 };
        let blocks = into_analysis_blocks(&executed);
        assert_eq!(blocks.len(), 1, "only complete pairs are analyzable");
        assert_eq!(blocks[0].block_id, 1);
        // The stopped run is still in the record for the report.
        assert!(matches!(
            executed[1].baseline.completion,
            Completion::StoppedEarly { .. }
        ));
    }
}
