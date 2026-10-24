//! `ramshaper-cli` — read-only diagnostics (G0: RS-001..RS-006; M1: RS-007..009).
//!
//! Commands: `info [--json]`, `low-memory-watch [--json] [--interval-ms]`,
//! `processes [--json] [--interval-ms]`, `--version`, `--help`.
//! Contracts (doc 14 §8): stdout carries data only, diagnostics go to
//! stderr; exit codes 0 success (incl. healthy Ctrl+C cancel), 1 runtime
//! failure / no usable global sample, 2 usage or configuration error,
//! 3 partial snapshot. No mutating command exists.

#![deny(unsafe_code)]

use std::ffi::OsString;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use ramshaper_core::clock::{MonotonicClock as _, WallClock as _};
use ramshaper_core::render::{render_info_text, render_watch_line};
use ramshaper_core::snapshot::{
    build_snapshot, classify_snapshot, SnapshotHealth, SnapshotInputs, SystemSnapshot, ToolInfo,
};
use ramshaper_core::watch::{run_watch, WatchConfig};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TOOL_NAME: &str = "ramshaper-cli";
/// Default interval between the two process scans used for rates.
const DEFAULT_PROCESS_INTERVAL_MS: u64 = 500;
/// Minimum graceful auto-stop duration for the watch (1 s).
const MIN_STOP_AFTER_MS: u64 = 1_000;
/// Maximum graceful auto-stop duration (30 days).
const MAX_STOP_AFTER_MS: u64 = 30 * 24 * 60 * 60 * 1000;

const HELP: &str = "\
ramshaper-cli — read-only Windows memory diagnostics

USAGE:
    ramshaper-cli <COMMAND> [OPTIONS]

COMMANDS:
    info                 Print one system memory snapshot (with pressure state)
    low-memory-watch     Sample system memory + low-memory condition until Ctrl+C
    processes            List processes with memory, CPU and I/O (two-sample rates)
    config               Validate, export or atomically import TOML configuration

CONFIG USAGE:
    ramshaper-cli config export [--config <PATH>] <DEST>
    ramshaper-cli config import <SOURCE> --into <DEST>

`config export` writes validated TOML to DEST (defaults when --config is
absent). `config import` validates SOURCE against the closed schema
(field-level errors on any violation) and atomically replaces DEST; the
parsed mode must be observe_only, and profiles/exclusions are stored as
inert labels that never enable actions. Both actions touch only the named
file - no OS settings, no other files (RS-012).

OPTIONS:
    --json               JSON output (info/processes: one versioned object;
                         watch: newline-delimited objects)
    --interval-ms <MS>   watch: poll interval (default 1000)
                         processes: rate window between two scans (default 500)
                         range 100..=3600000
    --stop-after-ms <MS> watch: stop gracefully after MS milliseconds and
                         print the history summary (default: run until
                         Ctrl+C); range 1000..=2592000000
    --into <DEST>        config import: destination file, replaced atomically
    --config <PATH>      Load a TOML configuration file (schema_version 1,
                         observe-only; invalid files exit 2)
    --version, -V        Print version
    --help, -h           Print this help

EXIT CODES:
    0  success (including a healthy Ctrl+C-cancelled watch)
    1  runtime failure / no usable global sample
    2  usage or configuration error
    3  partial snapshot (optional metric/truncation reported)

Per-process access_denied/exited statuses are expected on protected processes
and do not fail the processes command. This tool is strictly read-only: it
queries OS counters and conditions and never changes process settings,
priorities, working sets or jobs. No network connection, telemetry or update
check is performed.
";

/// Parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliArgs {
    command: Command,
    json: bool,
    /// Raw `--interval-ms` flag; `None` when absent. Consumers apply the
    /// precedence flag > config file > built-in default (see
    /// [`watch_interval`]; `processes` resolves its own default).
    interval_ms: Option<u64>,
    stop_after_ms: Option<u64>,
    config_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigAction {
    Export,
    Import,
}

/// Validated shape of the `config` subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSpec {
    action: ConfigAction,
    /// `export --config <SRC>` source, or the `import <SRC>` positional.
    source: Option<PathBuf>,
    /// Destination file (positional for export, `--into` for import).
    dest: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Info,
    LowMemoryWatch,
    Processes,
    Config(ConfigSpec),
}

