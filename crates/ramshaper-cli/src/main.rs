//! `ramshaper-cli` — diagnostics + the gated managed launcher (G0:
//! RS-001..RS-006; M1: RS-007..009; G4: RS-023).
//!
//! Commands: `info [--json]`, `low-memory-watch [--json] [--interval-ms]`,
//! `processes [--json] [--interval-ms]`, `policies --dry-run [--json]`,
//! `config export|import`, `launch [options] -- <PATH> [ARGS...]`,
//! `--version`, `--help`.
//! Contracts (doc 14 §8): stdout carries data only, diagnostics go to
//! stderr; exit codes 0 success (incl. healthy Ctrl+C cancel), 1 runtime
//! failure / no usable global sample / declined consent, 2 usage or
//! configuration error, 3 partial snapshot. Observation commands never
//! mutate anything; `launch` creates a NEW process the operator typed a
//! per-launch consent for, into a fresh advisory job — it never touches
//! existing processes (RS-023).

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
/// Maximum advisory CPU threshold in milliseconds: the value converts to
/// 100-ns units (×10_000) and must stay inside `i64` (the Win32 unit).
const MAX_ADVISORY_CPU_MS: u64 = (i64::MAX / 10_000) as u64;
/// Default launch monitoring interval (`--interval-ms` overrides).
const DEFAULT_LAUNCH_INTERVAL_MS: u64 = 1_000;

const HELP: &str = "\
ramshaper-cli — Windows memory diagnostics + the gated managed launcher

USAGE:
    ramshaper-cli <COMMAND> [OPTIONS]

COMMANDS:
    info                 Print one system memory snapshot (with pressure state)
    low-memory-watch     Sample system memory + low-memory condition until Ctrl+C
    processes            List processes with memory, CPU and I/O (two-sample rates)
    policies             Inspect the policy decision layer (dry run only)
    config               Validate, export or atomically import TOML configuration
    launch               Launch ONE explicitly chosen executable into a fresh
                         advisory job (typed per-launch consent; RS-023)
    restore              Ask the agent to restore one action or disable-all +
                         restore (safe direction; needs a policy-gate agent)

CONFIG USAGE:
    ramshaper-cli config export [--config <PATH>] <DEST>
    ramshaper-cli config import <SOURCE> --into <DEST>

POLICIES USAGE:
    ramshaper-cli policies --dry-run [--json] [--trace <FILE>]

    Simulates the policy decision layer over a SYNTHETIC trace: no OS
    reads, no OS writes, no real process is touched. Without --trace the
    built-in demo trace runs; the same command always prints byte-
    identical output. With --trace, FILE (schema ramshaper.policy-trace.v1,
    closed JSON) supplies the walk: a target identity, an assumed
    allowlist (a walk input, never evidence) and ordered steps of
    observations (low_memory true/false/null, per-step target facts,
    confirm/disable/config/journal flags). Output stays SYNTHETIC-marked.
    A real policy experiment requires the agent's typed console consent
    (the `--policy-journal` flag on ramshaper-agent) and an allowlisted
    target (none exist until RS-024 Stage B testing).

LAUNCH USAGE:
    ramshaper-cli launch [--json] [--interval-ms <MS>] [--stop-after-ms <MS>]
                         [--advisory-max-active <N>] [--advisory-max-cpu-ms <MS>]
                         -- <PATH> [ARGS...]

    Launches exactly the named image (no PATH search, no shell, no
    shortcut/association resolution; shortcuts, scripts and non-executable
    files fail explicitly) into a fresh ADVISORY job and prints group
    accounting until Ctrl+C or --stop-after-ms, then REMOVES MONITORING and
    exits. The prompt shows the exact command line, preflight facts and the
    semantics; type LAUNCH (exactly, alone) to consent to this one launch.
    A consent buys at most one attempt; a declined or failed launch is
    never retried. Semantics (stated on every launch): the job is NOT a
    sandbox and enforces nothing; membership is NOT detachable and lasts
    for the process lifetime; removing monitoring does not detach or stop
    the application; no kill-on-close, hard, time or process-count limits
    exist. Existing processes are never touched (RS-023).

RESTORE USAGE:
    ramshaper-cli restore [--pipe-name <NAME>] (--action <ID> | --disable-all) [--json]

    Asks the session agent to restore one policy action (by durable id) or
    to disable all actions and restore every outstanding record. The agent
    must have been started with --policy-journal; agents without the
    policy gate answer Unsupported (the refusal is printed, never guessed).
    Restore is the SAFE direction: recovery is never blocked by consent,
    an external change to the setting is preserved (Conflict), and a dead
    target resolves without writing. Exit 0 restored (or scheduled),
    1 refused/failed (the reason is printed), 2 usage.

`config export` writes validated TOML to DEST (defaults when --config is
absent). `config import` validates SOURCE against the closed schema
(field-level errors on any violation) and atomically replaces DEST; the
parsed mode must be observe_only, and profiles/exclusions are stored as
inert labels that never enable actions. Both actions touch only the named
file - no OS settings, no other files (RS-012).

OPTIONS:
    --json               JSON output (info/processes: one versioned object;
                         watch/launch samples: newline-delimited objects plus
                         a final summary for launch)
    --interval-ms <MS>   watch: poll interval (default 1000)
                         processes: rate window between two scans (default 500)
                         launch: monitoring interval (default 1000)
                         range 100..=3600000
    --stop-after-ms <MS> watch: stop gracefully after MS milliseconds and
                         print the history summary (default: run until
                         Ctrl+C); range 1000..=2592000000
                         launch: remove monitoring after MS milliseconds
    --advisory-max-active <N>      launch: advisory live-member threshold
                                   (0 = not evaluated, the default)
    --advisory-max-cpu-ms <MS>     launch: advisory accumulated-CPU threshold
                                   in milliseconds (0 = not evaluated, the
                                   default); exceeding is REPORTED, never
                                   enforced
    --into <DEST>        config import: destination file, replaced atomically
    --config <PATH>      Load a TOML configuration file (schema_version 1,
                         observe-only; invalid files exit 2)
    --trace <FILE>       policies --dry-run: walk a user-authored
                         ramshaper.policy-trace.v1 file instead of the
                         built-in demo trace (still synthetic; invalid
                         files exit 2)
    --version, -V        Print version
    --help, -h           Print this help

