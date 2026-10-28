//! `ramshaper-trial` — the Stage B trial driver (RS-024 Stage B; design
//! memo 2026-10-28, follow-up ticket 3).
//!
//! Orchestrates counterbalanced A/B runs of the `ramshaper-subject`
//! workload. The A (baseline) runs are observe-only; the B (treatment)
//! runs are wrapped in the shipped consent machinery over the wire:
//! `PreviewAction` → `ConfirmAction` before the subject's measured loop,
//! `RestoreAction` after it — the **agent remains the only OS-writing
//! process** (single-writer principle; the driver performs no OS
//! mutation itself). The driver cannot enable a policy session and
//! cannot grant consent: the session's typed `ENABLE` (and, after
//! ADR-045, the operator-declared subject) belong to the agent's console.
//! A refusal at any step is fail-fast: the trial stops and no blocks
//! file is written for a half-executed experiment.
//!
//! Output: `analyze-ab`-compatible blocks JSON (`ramshaper-bench::analysis::Block`)
//! — `{block_id, baseline: {run_id, events}, treatment: {run_id, events}}`,
//! untagged plain-number latencies in milliseconds — plus a human summary
//! on stdout. Run identifiers are `{prefix}b{block}-{A|B}`; the treatment
//! action id is `trial-{run_id}`, deterministic for replay.
//!
//! The happy path is exercised end-to-end by the tests with a
//! test-constructed gate (the agent's own live-test pattern); ADR-045 —
//! the operator-facing allowlist/classification flags — gates only the
//! shipped production path, not this driver.

use std::process::ExitCode;

use ramshaper_trial::{parse_args, TOOL_NAME};

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
    match ramshaper_trial::run_trial(
        &ramshaper_trial::PipeRpc {
            pipe_name: &parsed.pipe_name,
        },
        &parsed,
    ) {
        Ok(blocks) => {
            let text = match serde_json::to_string_pretty(&ramshaper_trial::blocks_to_wire(&blocks))
            {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("error: failed to serialize the blocks: {error}");
                    return ExitCode::from(1);
                }
            };
            if let Err(error) = std::fs::write(&parsed.out, text.as_bytes()) {
                eprintln!("error: cannot write {}: {error}", parsed.out.display());
                return ExitCode::from(1);
            }
            println!(
                "{} block(s) written to {} (seed {}); analyze with: ramshaper-bench analyze-ab --blocks {}",
                blocks.len(),
                parsed.out.display(),
                parsed.seed,
                parsed.out.display()
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("trial stopped; no blocks file was written for a half-executed experiment");
            ExitCode::from(1)
        }
    }
}