/// Parse outcome (unit-testable, no I/O).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseOutcome {
    Run(CliArgs),
    Version,
    Help,
    UsageError(String),
}

pub fn parse_args<I: IntoIterator<Item = OsString>>(args: I) -> ParseOutcome {
    let mut args = args.into_iter().peekable();
    let mut command: Option<Command> = None;
    let mut json = false;
    let mut interval_ms: Option<u64> = None;
    let mut stop_after_ms: Option<u64> = None;
    let mut config_path: Option<PathBuf> = None;
    let mut into: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "--version" | "-V" => return ParseOutcome::Version,
            "--help" | "-h" => return ParseOutcome::Help,
            "--json" => json = true,
            "--config" => match args.next() {
                Some(value) => config_path = Some(PathBuf::from(value)),
                None => return ParseOutcome::UsageError("--config requires a path".into()),
            },
            "--interval-ms" => match args.next() {
                Some(value) => match value.to_string_lossy().parse::<u64>() {
                    Ok(ms) => {
                        if WatchConfig::new(ms).is_err() {
                            return ParseOutcome::UsageError(format!(
                                "--interval-ms {ms} out of range (100..=3600000)"
                            ));
                        }
                        interval_ms = Some(ms);
                    }
                    Err(_) => {
                        return ParseOutcome::UsageError(format!(
                            "--interval-ms expects a number, got {}",
                            value.to_string_lossy()
                        ));
                    }
                },
                None => return ParseOutcome::UsageError("--interval-ms requires a value".into()),
            },
            "--stop-after-ms" => match args.next() {
                Some(value) => match value.to_string_lossy().parse::<u64>() {
                    Ok(ms) if (MIN_STOP_AFTER_MS..=MAX_STOP_AFTER_MS).contains(&ms) => {
                        stop_after_ms = Some(ms);
                    }
                    Ok(ms) => {
                        return ParseOutcome::UsageError(format!(
                            "--stop-after-ms {ms} out of range ({MIN_STOP_AFTER_MS}..={MAX_STOP_AFTER_MS})"
                        ));
                    }
                    Err(_) => {
                        return ParseOutcome::UsageError(format!(
                            "--stop-after-ms expects a number, got {}",
                            value.to_string_lossy()
                        ));
                    }
                },
                None => return ParseOutcome::UsageError("--stop-after-ms requires a value".into()),
            },
            "--into" => match args.next() {
                Some(value) => into = Some(PathBuf::from(value)),
                None => return ParseOutcome::UsageError("--into requires a path".into()),
            },
            "config" if command.is_none() => {
                let action = match args.next() {
                    Some(action) => match action.to_string_lossy().as_ref() {
                        "export" => ConfigAction::Export,
                        "import" => ConfigAction::Import,
                        other => {
                            return ParseOutcome::UsageError(format!(
                                "unknown config action '{other}' (expected export or import)"
                            ));
                        }
                    },
                    None => {
                        return ParseOutcome::UsageError(
                            "config requires an action (export or import)".into(),
                        );
                    }
                };
                command = Some(Command::Config(ConfigSpec {
                    action,
                    source: None,
                    dest: PathBuf::new(),
                }));
            }
            "info" if command.is_none() => command = Some(Command::Info),
            "low-memory-watch" if command.is_none() => command = Some(Command::LowMemoryWatch),
            "processes" if command.is_none() => command = Some(Command::Processes),
            other => {
                // The config subcommand takes positional paths: export has a
                // single destination; import takes the source file (the
                // destination comes via --into).
                let is_flag = other.starts_with('-');
                if let Some(Command::Config(spec)) = &mut command {
                    if !is_flag {
                        if spec.action == ConfigAction::Import && spec.source.is_none() {
                            spec.source = Some(PathBuf::from(arg));
                        } else if spec.dest.as_os_str().is_empty() {
                            spec.dest = PathBuf::from(arg);
                        } else {
                            return ParseOutcome::UsageError(format!(
                                "unexpected argument '{other}'"
                            ));
                        }
                        continue;
                    }
                }
                return ParseOutcome::UsageError(if command.is_none() {
                    format!("unknown command '{other}'")
                } else {
                    format!("unexpected argument '{other}'")
                });
            }
        }
    }

    let Some(command) = command else {
        return ParseOutcome::UsageError("missing command".into());
    };
    if command == Command::Info && interval_ms.is_some() {
        return ParseOutcome::UsageError("--interval-ms is not valid for info".into());
    }
    if stop_after_ms.is_some() && command != Command::LowMemoryWatch {
        return ParseOutcome::UsageError(
            "--stop-after-ms is only valid for low-memory-watch".into(),
        );
    }
    if let Command::Config(spec) = &command {
        match spec.action {
            ConfigAction::Export => {
                if spec.dest.as_os_str().is_empty() {
                    return ParseOutcome::UsageError(
                        "config export requires a destination path".into(),
                    );
                }
                if into.is_some() {
                    return ParseOutcome::UsageError(
                        "--into is only valid for config import".into(),
                    );
                }
            }
            ConfigAction::Import => {
                if spec.source.is_none() {
                    return ParseOutcome::UsageError("config import requires a source file".into());
                }
                if config_path.is_some() {
                    return ParseOutcome::UsageError(
                        "--config is not valid for config import; give the source as the \
                         positional argument"
                            .into(),
                    );
                }
                if into.is_none() {
                    return ParseOutcome::UsageError("config import requires --into <DEST>".into());
                }
            }
        }
    }
    // Import takes its destination from --into.
    let command = match (command, into) {
        (Command::Config(mut spec), Some(dest)) => {
            spec.dest = dest;
            Command::Config(spec)
        }
        (command, _) => command,
    };
    ParseOutcome::Run(CliArgs {
        command,
        json,
        interval_ms,
        stop_after_ms,
        config_path,
    })
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match parse_args(args) {
        ParseOutcome::Version => {
            println!("{TOOL_NAME} {VERSION}");
            ExitCode::SUCCESS
        }
        ParseOutcome::Help => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        ParseOutcome::UsageError(message) => {
            eprintln!("error: {message}");
            eprintln!("usage: {TOOL_NAME} <info|low-memory-watch|processes|config> [--json] [--interval-ms <ms>] [--stop-after-ms <ms>] [--config <path>] [--into <path>]");
            eprintln!("run '{TOOL_NAME} --help' for details");
            ExitCode::from(2)
        }
        ParseOutcome::Run(args) => run(args),
    }
}