EXIT CODES:
    0  success (including a healthy Ctrl+C-cancelled watch or monitoring
       removal)
    1  runtime failure / no usable global sample / launch declined or
       refused
    2  usage or configuration error (including an invalid launch request)
    3  partial snapshot (optional metric/truncation reported)

Per-process access_denied/exited statuses are expected on protected processes
and do not fail the processes command. The observation commands are strictly
read-only: they query OS counters and conditions and never change process
settings, priorities, working sets or jobs. `launch` is the one gated
exception: it starts a NEW process you explicitly consent to, into a fresh
advisory job, and never modifies an existing process. No network connection,
telemetry or update check is performed.
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
    /// Raw `--dry-run` flag; valid only for `policies` (its only mode —
    /// the demo trace is synthetic by definition).
    dry_run: bool,
    /// `--trace <FILE>`: a user-authored `ramshaper.policy-trace.v1` walk
    /// instead of the built-in demo trace (still synthetic: no OS reads,
    /// no OS writes).
    trace: Option<PathBuf>,
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

/// The `launch` request and its advisory thresholds (RS-023). Everything
/// after `--` is the request: the first token is the exact image path, the
/// rest is the verbatim argument string. The consent is NOT part of the
/// spec — it is typed at run time or the launch never happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    executable: OsString,
    arguments: String,
    /// Advisory live-member threshold (0 = not evaluated).
    advisory_max_active: u32,
    /// Advisory accumulated-CPU threshold in milliseconds (0 = not
    /// evaluated; converted to 100-ns units at run time).
    advisory_max_cpu_ms: u64,
}

impl LaunchSpec {
    fn empty() -> Self {
        Self {
            executable: OsString::new(),
            arguments: String::new(),
            advisory_max_active: 0,
            advisory_max_cpu_ms: 0,
        }
    }
}

