//! `ramshaper-bench` — measurement harnesses (Stage A: observer overhead).
//!
//! `ramshaper-bench observer-overhead [--iterations N] [--json] [--config P]`
//! measures the cost of this build's read-only collection paths on the
//! current machine: per-sample global snapshot latency and per-scan process
//! inventory latency, plus the harness process's own memory. Output is a
//! machine-readable report classified with its AT-21 hardware cohort: the
//! cohort comes from the `[cohort] declared` config section (default `none`)
//! and is verified against the machine's measured total physical memory, so
//! a report can never carry a cohort the machine does not belong to. This
//! 64 GiB validation machine can therefore only produce non-cohort baseline
//! reports (`cohort.status = "baseline"`).

#![deny(unsafe_code)]

use std::process::ExitCode;
use std::time::Instant;

use ramshaper_core::clock::WallClock as _;
use serde::Serialize;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_ITERATIONS: usize = 200;

const HELP: &str = "\
ramshaper-bench — RAMShaper measurement harnesses (Stage A)

USAGE:
    ramshaper-bench observer-overhead [--iterations N] [--json] [--config P]

Measures per-collection latency of the read-only collector paths on this
machine (global system snapshot; process inventory scan) and reports the
harness process's own working set / private commit.

The report is classified with its AT-21 hardware cohort from the optional
TOML config's [cohort] declared value (4gib, 8gib, 16gib, or none; default
none). A declared cohort is verified against the machine's measured total
physical memory and a mismatch refuses the run (exit 2): observations from
a machine outside a cohort cannot be published as cohort data. With no
declaration the report is a non-cohort baseline.
";

fn main() -> ExitCode {
    let mut harness: Option<&str> = None;
    let mut json = false;
    let mut iterations: Option<usize> = None;
    let mut config_path: Option<std::path::PathBuf> = None;

    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--help" | "-h" => {
                print!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("ramshaper-bench {VERSION}");
                return ExitCode::SUCCESS;
            }
            "--json" => json = true,
            "--config" => match args.next() {
                Some(value) => config_path = Some(std::path::PathBuf::from(value)),
                None => {
                    eprintln!("error: --config requires a path");
                    return ExitCode::from(2);
                }
            },
            "--iterations" => match args
                .next()
                .and_then(|v| v.to_string_lossy().parse::<usize>().ok())
            {
                Some(n) if (1..=100_000).contains(&n) => iterations = Some(n),
                _ => {
                    eprintln!("error: --iterations requires a number in 1..=100000");
                    return ExitCode::from(2);
                }
            },
            "observer-overhead" if harness.is_none() => harness = Some("observer-overhead"),
            other => {
                eprintln!("error: unexpected argument '{other}'");
                eprintln!(
                    "usage: ramshaper-bench observer-overhead [--iterations N] [--json] [--config P]"
                );
                return ExitCode::from(2);
            }
        }
    }

    match harness {
        Some("observer-overhead") => run_observer_overhead(
            iterations.unwrap_or(DEFAULT_ITERATIONS),
            json,
            config_path.as_ref(),
        ),
        _ => {
            eprintln!("error: missing harness name");
            eprintln!(
                "usage: ramshaper-bench observer-overhead [--iterations N] [--json] [--config P]"
            );
            ExitCode::from(2)
        }
    }
}

/// Simple percentile of an unsorted sample list (nearest-rank method on the
/// sorted copy). Public for unit tests.
#[must_use]
pub fn percentile(mut samples: Vec<f64>, p: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    let clamped = p.clamp(0.0, 100.0);
    let rank = ((clamped / 100.0) * (samples.len() as f64 - 1.0)).round() as usize;
    samples[rank.min(samples.len() - 1)]
}