/// Loaded configuration (defaults when no file given). A specified file that
/// fails to parse is a configuration error (exit 2), per contract 14 §7.
fn load_config(path: Option<&PathBuf>) -> Result<ramshaper_core::CoreConfig, String> {
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

/// Watch cadence precedence: explicit `--interval-ms` flag, then the config
/// file's `watch.interval_ms`, then the built-in default.
///
/// Regression guard: the M1 increment passed `config.watch.interval_ms` to
/// the watch and dropped the parsed flag, so `--interval-ms` was silently
/// ignored for `low-memory-watch` (the G0 cadence tests predate it and the
/// M1 cadence demo used `--config`, which is why it went undetected).
#[must_use]
fn watch_interval(args: &CliArgs, config: &ramshaper_core::CoreConfig) -> u64 {
    args.interval_ms.unwrap_or(config.watch.interval_ms)
}

/// Validate, export or atomically import TOML configuration (RS-012). Both
/// actions are inert file operations: they write only the named TOML file
/// (temp + `MoveFileExW` replace via `ramshaper_windows::fs::atomic_write`)
/// and never touch OS settings, the agent or any other file. Validation
/// failures are field-level configuration errors (exit 2); I/O failures are
/// runtime errors (exit 1).
#[cfg(windows)]
fn run_config(spec: &ConfigSpec, config: &ramshaper_core::CoreConfig) -> ExitCode {
    use ramshaper_windows::fs::atomic_write;

    match spec.action {
        ConfigAction::Export => {
            let text = match config.to_toml() {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::from(1);
                }
            };
            match atomic_write(&spec.dest, text.as_bytes()) {
                Ok(()) => {
                    println!(
                        "exported validated configuration to {}",
                        spec.dest.display()
                    );
                    ExitCode::SUCCESS
                }
                Err((code, function)) => {
                    eprintln!(
                        "error: cannot write {}: OS error {code} from {function}",
                        spec.dest.display()
                    );
                    ExitCode::from(1)
                }
            }
        }
        ConfigAction::Import => {
            let source = spec.source.as_deref().expect("validated at parse time");
            let text = match std::fs::read_to_string(source) {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("error: cannot read {}: {error}", source.display());
                    return ExitCode::from(1);
                }
            };
            // Closed-schema validation with field-level errors; the parsed
            // mode is observe_only by construction (anything else is
            // rejected before this point).
            let parsed = match ramshaper_core::CoreConfig::parse(&text) {
                Ok(parsed) => parsed,
                Err(error) => {
                    eprintln!(
                        "error: {} is not a valid configuration: {error}",
                        source.display()
                    );
                    return ExitCode::from(2);
                }
            };
            let canonical = match parsed.to_toml() {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::from(1);
                }
            };
            match atomic_write(&spec.dest, canonical.as_bytes()) {
                Ok(()) => {
                    println!(
                        "imported validated configuration into {} (mode: {}, profile: {})",
                        spec.dest.display(),
                        parsed.mode,
                        parsed.profiles.active
                    );
                    ExitCode::SUCCESS
                }
                Err((code, function)) => {
                    eprintln!(
                        "error: cannot write {}: OS error {code} from {function}",
                        spec.dest.display()
                    );
                    ExitCode::from(1)
                }
            }
        }
    }
}