/// The `restore` request (contract 14 §8 CLI surface): one verb per
/// invocation. The consent gate lives on the agent; this command only
/// speaks the safe direction over the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreSpec {
    /// `None` = derive from the logon session (the agent's own default).
    pub pipe_name: Option<String>,
    /// Restore one action by durable id.
    pub action: Option<String>,
    /// Disable all actions and restore every outstanding record.
    pub disable_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Info,
    LowMemoryWatch,
    Processes,
    Policies,
    Config(ConfigSpec),
    Launch(LaunchSpec),
    Restore(RestoreSpec),
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
    let mut dry_run = false;
    let mut interval_ms: Option<u64> = None;
    let mut stop_after_ms: Option<u64> = None;
    let mut config_path: Option<PathBuf> = None;
    let mut trace: Option<PathBuf> = None;
    let mut into: Option<PathBuf> = None;
    let mut advisory_active_seen = false;
    let mut advisory_cpu_seen = false;
    let mut advisory_max_active: Option<u32> = None;
    let mut advisory_max_cpu_ms: Option<u64> = None;

    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "--version" | "-V" => return ParseOutcome::Version,
            "--help" | "-h" => return ParseOutcome::Help,
            "--json" => json = true,
            "--dry-run" => dry_run = true,
            "--config" => match args.next() {
                Some(value) => config_path = Some(PathBuf::from(value)),
                None => return ParseOutcome::UsageError("--config requires a path".into()),
            },
            "--pipe-name" => match args.next() {
                Some(value) => {
                    if let Some(Command::Restore(spec)) = &mut command {
                        spec.pipe_name = Some(value.to_string_lossy().to_string());
                    } else {
                        return ParseOutcome::UsageError(
                            "--pipe-name is only valid for restore".into(),
                        );
                    }
                }
                None => return ParseOutcome::UsageError("--pipe-name requires a value".into()),
            },
            "--action" => match args.next() {
                Some(value) => {
                    if let Some(Command::Restore(spec)) = &mut command {
                        spec.action = Some(value.to_string_lossy().to_string());
                    } else {
                        return ParseOutcome::UsageError(
                            "--action is only valid for restore".into(),
                        );
                    }
                }
                None => return ParseOutcome::UsageError("--action requires a value".into()),
            },
            "--disable-all" => {
                if let Some(Command::Restore(spec)) = &mut command {
                    spec.disable_all = true;
                } else {
                    return ParseOutcome::UsageError(
                        "--disable-all is only valid for restore".into(),
                    );
                }
            }
            "--trace" => match args.next() {
                Some(value) => trace = Some(PathBuf::from(value)),
                None => return ParseOutcome::UsageError("--trace requires a path".into()),
            },
            "--advisory-max-active" => match args.next() {
                Some(value) => match value.to_string_lossy().parse::<u32>() {
                    Ok(count) => {
                        advisory_active_seen = true;
                        advisory_max_active = Some(count);
                    }
                    Err(_) => {
                        return ParseOutcome::UsageError(format!(
                            "--advisory-max-active expects a number, got {}",
                            value.to_string_lossy()
                        ));
                    }
                },
                None => {
                    return ParseOutcome::UsageError(
                        "--advisory-max-active requires a value".into(),
                    );
                }
            },
            "--advisory-max-cpu-ms" => match args.next() {
                Some(value) => match value.to_string_lossy().parse::<u64>() {
                    Ok(ms) if ms <= MAX_ADVISORY_CPU_MS => {
                        advisory_cpu_seen = true;
                        advisory_max_cpu_ms = Some(ms);
                    }
                    Ok(ms) => {
                        return ParseOutcome::UsageError(format!(
                            "--advisory-max-cpu-ms {ms} out of range (0..={MAX_ADVISORY_CPU_MS})"
                        ));
                    }
                    Err(_) => {
                        return ParseOutcome::UsageError(format!(
                            "--advisory-max-cpu-ms expects a number, got {}",
                            value.to_string_lossy()
                        ));
                    }
                },
                None => {
                    return ParseOutcome::UsageError(
                        "--advisory-max-cpu-ms requires a value".into(),
                    );
                }
            },
            "--" => {
                // Everything after `--` is the launch request: the first
                // token is the exact image path, the rest is the verbatim
                // argument string. No flag is interpreted past this point.
                let Some(Command::Launch(spec)) = &mut command else {
                    return ParseOutcome::UsageError(
                        "-- is only valid for launch (launch -- <PATH> [ARGS...])".into(),
                    );
                };
                let tokens = args.by_ref();
                let Some(executable) = tokens.next() else {
                    return ParseOutcome::UsageError(
                        "launch requires a request after `--` (launch -- <PATH> [ARGS...])".into(),
                    );
                };
                spec.executable = executable;
                let mut arguments = String::new();
                for (index, token) in tokens.enumerate() {
                    let Some(text) = token.to_str() else {
                        return ParseOutcome::UsageError(format!(
                            "launch argument {index} after `--` is not valid UTF-8; the \
                             request must be expressible as text"
                        ));
                    };
                    if !arguments.is_empty() {
                        arguments.push(' ');
                    }
                    arguments.push_str(text);
                }
                spec.arguments = arguments;
            }
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
            "policies" if command.is_none() => command = Some(Command::Policies),
            "launch" if command.is_none() => command = Some(Command::Launch(LaunchSpec::empty())),
            "restore" if command.is_none() => {
                command = Some(Command::Restore(RestoreSpec {
                    pipe_name: None,
                    action: None,
                    disable_all: false,
                }));
            }
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
                if matches!(command, Some(Command::Launch(_))) && !is_flag {
                    return ParseOutcome::UsageError(
                        "launch takes its request after `--` (launch -- <PATH> [ARGS...])".into(),
                    );
                }
                return ParseOutcome::UsageError(if command.is_none() {
                    format!("unknown command '{other}'")
                } else {
                    format!("unexpected argument '{other}'")
                });
            }
        }
    }

    let Some(mut command) = command else {
        return ParseOutcome::UsageError("missing command".into());
    };
    if command == Command::Info && interval_ms.is_some() {
        return ParseOutcome::UsageError("--interval-ms is not valid for info".into());
    }
    if stop_after_ms.is_some() && !matches!(command, Command::LowMemoryWatch | Command::Launch(_)) {
        return ParseOutcome::UsageError(
            "--stop-after-ms is only valid for low-memory-watch and launch".into(),
        );
    }
    if dry_run && command != Command::Policies {
        return ParseOutcome::UsageError("--dry-run is only valid for policies".into());
    }
    if trace.is_some() && command != Command::Policies {
        return ParseOutcome::UsageError("--trace is only valid for policies --dry-run".into());
    }
    if command == Command::Policies && !dry_run {
        return ParseOutcome::UsageError(
            "policies requires --dry-run (the only supported mode)".into(),
        );
    }
    if (advisory_active_seen || advisory_cpu_seen) && !matches!(command, Command::Launch(_)) {
        return ParseOutcome::UsageError(
            "--advisory-max-active/--advisory-max-cpu-ms are only valid for launch".into(),
        );
    }
    if let Command::Restore(spec) = &mut command {
        // Exactly one verb per invocation; the pipe name may default.
        if spec.action.is_some() && spec.disable_all {
            return ParseOutcome::UsageError(
                "restore takes --action or --disable-all, not both".into(),
            );
        }
        if spec.action.is_none() && !spec.disable_all {
            return ParseOutcome::UsageError(
                "restore requires --action <ID> or --disable-all".into(),
            );
        }
    }
    if let Command::Launch(spec) = &mut command {
        if spec.executable.is_empty() {
            return ParseOutcome::UsageError(
                "launch requires a request after `--` (launch -- <PATH> [ARGS...])".into(),
            );
        }
        if let Some(count) = advisory_max_active {
            spec.advisory_max_active = count;
        }
        if let Some(ms) = advisory_max_cpu_ms {
            spec.advisory_max_cpu_ms = ms;
        }
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
        dry_run,
        trace,
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
            eprintln!("usage: {TOOL_NAME} <info|low-memory-watch|processes|policies|config|launch|restore> [--json] [--interval-ms <ms>] [--stop-after-ms <ms>] [--dry-run] [--config <path>] [--into <path>]");
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

/// `launch` (RS-023): one typed-consented managed launch with advisory
/// monitoring, then monitoring REMOVAL (a handle drop — never a detach,
/// never a kill). stdout carries the launch record, monitoring samples and
/// the removal summary; stderr carries the prompt, preflight notes and
/// warnings. Exit 0 launched+removed, 1 declined or refused (explicit,
/// never retried), 2 invalid request.
#[cfg(windows)]
fn run_launch(
    spec: &LaunchSpec,
    json: bool,
    interval_ms: u64,
    stop_after_ms: Option<u64>,
) -> ExitCode {
    use std::io::{BufRead as _, Read as _};
    use std::time::Duration;

    use ramshaper_core::watch::{WaitHandle as _, WaitOutcome};
    use ramshaper_windows::console::ConsoleCancel;
    use ramshaper_windows::job::{
        compatibility_report, evaluate_advisory, AdvisoryStatus, AdvisoryThresholds,
    };
    use ramshaper_windows::launch::{
        consent_decide, render_consent_prompt, LaunchRequest, ManagedLaunchSession,
    };

    let Ok(config) = WatchConfig::new(interval_ms) else {
        eprintln!("error: invalid interval {interval_ms} ms");
        return ExitCode::from(2);
    };
    let interval_ms = config.interval_ms();

    let request = LaunchRequest::new(spec.executable.clone(), spec.arguments.clone());
    if let Err(error) = request.validate() {
        eprintln!("error: {error}");
        return ExitCode::from(2);
    }

    // Preflight before the prompt (the launch captures its own copy).
    let preflight = match compatibility_report(None, None) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("error: launcher preflight failed: {error:?}");
            return ExitCode::from(1);
        }
    };
    for note in &preflight.notes {
        eprintln!("preflight: {note}");
    }

    // The typed consent: bounded read (the word is 6 bytes; the 4 KiB cap
    // is the bounded-memory rule shared with the agent's consent path).
    eprint!("{}", render_consent_prompt(&request, &preflight));
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let read = std::io::stdin().lock().take(4_096).read_line(&mut line);
    let typed = match read {
        Ok(0) => None, // end of input: a decline, never a guess
        Ok(_) => Some(line),
        Err(_) => None,
    };
    let Some(consent) = consent_decide(typed.as_deref()) else {
        eprintln!("launch declined; nothing was started");
        return ExitCode::from(1);
    };

    let session = match ManagedLaunchSession::launch(request, consent) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(1);
        }
    };
    let pid = session.child_pid();
    let command_line = session.request().command_line();
    let thresholds = AdvisoryThresholds {
        max_active_processes: spec.advisory_max_active,
        max_total_cpu_time_100ns: advisory_cpu_100ns(spec.advisory_max_cpu_ms),
    };

    if json {
        let record = serde_json::json!({
            "schema": "ramshaper.launch.v1",
            "event": "launched",
            "pid": pid,
            "request": {
                "executable": session.request().executable.to_string_lossy(),
                "arguments": session.request().arguments,
                "command_line": command_line,
            },
            "preflight_notes": session.preflight().notes,
            "limit_flags": session.limit_flags().ok(),
        });
        println!("{record}");
    } else {
        println!("launched pid {pid}: {command_line}");
    }

    let Ok(cancel) = ConsoleCancel::install() else {
        eprintln!("error: failed to install the Ctrl+C handler; refusing an uncancellable monitor");
        drop(session); // remove monitoring: drop the view, the app keeps running
        eprintln!("monitoring removed; pid {pid} keeps running");
        return ExitCode::from(1);
    };
    let cancel = std::sync::Arc::new(cancel);
    if let Some(stop_after_ms) = stop_after_ms {
        eprintln!("removing monitoring after {stop_after_ms} ms (the application keeps running)");
        let timer_cancel = std::sync::Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(stop_after_ms));
            timer_cancel.trigger();
        });
    }
    eprintln!(
        "monitoring pid {pid} every {interval_ms} ms; Ctrl+C removes monitoring (the application keeps running)"
    );

    let mut samples: u64 = 0;
    loop {
        samples += 1;
        match session.accounting() {
            Ok(accounting) => {
                let flags = session.limit_flags().ok();
                let advisory_codes = match evaluate_advisory(&accounting, &thresholds) {
                    AdvisoryStatus::Within => Vec::new(),
                    AdvisoryStatus::Exceeded(exceeded) => exceeded.codes(),
                };
                if json {
                    let sample = serde_json::json!({
                        "schema": "ramshaper.launch-sample.v1",
                        "sequence": samples,
                        "pid": pid,
                        "active_processes": accounting.active_processes,
                        "total_cpu_time_100ns": accounting.total_cpu_time_100ns(),
                        "limit_flags": flags,
                        "advisory_exceeded": advisory_codes,
                    });
                    println!("{sample}");
                } else {
                    let advisory = if advisory_codes.is_empty() {
                        String::new()
                    } else {
                        format!("  advisory exceeded: {}", advisory_codes.join(", "))
                    };
                    println!(
                        "sample {:>4}  pid {}  active {}  cpu {:.3} s  flags {}{advisory}",
                        samples,
                        pid,
                        accounting.active_processes,
                        accounting.total_cpu_time_100ns() as f64 / 10_000_000.0,
                        flags.map_or("unavailable".to_string(), |f| f.to_string()),
                    );
                }
                let _ = std::io::stdout().flush();
            }
            Err(error) => eprintln!("warning: accounting query failed: {error:?}"),
        }
        match cancel.wait(interval_ms.min(u32::MAX as u64) as u32) {
            WaitOutcome::Signaled => break,
            WaitOutcome::TimedOut => {}
        }
    }

    let preflight_notes = session.preflight().notes.clone();
    let removed = session.remove_monitoring();
    if json {
        let final_obj = serde_json::json!({
            "schema": "ramshaper.launch.v1",
            "event": "monitoring_removed",
            "pid": removed.child_pid,
            "request": {
                "executable": spec.executable.to_string_lossy(),
                "arguments": spec.arguments,
                "command_line": command_line,
            },
            "preflight_notes": preflight_notes,
            "final_limit_flags": removed.final_limit_flags,
            "final_accounting": removed.final_accounting.map(|a| serde_json::json!({
                "total_processes": a.total_processes,
                "active_processes": a.active_processes,
                "terminated_processes": a.terminated_processes,
                "total_cpu_time_100ns": a.total_cpu_time_100ns(),
                "total_page_faults": a.total_page_faults,
            })),
            "statements": removed.statements,
            "samples": samples,
        });
        println!("{final_obj}");
    } else {
        println!();
        println!(
            "monitoring removed for pid {} (launched {command_line})",
            removed.child_pid
        );
        match removed.final_accounting {
            Some(accounting) => println!(
                "  final accounting: active {}, total processes {}, cpu {:.3} s, page faults {}",
                accounting.active_processes,
                accounting.total_processes,
                accounting.total_cpu_time_100ns() as f64 / 10_000_000.0,
                accounting.total_page_faults,
            ),
            None => println!("  final accounting: unavailable (the final query failed)"),
        }
        println!(
            "  final limit flags: {}",
            removed
                .final_limit_flags
                .map_or("unavailable".to_string(), |f| f.to_string())
        );
        for statement in &removed.statements {
            println!("  semantics: {statement}");
        }
        println!("the application keeps running; monitoring was removed, nothing was detached or stopped");
    }
    ExitCode::SUCCESS
}