#[derive(Serialize)]
struct LatencySummary {
    samples: usize,
    min_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

/// AT-21 cohort classification of this report (RS-025). `status` is
/// "baseline" for a `none` declaration (no cohort claim) and "verified"
/// when the declared cohort matched the measured machine.
#[derive(Serialize)]
struct CohortReport {
    declared: &'static str,
    measured: &'static str,
    total_physical_bytes: Option<u64>,
    status: &'static str,
}

/// Verify the declared cohort against the machine's measured total physical
/// memory and build the report block. Platform-independent; unit-tested.
fn classify_observation_cohort(
    declared: ramshaper_core::HardwareCohort,
    measured_total_bytes: Option<u64>,
) -> Result<CohortReport, ramshaper_core::CohortError> {
    let classification = ramshaper_core::cohort::verify_declared(declared, measured_total_bytes)?;
    let measured = measured_total_bytes.map(ramshaper_core::HardwareCohort::from_total_bytes);
    Ok(CohortReport {
        declared: declared.as_str(),
        measured: measured.map_or("unknown", ramshaper_core::HardwareCohort::as_str),
        total_physical_bytes: measured_total_bytes,
        status: if classification == ramshaper_core::HardwareCohort::NonCohort {
            "baseline"
        } else {
            "verified"
        },
    })
}

fn summarize(samples: Vec<f64>) -> LatencySummary {
    let min = samples.iter().copied().fold(f64::INFINITY, f64::min);
    LatencySummary {
        samples: samples.len(),
        min_ms: min,
        p50_ms: percentile(samples.clone(), 50.0),
        p95_ms: percentile(samples.clone(), 95.0),
        max_ms: samples.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    }
}

#[derive(Serialize)]
struct OverheadReport {
    schema: &'static str,
    measured_at_utc: String,
    tool: String,
    iterations: usize,
    system_snapshot: LatencySummary,
    process_scan: LatencySummary,
    process_count_last: usize,
    bench_self_working_set_bytes: Option<u64>,
    bench_self_private_commit_bytes: Option<u64>,
    os: ramshaper_core::OsInfo,
    cohort: CohortReport,
    note: String,
}

/// Loaded configuration (defaults when no file given). A specified file that
/// fails to parse is a configuration error (exit 2), per contract 14 §7.
fn load_config(path: Option<&std::path::PathBuf>) -> Result<ramshaper_core::CoreConfig, String> {
    match path {
        None => Ok(ramshaper_core::CoreConfig::default()),
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read config {}: {e}", path.display()))?;
            ramshaper_core::CoreConfig::parse(&text)
                .map_err(|e| format!("invalid config {}: {e}", path.display()))
        }
    }
}

#[cfg(windows)]
fn run_observer_overhead(
    iterations: usize,
    json: bool,
    config_path: Option<&std::path::PathBuf>,
) -> ExitCode {
    use ramshaper_core::units::pages_to_bytes;
    use ramshaper_platform::{ReadOnlyProcessMonitor as _, ReadOnlySystemMonitor as _};
    use ramshaper_windows::WindowsSystemMonitor;

    let config = match load_config(config_path) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };

    let monitor = WindowsSystemMonitor::new();

    // Warm-up pass (first calls may load DLLs / allocate caches).
    let _ = monitor.collect_system();
    let _ = monitor.collect_processes();

    // One reading of the machine's total physical memory for the AT-21
    // cohort classification. Absent or unconvertible -> None (unverifiable).
    let measured_total_bytes = monitor
        .collect_system()
        .memory
        .ok()
        .and_then(|raw| pages_to_bytes(raw.physical_total_pages, raw.page_size_bytes).ok());

    let cohort = match classify_observation_cohort(config.cohort.declared, measured_total_bytes) {
        Ok(cohort) => cohort,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let note = if cohort.status == "verified" {
        format!(
            "machine-local observer-overhead measurement on declared cohort '{}' (verified \
             against measured total physical memory)",
            cohort.declared
        )
    } else {
        "machine-local observer-overhead measurement; non-cohort baseline, not a cohort claim"
            .to_string()
    };

    eprintln!("measuring {iterations} system-snapshot iterations...");
    let mut system_latencies = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let _ = monitor.collect_system();
        system_latencies.push(started.elapsed().as_secs_f64() * 1000.0);
    }

    eprintln!("measuring {iterations} process-scan iterations...");
    let mut scan_latencies = Vec::with_capacity(iterations);
    let mut last_count = 0usize;
    for _ in 0..iterations {
        let started = Instant::now();
        let scan = monitor.collect_processes();
        scan_latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        last_count = scan.processes.len();
    }

    let (working_set, private_commit) =
        ramshaper_windows::processes::current_process_memory_bytes()
            .map_or((None, None), |(ws, priv_)| (Some(ws), Some(priv_)));

    let report = OverheadReport {
        schema: "ramshaper.observer_overhead.v1",
        measured_at_utc: ramshaper_core::rfc3339::format_utc_from_unix_ms(
            ramshaper_core::SystemClock.unix_epoch_ms(),
        ),
        tool: format!("ramshaper-bench {VERSION}"),
        iterations,
        system_snapshot: summarize(system_latencies),
        process_scan: summarize(scan_latencies),
        process_count_last: last_count,
        bench_self_working_set_bytes: working_set,
        bench_self_private_commit_bytes: private_commit,
        os: monitor.os_info(),
        cohort,
        note,
    };

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(out) => {
                println!("{out}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: serialization failed: {e}");
                ExitCode::from(1)
            }
        }
    } else {
        println!("observer overhead ({} iterations)", report.iterations);
        println!(
            "  system snapshot : min {:.3} ms, p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms",
            report.system_snapshot.min_ms,
            report.system_snapshot.p50_ms,
            report.system_snapshot.p95_ms,
            report.system_snapshot.max_ms
        );
        println!(
            "  process scan    : min {:.3} ms, p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms ({} processes)",
            report.process_scan.min_ms,
            report.process_scan.p50_ms,
            report.process_scan.p95_ms,
            report.process_scan.max_ms,
            report.process_count_last
        );
        println!(
            "  bench self      : working set {} bytes, private commit {} bytes",
            report.bench_self_working_set_bytes.unwrap_or(0),
            report.bench_self_private_commit_bytes.unwrap_or(0)
        );
        println!(
            "  cohort          : {} ({}; measured {}, total physical {} bytes)",
            report.cohort.declared,
            report.cohort.status,
            report.cohort.measured,
            report
                .cohort
                .total_physical_bytes
                .map_or_else(|| "unknown".to_string(), |b| b.to_string())
        );
        println!("  note            : {}", report.note);
        ExitCode::SUCCESS
    }
}