fn run(args: CliArgs) -> ExitCode {
    let config = match load_config(args.config_path.as_ref()) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };
    match args.command {
        Command::Config(spec) => run_config(&spec, &config),
        Command::Info => run_info(args.json),
        Command::LowMemoryWatch => run_watch_command(
            args.json,
            watch_interval(&args, &config),
            args.stop_after_ms,
        ),
        Command::Processes => run_processes(
            args.json,
            args.interval_ms.unwrap_or(DEFAULT_PROCESS_INTERVAL_MS),
        ),
    }
}

#[cfg(windows)]
fn run_info(json: bool) -> ExitCode {
    use ramshaper_windows::WindowsSystemMonitor;

    let monitor = WindowsSystemMonitor::new();
    let snapshot = collect_one(&monitor, 1);
    let pressure =
        ramshaper_core::pressure::PressureClassifier::new().observe(pressure_input_for(&snapshot));
    if json {
        match serde_json::to_string_pretty(&with_pressure(&snapshot, &pressure)) {
            Ok(out) => println!("{out}"),
            Err(e) => {
                eprintln!("error: failed to serialize snapshot: {e}");
                return ExitCode::from(1);
            }
        }
    } else {
        print!("{}", render_info_text(&snapshot));
        println!();
        println!("Memory pressure (rule {})", pressure.rule_version);
        println!("  state           : {}", pressure.level.as_str());
        println!("  evidence        : {}", evidence_text(&pressure));
        println!(
            "  note            : a single observation cannot establish Normal/Elevated; watch to accumulate evidence"
        );
        println!("  meaning         : {}", pressure.level.display_text());
    }
    health_to_exit(classify_snapshot(&snapshot))
}

/// Snapshot + pressure observation in one JSON object.
#[cfg(windows)]
fn with_pressure<'a>(
    snapshot: &'a SystemSnapshot,
    pressure: &'a ramshaper_core::pressure::PressureObservation,
) -> impl serde::Serialize + 'a {
    #[derive(serde::Serialize)]
    struct Wrapped<'a> {
        #[serde(flatten)]
        snapshot: &'a SystemSnapshot,
        pressure: &'a ramshaper_core::pressure::PressureObservation,
    }
    Wrapped { snapshot, pressure }
}

/// Build the classifier input from a snapshot's metric validity.
#[cfg(windows)]
fn pressure_input_for(
    snapshot: &SystemSnapshot,
) -> Option<ramshaper_core::pressure::PressureInput> {
    let m = &snapshot.metrics;
    let missing = ramshaper_core::pressure::missing_inputs(
        m.physical_total.status.is_ok(),
        m.physical_available.status.is_ok(),
        m.commit_used.status.is_ok(),
        m.commit_limit.status.is_ok(),
        m.low_memory.status.is_ok(),
    );
    if missing.is_empty() {
        Some(ramshaper_core::pressure::PressureInput {
            low_memory_signaled: matches!(
                m.low_memory.state,
                Some(ramshaper_core::LowMemorySignal::Signaled)
            ),
            monotonic_ms: snapshot.collected_monotonic_ms,
        })
    } else {
        None
    }
}