/// Advisory CPU threshold conversion: milliseconds to 100-ns units (the
/// parse-time cap keeps the product inside `i64`).
fn advisory_cpu_100ns(ms: u64) -> i64 {
    i64::try_from(ms.saturating_mul(10_000)).unwrap_or(i64::MAX)
}

/// The restore wire client (contract 14 §8): one fresh connection, one
/// verb, the response verbatim. The agent enforces every rule — this
/// command adds no authority; a gateless agent answers Unsupported and
/// the refusal is the outcome.
#[cfg(windows)]
fn restore_rpc(
    pipe_name: &str,
    request_id: &str,
    command: &str,
    payload: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use ramshaper_core::ipc::RequestEnvelope;
    use ramshaper_windows::pipe::{client_connect, read_frame, write_frame};

    const PREFIX: &str = "\\\\.\\pipe\\";
    let name = if pipe_name.starts_with(PREFIX) {
        pipe_name.to_string()
    } else {
        format!("{PREFIX}{pipe_name}")
    };
    let client = client_connect(&name).map_err(|error| format!("agent not reachable: {error}"))?;
    let envelope = RequestEnvelope {
        protocol_version: ramshaper_core::ipc::PROTOCOL_VERSION,
        request_id: request_id.to_string(),
        agent_instance_id: None,
        command: command.to_string(),
        payload,
    };
    let body = serde_json::to_vec(&envelope).map_err(|error| format!("encode failed: {error}"))?;
    write_frame(&client, &body).map_err(|error| format!("write failed: {error}"))?;
    let frame = read_frame(&client).map_err(|error| format!("read failed: {error}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&frame).map_err(|error| format!("malformed response: {error}"))?;
    if value["ok"] == true {
        Ok(value["data"].clone())
    } else {
        Err(format!(
            "{}: {}",
            value["error"]["code"].as_str().unwrap_or("unknown"),
            value["error"]["message"].as_str().unwrap_or("")
        ))
    }
}

/// `restore` (contract 14 §8): the safe direction over the wire. stdout
/// carries the outcome JSON; stderr the refusals. Exit 0 restored or
/// scheduled, 1 refused/failed (Unsupported, Conflict, unreachable...),
/// 2 usage (handled at parse time).
#[cfg(windows)]
fn run_restore(spec: &RestoreSpec, json: bool) -> ExitCode {
    use ramshaper_core::ipc::{
        DisableActionsRequest, RestoreActionRequest, COMMAND_DISABLE_ACTIONS,
        COMMAND_RESTORE_ACTION,
    };

    let pipe_name = match &spec.pipe_name {
        Some(name) => name.clone(),
        None => match ramshaper_agent::server::default_pipe_name() {
            Ok(name) => name,
            Err(error) => {
                eprintln!("error: cannot derive the default pipe name: {error}");
                eprintln!("pass --pipe-name explicitly");
                return ExitCode::from(1);
            }
        },
    };

    let (command, payload, request_id) = if let Some(action_id) = &spec.action {
        (
            COMMAND_RESTORE_ACTION,
            serde_json::to_value(RestoreActionRequest {
                action_id: action_id.clone(),
            })
            .expect("restore payload serializes"),
            "cli-restore".to_string(),
        )
    } else {
        (
            COMMAND_DISABLE_ACTIONS,
            serde_json::to_value(DisableActionsRequest {}).expect("disable payload serializes"),
            "cli-disable-all".to_string(),
        )
    };

    match restore_rpc(&pipe_name, &request_id, command, Some(payload)) {
        Ok(data) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "schema": "ramshaper.restore.v1",
                        "verb": command,
                        "outcome": data,
                    })
                );
            } else if let Some(action_id) = &spec.action {
                let outcome = data["outcome"].as_str().unwrap_or("unknown");
                match outcome {
                    "restored" => println!(
                        "action {action_id} restored (wrote_back: {})",
                        data["wrote_back"].as_bool().unwrap_or(false)
                    ),
                    "retry_scheduled" => println!(
                        "action {action_id}: restore retry scheduled (due at {} ms)",
                        data["due_unix_ms"]
                    ),
                    "manual_retry" => println!(
                        "action {action_id}: bounded retries exhausted; the record stays pending for manual retry"
                    ),
                    other => println!("action {action_id}: {other}"),
                }
            } else {
                println!(
                    "disable-all: {} restored/resolved",
                    data["restored"].as_u64().unwrap_or(0)
                );
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("nothing was changed by this command");
            ExitCode::from(1)
        }
    }
}