#[cfg(not(windows))]
fn run_observer_overhead(
    _iterations: usize,
    _json: bool,
    _config_path: Option<&std::path::PathBuf>,
) -> ExitCode {
    eprintln!("error: observer-overhead requires the Windows read-only adapter");
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramshaper_core::HardwareCohort;

    #[test]
    fn percentile_matches_known_values() {
        let samples: Vec<f64> = (1..=100).map(f64::from).collect();
        // Nearest-rank on 100 samples: round(0.50*99)=50 -> 51.0; round(0.95*99)=94 -> 95.0.
        assert_eq!(percentile(samples.clone(), 50.0), 51.0);
        assert_eq!(percentile(samples.clone(), 95.0), 95.0);
        assert_eq!(percentile(samples.clone(), 0.0), 1.0);
        assert_eq!(percentile(samples.clone(), 100.0), 100.0);
        assert_eq!(percentile(Vec::new(), 50.0), 0.0);
    }

    #[test]
    fn percentile_handles_unsorted_input() {
        let samples = vec![10.0, 1.0, 5.0, 3.0, 7.0];
        assert_eq!(percentile(samples, 50.0), 5.0);
    }

    #[test]
    fn baseline_declaration_classifies_the_observation() {
        // No reading at all still classifies as a baseline.
        let report =
            classify_observation_cohort(HardwareCohort::NonCohort, None).expect("baseline");
        assert_eq!(report.declared, "none");
        assert_eq!(report.measured, "unknown");
        assert_eq!(report.total_physical_bytes, None);
        assert_eq!(report.status, "baseline");

        // A measured reading is attached even for a baseline, so a reviewer
        // can audit what the machine actually was.
        let report = classify_observation_cohort(HardwareCohort::NonCohort, Some(68_448_034_816))
            .expect("baseline");
        assert_eq!(report.measured, "none");
        assert_eq!(report.total_physical_bytes, Some(68_448_034_816));
        assert_eq!(report.status, "baseline");

        // A declared cohort on a matching machine is "verified".
        let gib = 1024 * 1024 * 1024;
        let report = classify_observation_cohort(HardwareCohort::Cohort8GiB, Some(8 * gib))
            .expect("verified");
        assert_eq!(report.declared, "8gib");
        assert_eq!(report.measured, "8gib");
        assert_eq!(report.status, "verified");
    }

    /// AT-21: this 64 GiB machine declaring any cohort must refuse the run
    /// instead of stamping a cohort claim onto a baseline machine.
    #[test]
    fn declared_cohort_on_this_64gib_machine_is_refused() {
        for declared in [
            HardwareCohort::Cohort4GiB,
            HardwareCohort::Cohort8GiB,
            HardwareCohort::Cohort16GiB,
        ] {
            assert!(classify_observation_cohort(declared, Some(68_448_034_816)).is_err());
        }
        assert!(classify_observation_cohort(HardwareCohort::Cohort8GiB, None).is_err());
    }
}