#[cfg(windows)]
fn evidence_text(pressure: &ramshaper_core::pressure::PressureObservation) -> String {
    match pressure.evidence {
        ramshaper_core::pressure::EvidenceQuality::Complete => "complete".to_string(),
        ramshaper_core::pressure::EvidenceQuality::Partial => {
            format!("partial (missing: {})", pressure.missing_inputs.join(", "))
        }
        ramshaper_core::pressure::EvidenceQuality::Unavailable => {
            "unavailable (no valid inputs)".to_string()
        }
    }
}

#[cfg(windows)]
fn run_watch_command(json: bool, interval_ms: u64, stop_after_ms: Option<u64>) -> ExitCode {
    use ramshaper_core::history::BoundedHistory;
    use ramshaper_core::pressure::PressureClassifier;
    use ramshaper_windows::console::ConsoleCancel;
    use ramshaper_windows::WindowsSystemMonitor;

    let Ok(config) = WatchConfig::new(interval_ms) else {
        eprintln!("error: invalid interval {interval_ms} ms");
        return ExitCode::from(2);
    };
    let Ok(cancel) = ConsoleCancel::install() else {
        eprintln!(
            "error: failed to install Ctrl+C handler; refusing to run an uncancellable watch"
        );
        return ExitCode::from(1);
    };
    let cancel = std::sync::Arc::new(cancel);
    if let Some(stop_after_ms) = stop_after_ms {
        eprintln!("auto-stop after {stop_after_ms} ms");
        let timer_cancel = std::sync::Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(stop_after_ms));
            timer_cancel.trigger();
        });
    }
    eprintln!(
        "watching low-memory condition every {} ms; press Ctrl+C to stop",
        config.interval_ms()
    );

    let monitor = WindowsSystemMonitor::new();
    let clock = ramshaper_core::SystemClock;
    let mut sequence: u64 = 0;
    let mut saw_usable_sample = false;
    let mut classifier = PressureClassifier::new();
    // Bounded history with the configured retention window (RS-008).
    let mut history: BoundedHistory<()> = BoundedHistory::new();
    let mut stdout = std::io::stdout().lock();

    let summary = run_watch(&config, &clock, &*cancel, |_| {
        sequence += 1;
        let snapshot = collect_one(&monitor, sequence);
        if classify_snapshot(&snapshot) != SnapshotHealth::NoUsableGlobalSample {
            saw_usable_sample = true;
        }
        let pressure = classifier.observe(pressure_input_for(&snapshot));
        // Size estimate: the serialized snapshot length (payload budget).
        let size = serde_json::to_string(&snapshot).map_or(0, |s| s.len());
        history.push(sequence, snapshot.collected_monotonic_ms, size, ());
        if json {
            let wrapped = with_pressure(&snapshot, &pressure);
            match serde_json::to_string(&wrapped) {
                Ok(line) => {
                    let _ = writeln!(stdout, "{line}");
                    let _ = stdout.flush();
                }
                Err(e) => eprintln!("warning: snapshot serialization failed: {e}"),
            }
        } else {
            let _ = write!(stdout, "{}", render_watch_line(&snapshot));
            let _ = stdout.flush();
        }
    });

    let stats = history.stats();
    eprintln!(
        "watch cancelled after {} snapshot(s); handles released",
        summary.iterations
    );
    eprintln!(
        "history: {} stored, {} gap(s), {} evicted, {} expired, {} rejected",
        stats.stored, stats.gaps, stats.evicted, stats.expired, stats.rejected
    );
    if saw_usable_sample {
        ExitCode::SUCCESS
    } else {
        eprintln!("error: no usable global sample was collected");
        ExitCode::from(1)
    }
}