/// The restore command requires the Windows pipe adapter.
#[cfg(not(windows))]
fn run_restore(_spec: &RestoreSpec, _json: bool) -> ExitCode {
    eprintln!("error: restore requires the Windows adapter; this platform is not supported");
    ExitCode::from(1)
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
        Command::Policies => run_policies(args.json, args.trace.as_ref()),
        Command::Restore(spec) => run_restore(&spec, args.json),
        Command::Launch(spec) => run_launch(
            &spec,
            args.json,
            args.interval_ms.unwrap_or(DEFAULT_LAUNCH_INTERVAL_MS),
            args.stop_after_ms,
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
/// `policies --dry-run`: the deterministic RS-019 decision walk over the
/// built-in synthetic trace (contract 14 §8). No OS reads, no OS writes,
/// no clock — the same trace always produces byte-identical output. The
/// allowlist carries only the trace's own synthetic image (production
/// allowlists stay empty until RS-024 Stage B evidence exists).
/// `policies --dry-run [--trace <FILE>] [--json]`: the deterministic RS-019
/// decision walk. Without `--trace` the built-in demo trace runs (pinned
/// byte-identical). With `--trace` a user-authored
/// `ramshaper.policy-trace.v1` file is walked instead — still SYNTHETIC
/// (no OS reads, no OS writes; the file's allowlist is a walk input, never
/// evidence). A missing/unparsable trace file is a configuration error
/// (exit 2).
fn run_policies(json: bool, trace_path: Option<&PathBuf>) -> ExitCode {
    let rules = ramshaper_core::policy::PolicyRules::default_experiment();

    // A user-authored trace file: read, parse, walk, print, done (still
    // SYNTHETIC — the walk performs no OS reads and no OS writes).
    if let Some(path) = trace_path {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: cannot read trace {}: {error}", path.display());
                return ExitCode::from(2);
            }
        };
        let parsed = match ramshaper_core::policy::ParsedTrace::parse(&text) {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!("error: {} is not a valid trace: {error}", path.display());
                return ExitCode::from(2);
            }
        };
        let names: Vec<&str> = parsed
            .allowlist_names()
            .iter()
            .map(String::as_str)
            .collect();
        let allowlist =
            ramshaper_core::eligibility::Allowlist::new(rules.rules_version.as_str(), &names);
        let identity = parsed.target_identity();
        let config = ramshaper_core::policy::DryRunConfig {
            rules: &rules,
            allowlist: &allowlist,
            user_exclusions: &[],
            target_identity: &identity,
        };
        let steps = parsed.steps();
        let log = ramshaper_core::policy::dry_run(&config, &steps);
        print_log(
            json,
            &log,
            &format!(
                "policies --dry-run --trace {}: SYNTHETIC trace — no OS reads, no OS writes",
                path.display()
            ),
        );
        return ExitCode::SUCCESS;
    }

    let allowlist = ramshaper_core::eligibility::Allowlist::new(
        rules.rules_version.as_str(),
        &["SYNTHETIC-APP.EXE"],
    );
    let identity = ramshaper_core::identity::ProcessIdentity::new(4242, 133_000_000_000_000_000);
    let config = ramshaper_core::policy::DryRunConfig {
        rules: &rules,
        allowlist: &allowlist,
        user_exclusions: &[],
        target_identity: &identity,
    };
    let log = ramshaper_core::policy::dry_run(&config, &ramshaper_core::policy::demo_trace());
    print_log(
        json,
        &log,
        "policies --dry-run: SYNTHETIC trace — no OS reads, no OS writes",
    );
    // Deterministic success: the dry run cannot fail at runtime (§8 exit
    // codes; a usage error already exited 2 during parsing).
    ExitCode::SUCCESS
}