#[cfg(windows)]
fn run_processes(json: bool, interval_ms: u64) -> ExitCode {
    use ramshaper_platform::ReadOnlyProcessMonitor as _;
    use ramshaper_windows::WindowsSystemMonitor;

    let Ok(config) = WatchConfig::new(interval_ms) else {
        eprintln!("error: invalid interval {interval_ms} ms");
        return ExitCode::from(2);
    };
    let monitor = WindowsSystemMonitor::new();
    let clock = ramshaper_core::SystemClock;

    // Two scans of the same table; rates come from the exact monotonic gap.
    let first = monitor.collect_processes();
    let start = clock.monotonic_ms();
    std::thread::sleep(std::time::Duration::from_millis(config.interval_ms()));
    let second = monitor.collect_processes();
    let elapsed = clock.monotonic_ms().saturating_sub(start);

    if first.scan_failure.is_some() && second.scan_failure.is_some() {
        eprintln!("error: process scans failed: {:?}", second.scan_failure);
        return ExitCode::from(1);
    }
    // The rendered inventory comes from the latest successful scan; the
    // other pass only contributes rate baselines.
    let scan_failed = second.scan_failure.is_some();

    let mut baselines = ramshaper_core::process::RateBaselines::new();
    let _ = baselines.advance(&first, 0); // first pass: establish baselines
    let outcome = baselines.advance(&second, elapsed);
    let (scan_cpus, scan_truncated) = (second.logical_cpu_count, second.truncated);

    let mut entries = outcome.entries;
    // Sort by working set (desc); unavailable values sink to the end.
    entries.sort_by(|a, b| {
        b.working_set
            .value
            .cmp(&a.working_set.value)
            .then_with(|| a.identity.pid.cmp(&b.identity.pid))
    });

    let observed_at = ramshaper_core::rfc3339::format_utc_from_unix_ms(clock.unix_epoch_ms());
    let inventory = ramshaper_core::process::build_inventory(
        entries,
        scan_cpus,
        elapsed,
        observed_at,
        1,
        scan_truncated,
        Some(outcome.churn),
    );

    if json {
        match serde_json::to_string_pretty(&inventory) {
            Ok(out) => println!("{out}"),
            Err(e) => {
                eprintln!("error: failed to serialize inventory: {e}");
                return ExitCode::from(1);
            }
        }
    } else {
        print!("{}", render_processes_text(&inventory));
    }

    // Exit code: scan-level failures or truncation make the output partial.
    if scan_truncated || scan_failed {
        ExitCode::from(3)
    } else {
        ExitCode::SUCCESS
    }
}

/// Text table for the process inventory (presentation boundary).
#[must_use]
fn render_processes_text(inventory: &ramshaper_core::process::ProcessInventory) -> String {
    use ramshaper_core::units::format_bytes;
    let mut out = String::with_capacity(4096);
    out.push_str(&format!(
        "Processes ({} listed, {} logical CPUs, rate window {} ms, observed {})\n",
        inventory.processes.len(),
        inventory.logical_cpu_count,
        inventory.rate_interval_ms,
        inventory.observed_at_utc
    ));
    if let Some(note) = &inventory.truncation_note {
        out.push_str(&format!("TRUNCATED: {note}\n"));
    }
    if let Some(churn) = &inventory.churn {
        out.push_str(&format!(
            "churn vs previous scan: {} appeared, {} vanished\n",
            churn.appeared, churn.vanished
        ));
    }
    out.push_str(&format!(
        "{:>7}  {:<28} {:>12} {:>12} {:>8} {:>12} {:>12}\n",
        "pid", "name", "workingset", "private", "cpu%", "io-read/s", "io-write/s"
    ));
    for entry in &inventory.processes {
        let ws = entry.working_set.value.map_or_else(
            || format!("[{}]", short_status(entry.working_set.status)),
            format_bytes,
        );
        let privb = entry.private_commit.value.map_or_else(
            || format!("[{}]", short_status(entry.private_commit.status)),
            format_bytes,
        );
        let cpu = entry.cpu_percent.rate.map_or_else(
            || format!("[{}]", short_status(entry.cpu_percent.status)),
            |r| format!("{r:.2}"),
        );
        let io_r = entry.io_read.rate.map_or_else(
            || format!("[{}]", short_status(entry.io_read.status)),
            |r| format_bytes(r as u64),
        );
        let io_w = entry.io_write.rate.map_or_else(
            || format!("[{}]", short_status(entry.io_write.status)),
            |r| format_bytes(r as u64),
        );
        let name: String = entry.name.chars().take(28).collect();
        out.push_str(&format!(
            "{:>7}  {:<28} {:>12} {:>12} {:>8} {:>12} {:>12}\n",
            entry.identity.pid, name, ws, privb, cpu, io_r, io_w
        ));
    }
    out.push_str(
        "\nNotes: cpu% is percent of total machine capacity; process I/O is process\n\
         accounting, not physical-disk or paging throughput. Names/parents are labels;\n\
         identity is pid + exact creation FILETIME. [status] marks unavailable values.\n",
    );
    out
}

fn short_status(status: ramshaper_core::MetricStatus) -> &'static str {
    match status {
        ramshaper_core::MetricStatus::Ok => "ok",
        ramshaper_core::MetricStatus::WarmingUp => "warming",
        ramshaper_core::MetricStatus::AccessDenied => "denied",
        ramshaper_core::MetricStatus::Unsupported => "unsup",
        ramshaper_core::MetricStatus::Exited => "exited",
        ramshaper_core::MetricStatus::Error => "error",
        ramshaper_core::MetricStatus::Stale => "stale",
    }
}

/// Collect exactly one snapshot from a monitor (shared by info and watch).
#[cfg(windows)]
fn collect_one<M: ramshaper_platform::ReadOnlySystemMonitor>(
    monitor: &M,
    sequence: u64,
) -> SystemSnapshot {
    let sample = monitor.collect_system();
    let inputs = SnapshotInputs {
        sample,
        os: monitor.os_info(),
        tool: ToolInfo {
            name: TOOL_NAME.to_string(),
            version: VERSION.to_string(),
        },
        agent_instance_id: ramshaper_windows::agent_instance_id(),
        sequence,
        observed_at_unix_ms: ramshaper_core::SystemClock.unix_epoch_ms(),
        monotonic_ms: ramshaper_core::SystemClock.monotonic_ms(),
    };
    build_snapshot(inputs)
}

/// Exit-code contract: 0 ok, 3 partial, 1 no usable global sample.
fn health_to_exit(health: SnapshotHealth) -> ExitCode {
    match health {
        SnapshotHealth::Ok => ExitCode::SUCCESS,
        SnapshotHealth::Partial => ExitCode::from(3),
        SnapshotHealth::NoUsableGlobalSample => ExitCode::from(1),
    }
}

#[cfg(not(windows))]
fn run_info(_json: bool) -> ExitCode {
    eprintln!("error: ramshaper-cli requires the Windows read-only adapter; this platform is not supported");
    ExitCode::from(1)
}

#[cfg(not(windows))]
fn run_watch_command(_json: bool, _interval_ms: u64, _stop_after_ms: Option<u64>) -> ExitCode {
    eprintln!("error: ramshaper-cli requires the Windows read-only adapter; this platform is not supported");
    ExitCode::from(1)
}