/// The shared output shape: JSON = the versioned log object verbatim;
/// text = SYNTHETIC-marked header, the walk, the summary.
fn print_log(json: bool, log: &ramshaper_core::policy::DryRunLog, source_label: &str) {
    if json {
        match serde_json::to_string_pretty(log) {
            Ok(out) => println!("{out}"),
            Err(e) => {
                eprintln!("error: failed to serialize the dry-run log: {e}");
            }
        }
    } else {
        println!("{source_label}");
        println!(
            "schema {} (rules {}, pressure {})",
            log.schema, log.rules_version, log.pressure_rule_version
        );
        for step in &log.steps {
            println!("{}", render_dry_run_step(step));
        }
        let s = &log.summary;
        println!(
            "summary: {} previews offered, {} applies simulated, {} resolved, {} replays ignored, {} no-action steps",
            s.previews_offered, s.applies_simulated, s.actions_resolved, s.replays_ignored, s.no_actions
        );
    }
}

/// One human-readable decision line.
fn render_dry_run_step(step: &ramshaper_core::policy::StepDecision) -> String {
    use ramshaper_core::policy::PolicyDecision;
    let decision = match &step.decision {
        PolicyDecision::NoAction { reason } => {
            let detail = match reason {
                ramshaper_core::policy::NoActionReason::ActionActive { action_id } => {
                    format!(" ({action_id} outstanding)")
                }
                ramshaper_core::policy::NoActionReason::Ineligible { denies } => format!(
                    " ({})",
                    denies
                        .iter()
                        .map(|d| d.code().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                ramshaper_core::policy::NoActionReason::PreviewExpired {
                    offered_at_ms,
                    confirmed_at_ms,
                } => format!(" (offered {offered_at_ms}, confirmed {confirmed_at_ms})"),
                _ => String::new(),
            };
            format!("no_action {}{detail}", reason.code())
        }
        PolicyDecision::PreviewOffered {
            action_id, ttl_ms, ..
        } => format!("preview {action_id} (ttl {ttl_ms} ms; confirmation required)"),
        PolicyDecision::ApplySimulated {
            action_id,
            deadline_monotonic_ms,
        } => format!("apply {action_id} (simulated; deadline {deadline_monotonic_ms} ms)"),
        PolicyDecision::ActionResolved {
            action_id,
            cause,
            final_state,
        } => format!(
            "resolved {action_id} ({} -> {:?})",
            cause.code(),
            final_state
        ),
        PolicyDecision::ReplayIgnored { action_id } => {
            format!("replay {action_id} (already applied; no second write)")
        }
    };
    format!(
        "[{:>2}] {:>7} ms  {:<9}  {}",
        step.step,
        step.monotonic_ms,
        step.pressure_level.as_str(),
        decision
    )
}

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

#[cfg(not(windows))]
fn run_launch(
    _spec: &LaunchSpec,
    _json: bool,
    _interval_ms: u64,
    _stop_after_ms: Option<u64>,
) -> ExitCode {
    eprintln!("error: ramshaper-cli requires the Windows adapter; this platform is not supported");
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
            dry_run: false,
            trace: None,
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
            dry_run: false,
            trace: None,
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

    #[test]
    fn policies_requires_dry_run_and_vice_versa() {
        // The command's only mode is the synthetic dry run.
        assert!(matches!(
            parse(&["policies"]),
            ParseOutcome::UsageError(ref m) if m.contains("--dry-run")
        ));
        assert!(matches!(
            parse(&["--dry-run", "info"]),
            ParseOutcome::UsageError(ref m) if m.contains("--dry-run")
        ));
        assert!(matches!(
            parse(&["info", "--dry-run"]),
            ParseOutcome::UsageError(ref m) if m.contains("--dry-run")
        ));
        match parse(&["policies", "--dry-run"]) {
            ParseOutcome::Run(args) => {
                assert_eq!(args.command, Command::Policies);
                assert!(args.dry_run);
            }
            other => panic!("policies --dry-run must parse: {other:?}"),
        }
        // Flag order does not matter.
        assert!(matches!(
            parse(&["--dry-run", "policies", "--json"]),
            ParseOutcome::Run(_)
        ));
    }

    #[test]
    fn help_documents_the_policies_command() {
        assert!(HELP.contains("policies"));
        assert!(HELP.contains("--dry-run"));
        assert!(HELP.contains("SYNTHETIC"));
    }

    fn launch_spec(executable: &str, arguments: &str) -> LaunchSpec {
        LaunchSpec {
            executable: OsString::from(executable),
            arguments: arguments.to_string(),
            advisory_max_active: 0,
            advisory_max_cpu_ms: 0,
        }
    }

    #[test]
    fn parses_the_launch_request_after_the_separator() {
        match parse(&[
            "launch",
            "--",
            "C:\\Windows\\System32\\ping.exe",
            "-n",
            "4",
            "127.0.0.1",
        ]) {
            ParseOutcome::Run(args) => {
                assert_eq!(
                    args.command,
                    Command::Launch(launch_spec(
                        "C:\\Windows\\System32\\ping.exe",
                        "-n 4 127.0.0.1"
                    ))
                );
            }
            other => panic!("launch request must parse: {other:?}"),
        }
        // A request with no arguments.
        assert_eq!(
            parse(&["launch", "--", "C:\\tools\\app.exe"]),
            ParseOutcome::Run(CliArgs {
                command: Command::Launch(launch_spec("C:\\tools\\app.exe", "")),
                ..default_args(Command::Info)
            })
        );
        // Flags may precede the separator; thresholds fold into the spec.
        assert_eq!(
            parse(&[
                "launch",
                "--json",
                "--interval-ms",
                "250",
                "--stop-after-ms",
                "5000",
                "--advisory-max-active",
                "3",
                "--advisory-max-cpu-ms",
                "1500",
                "--",
                "C:\\tools\\app.exe",
            ]),
            ParseOutcome::Run(CliArgs {
                command: Command::Launch(LaunchSpec {
                    advisory_max_active: 3,
                    advisory_max_cpu_ms: 1_500,
                    ..launch_spec("C:\\tools\\app.exe", "")
                }),
                json: true,
                interval_ms: Some(250),
                stop_after_ms: Some(5_000),
                dry_run: false,
                trace: None,
                config_path: None,
            })
        );
    }

    #[test]
    fn policies_trace_flag_parses_and_is_policies_only() {
        match parse(&["policies", "--dry-run", "--trace", "traces\\x.json"]) {
            ParseOutcome::Run(args) => {
                assert_eq!(
                    args.trace,
                    Some(PathBuf::from("traces\\x.json")),
                    "the trace path is carried"
                );
            }
            other => panic!("--trace must parse: {other:?}"),
        }
        // Flag order does not matter.
        assert!(matches!(
            parse(&["--trace", "t.json", "policies", "--dry-run", "--json"]),
            ParseOutcome::Run(_)
        ));
        // Only policies accepts it.
        assert!(matches!(
            parse(&["--trace", "t.json", "info"]),
            ParseOutcome::UsageError(ref m) if m.contains("only valid for policies")
        ));
        assert!(matches!(
            parse(&["low-memory-watch", "--trace", "t.json"]),
            ParseOutcome::UsageError(ref m) if m.contains("only valid for policies")
        ));
        assert!(matches!(
            parse(&["policies", "--dry-run", "--trace"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["policies", "--trace", "t.json"]),
            ParseOutcome::UsageError(ref m) if m.contains("--dry-run")
        ));
    }

    #[test]
    fn help_documents_the_trace_option() {
        assert!(HELP.contains("--trace"));
        assert!(HELP.contains("ramshaper.policy-trace.v1"));
    }

    #[test]
    fn launch_request_rules() {
        // No request at all.
        assert!(matches!(
            parse(&["launch"]),
            ParseOutcome::UsageError(ref m) if m.contains("requires a request")
        ));
        // A bare `--` with nothing after it.
        assert!(matches!(
            parse(&["launch", "--"]),
            ParseOutcome::UsageError(ref m) if m.contains("requires a request")
        ));
        // Positionals before `--` are refused (the request lives after it).
        assert!(matches!(
            parse(&["launch", "C:\\tools\\app.exe"]),
            ParseOutcome::UsageError(ref m) if m.contains("after `--`")
        ));
        // The separator is launch-only.
        assert!(matches!(
            parse(&["info", "--"]),
            ParseOutcome::UsageError(ref m) if m.contains("only valid for launch")
        ));
        assert!(matches!(
            parse(&["--", "C:\\x.exe"]),
            ParseOutcome::UsageError(ref m) if m.contains("only valid for launch")
        ));
        // A non-UTF-8 argument is an explicit usage error, never silent
        // lossy conversion.
        let mut raw: Vec<OsString> = vec![OsString::from("launch"), OsString::from("--")];
        raw.push(OsString::from("C:\\tools\\app.exe"));
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt as _;
            raw.push(OsString::from_wide(&[0xD800])); // lone surrogate
            assert!(matches!(
                parse_args(raw),
                ParseOutcome::UsageError(ref m) if m.contains("not valid UTF-8")
            ));
        }
    }

    #[test]
    fn advisory_flags_are_launch_only_and_bounded() {
        assert!(matches!(
            parse(&["--advisory-max-active", "3", "info"]),
            ParseOutcome::UsageError(ref m) if m.contains("only valid for launch")
        ));
        assert!(matches!(
            parse(&["--advisory-max-cpu-ms", "10", "low-memory-watch"]),
            ParseOutcome::UsageError(ref m) if m.contains("only valid for launch")
        ));
        assert!(matches!(
            parse(&["launch", "--advisory-max-active"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["launch", "--advisory-max-active", "x", "--", "a.exe"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&[
                "launch",
                "--advisory-max-cpu-ms",
                "99999999999999999999",
                "--",
                "a.exe"
            ]),
            ParseOutcome::UsageError(_)
        ));
        // The boundary itself parses (it converts to i64 100-ns units).
        let boundary = (i64::MAX / 10_000).to_string();
        assert!(matches!(
            parse(&["launch", "--advisory-max-cpu-ms", &boundary, "--", "a.exe"]),
            ParseOutcome::Run(_)
        ));
    }

    #[test]
    fn advisory_cpu_conversion_stays_inside_i64() {
        assert_eq!(advisory_cpu_100ns(0), 0);
        assert_eq!(advisory_cpu_100ns(1_500), 15_000_000);
        // The capped boundary converts exactly (no saturation needed).
        assert_eq!(
            advisory_cpu_100ns(MAX_ADVISORY_CPU_MS),
            9_223_372_036_854_770_000
        );
    }

    #[test]
    fn stop_after_ms_is_valid_for_launch_too() {
        assert!(matches!(
            parse(&["launch", "--stop-after-ms", "5000", "--", "a.exe"]),
            ParseOutcome::Run(_)
        ));
        // Still refused elsewhere.
        assert!(matches!(
            parse(&["processes", "--stop-after-ms", "5000"]),
            ParseOutcome::UsageError(ref m) if m.contains("low-memory-watch and launch")
        ));
    }

    #[test]
    fn restore_parses_its_verb_rules() {
        // Exactly one verb required.
        assert!(matches!(
            parse(&["restore", "--pipe-name", "p"]),
            ParseOutcome::UsageError(ref m) if m.contains("--action")
        ));
        assert!(matches!(
            parse(&["restore", "--pipe-name", "p", "--action", "a", "--disable-all"]),
            ParseOutcome::UsageError(ref m) if m.contains("not both")
        ));
        // Verb flags are restore-only.
        assert!(matches!(
            parse(&["--action", "a", "info"]),
            ParseOutcome::UsageError(ref m) if m.contains("only valid for restore")
        ));
        assert!(matches!(
            parse(&["--disable-all", "info"]),
            ParseOutcome::UsageError(ref m) if m.contains("only valid for restore")
        ));
        assert!(matches!(
            parse(&["--pipe-name", "p", "info"]),
            ParseOutcome::UsageError(ref m) if m.contains("only valid for restore")
        ));
        // The flag value is required.
        assert!(matches!(
            parse(&["restore", "--pipe-name"]),
            ParseOutcome::UsageError(_)
        ));
        assert!(matches!(
            parse(&["restore", "--action"]),
            ParseOutcome::UsageError(_)
        ));
        // The happy parse: both verbs parse; the pipe name defaults to None.
        match parse(&["restore", "--pipe-name", "p", "--action", "act-1"]) {
            ParseOutcome::Run(args) => {
                assert!(!args.json);
            }
            other => panic!("restore must parse: {other:?}"),
        }
        assert!(matches!(
            parse(&["restore", "--disable-all", "--json"]),
            ParseOutcome::Run(_)
        ));
        // --dry-run stays policies-only.
        assert!(matches!(
            parse(&["restore", "--dry-run"]),
            ParseOutcome::UsageError(_)
        ));
    }

    #[test]
    fn help_documents_the_launch_command() {
        assert!(HELP.contains("launch"));
        assert!(HELP.contains("LAUNCH (exactly, alone)"));
        // The semantics sentences are line-wrapped in HELP; assert the
        // load-bearing fragments individually.
        assert!(HELP.contains("the job is NOT a"));
        assert!(HELP.contains("sandbox and enforces nothing"));
        assert!(HELP.contains("NOT detachable"));
        assert!(HELP.contains("--advisory-max-active"));
        assert!(HELP.contains("--advisory-max-cpu-ms"));
    }
}