#[cfg(not(windows))]
fn run_processes(_json: bool, _interval_ms: u64) -> ExitCode {
    eprintln!("error: ramshaper-cli requires the Windows read-only adapter; this platform is not supported");
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(list: &[&str]) -> ParseOutcome {
        parse_args(list.iter().map(OsString::from))
    }

    fn base_args(command: Command, interval: u64) -> CliArgs {
        CliArgs {
            command,
            json: false,
            interval_ms: Some(interval),
            stop_after_ms: None,
            config_path: None,
        }
    }

    /// Args with no `--interval-ms` flag: the value stays `None` at parse
    /// time and each command resolves its default (config file / built-in).
    fn default_args(command: Command) -> CliArgs {
        CliArgs {
            command,
            json: false,
            interval_ms: None,
            stop_after_ms: None,
            config_path: None,
        }
    }

    /// Regression: the M1 increment passed `config.watch.interval_ms` to the
    /// watch and dropped the explicit `--interval-ms` flag, so any flag value
    /// silently fell back to the config default (1000 ms).
    #[test]
    fn watch_interval_precedence_flag_overrides_config() {
        let config = ramshaper_core::CoreConfig::default();
        // No flag, no config file: built-in default (via the config default).
        assert_eq!(
            watch_interval(&default_args(Command::LowMemoryWatch), &config),
            1000
        );
        // Flag present: it wins.
        assert_eq!(
            watch_interval(&base_args(Command::LowMemoryWatch, 250), &config),
            250
        );

        let with_config = ramshaper_core::CoreConfig::parse(
            "schema_version = 1\n[watch]\ninterval_ms = 5000\nretention_ms = 600000\n",
        )
        .expect("valid");
        // Config value wins when the flag is absent...
        assert_eq!(
            watch_interval(&default_args(Command::LowMemoryWatch), &with_config),
            5000
        );
        // ...and the flag wins when present.
        assert_eq!(
            watch_interval(&base_args(Command::LowMemoryWatch, 250), &with_config),
            250
        );
    }

    #[test]
    fn parses_stop_after_ms_for_watch_only() {
        assert_eq!(
            parse(&["low-memory-watch", "--stop-after-ms", "86400000"]),
            ParseOutcome::Run(CliArgs {
                stop_after_ms: Some(86_400_000),
                ..default_args(Command::LowMemoryWatch)
            })
        );
        // Rejected outside the watch command.
        assert!(matches!(
            parse(&["info", "--stop-after-ms", "86400000"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["processes", "--stop-after-ms", "86400000"]),
            ParseOutcome::UsageError(_)
        ));
        // Rejected out of range and malformed.
        assert!(matches!(
            parse(&["low-memory-watch", "--stop-after-ms", "999"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["low-memory-watch", "--stop-after-ms", "0"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["low-memory-watch", "--stop-after-ms", "abc"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["low-memory-watch", "--stop-after-ms"]),
            ParseOutcome::UsageError(_)
        ));
    }

    #[test]
    fn parses_info_and_flags() {
        assert_eq!(
            parse(&["info"]),
            ParseOutcome::Run(default_args(Command::Info))
        );
        assert_eq!(
            parse(&["info", "--json"]),
            ParseOutcome::Run(CliArgs {
                json: true,
                ..default_args(Command::Info)
            })
        );
        assert_eq!(
            parse(&["low-memory-watch", "--json", "--interval-ms", "250"]),
            ParseOutcome::Run(CliArgs {
                json: true,
                ..base_args(Command::LowMemoryWatch, 250)
            })
        );
        // processes has its own default interval, resolved at run time.
        assert_eq!(
            parse(&["processes"]),
            ParseOutcome::Run(default_args(Command::Processes))
        );
        assert_eq!(
            parse(&["processes", "--interval-ms", "1500"]),
            ParseOutcome::Run(base_args(Command::Processes, 1500))
        );
        assert_eq!(
            parse(&["--json", "low-memory-watch"]),
            ParseOutcome::Run(CliArgs {
                json: true,
                ..default_args(Command::LowMemoryWatch)
            })
        );
        // Config path parsed.
        match parse(&["--config", "C:\\tmp\\rs.toml", "info"]) {
            ParseOutcome::Run(args) => {
                assert_eq!(args.config_path, Some(PathBuf::from("C:\\tmp\\rs.toml")))
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn version_and_help_win() {
        assert_eq!(parse(&["--version"]), ParseOutcome::Version);
        assert_eq!(parse(&["-V"]), ParseOutcome::Version);
        assert_eq!(parse(&["--help"]), ParseOutcome::Help);
        assert_eq!(parse(&["-h"]), ParseOutcome::Help);
        assert_eq!(parse(&["info", "--help"]), ParseOutcome::Help);
    }

    #[test]
    fn usage_errors() {
        assert!(matches!(parse(&[]), ParseOutcome::UsageError(_)));
        assert!(matches!(parse(&["bogus"]), ParseOutcome::UsageError(_)));
        assert!(matches!(
            parse(&["--nope", "info"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["info", "info"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["--interval-ms"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["--interval-ms", "abc"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["low-memory-watch", "--interval-ms", "1"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["low-memory-watch", "--interval-ms", "99999999999"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["info", "--interval-ms", "500"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(parse(&["--config"]), ParseOutcome::UsageError(_)));
    }

    #[test]
    fn exit_code_contract() {
        assert_eq!(health_to_exit(SnapshotHealth::Ok), ExitCode::SUCCESS);
        assert_eq!(health_to_exit(SnapshotHealth::Partial), ExitCode::from(3));
        assert_eq!(
            health_to_exit(SnapshotHealth::NoUsableGlobalSample),
            ExitCode::from(1)
        );
    }

    #[test]
    fn help_documents_new_commands() {
        assert!(HELP.contains("processes"));
        assert!(HELP.contains("--config"));
    }
}
