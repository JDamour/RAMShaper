//! `ramshaper-agent` — session agent binary (RS-030).
//!
//! `ramshaper-agent serve` runs the read-only monitor behind a
//! session-scoped named pipe until Ctrl+C. `ramshaper-agent probe` is a
//! protocol client used for evidence and by the future UI as a reference.
//! Without `--policy-journal` the agent performs no process mutation and
//! contains no network code: the named pipe is a local machine object with
//! an explicit user-only DACL (contract 14 §7). With it, the consent-gated
//! policy stage adds exactly one journaled, automatically-restored
//! confirmed action (Normal → Below Normal) — never anything else.

#![deny(unsafe_code)]

use std::io::{BufRead as _, Read as _};
use std::process::ExitCode;
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TOOL_NAME: &str = "ramshaper-agent";

const HELP: &str = "\
ramshaper-agent — RAMShaper session agent (read-only monitor + consent-gated policy stage)

USAGE:
    ramshaper-agent serve [--pipe-name <NAME>] [--interval-ms <MS>] [--config <PATH>]
                          [--policy-journal <PATH>]
    ramshaper-agent probe [--pipe-name <NAME>] [--subscribe <SECONDS>] [--request <CMD>]

COMMANDS:
    serve    Run the agent until Ctrl+C (collector + named-pipe server)
    probe    Connect as a protocol client: capabilities, one snapshot, and
             optionally a timed subscription stream

OPTIONS:
    --pipe-name <NAME>   Pipe name override (default: derived from the
                         logon session: \\\\.\\pipe\\ramshaper-agent-s<sid>-v1)
    --interval-ms <MS>   Collector cadence for serve (default 1000)
    --subscribe <SECS>   probe: stream snapshots for this many seconds
    --request <CMD>      probe: send this command instead of GetSnapshot
    --config <PATH>      TOML configuration (schema_version 1, observe-only)
    --policy-journal <PATH>
                         serve: open the policy gate over this SQLite action
                         journal (no default location). The gate starts
                         DISABLED; startup recovery reconciles outstanding
                         records before the pipe accepts; the operator must
                         type ENABLE on the agent console to allow actions
                         for THIS session (end of input is a decline)
    --version, -V        Print version
    --help, -h           Print this help

POLICY STAGE:
    Without --policy-journal the agent is strictly read-only: policy-stage
    commands answer Unsupported and ListActions reports no actions. With it
    the pipe serves ListActions, PreviewAction, ConfirmAction, RestoreAction
    and DisableActions, but previews and confirms stay refused until the
    exact word ENABLE is typed on the agent console (a per-session consent:
    never a flag, a profile, a reconnect or an IPC command). Enable requires
    the action journal to be readable — recovery must have read and
    processed it first; DisableActions always revokes new-action authority
    and clears pending previews, then restores every outstanding record, and
    a journal read failure is reported as such, never as an empty result.
";

#[derive(Debug, PartialEq)]
enum Command {
    Serve {
        pipe_name: Option<String>,
        interval_ms: Option<u64>,
        policy_journal: Option<std::path::PathBuf>,
    },
    Probe {
        pipe_name: Option<String>,
        subscribe_secs: Option<u64>,
        request: Option<String>,
    },
}

#[derive(Debug, PartialEq)]
enum Parsed {
    Run(Command, Option<std::path::PathBuf>),
    Version,
    Help,
    UsageError(String),
}

fn parse(args: Vec<std::ffi::OsString>) -> Parsed {
    let mut iter = args.into_iter();
    let mut command: Option<&'static str> = None;
    let mut pipe_name: Option<String> = None;
    let mut interval_ms: Option<u64> = None;
    let mut subscribe_secs: Option<u64> = None;
    let mut request: Option<String> = None;
    let mut config: Option<std::path::PathBuf> = None;
    let mut policy_journal: Option<std::path::PathBuf> = None;

    while let Some(arg) = iter.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "--version" | "-V" => return Parsed::Version,
            "--help" | "-h" => return Parsed::Help,
            "--json" => {} // accepted for symmetry, text/JSON is per-frame anyway
            "--pipe-name" => match iter.next() {
                Some(value) => pipe_name = Some(value.to_string_lossy().into_owned()),
                None => return Parsed::UsageError("--pipe-name requires a value".into()),
            },
            "--interval-ms" => match iter.next() {
                Some(value) => match value.to_string_lossy().parse::<u64>() {
                    Ok(ms) if (100..=3_600_000).contains(&ms) => interval_ms = Some(ms),
                    _ => return Parsed::UsageError("--interval-ms must be 100..=3600000".into()),
                },
                None => return Parsed::UsageError("--interval-ms requires a value".into()),
            },
            "--subscribe" => match iter.next() {
                Some(value) => match value.to_string_lossy().parse::<u64>() {
                    Ok(secs) if (1..=3_600).contains(&secs) => subscribe_secs = Some(secs),
                    _ => return Parsed::UsageError("--subscribe must be 1..=3600 seconds".into()),
                },
                None => return Parsed::UsageError("--subscribe requires a value".into()),
            },
            "--request" => match iter.next() {
                Some(value) => request = Some(value.to_string_lossy().into_owned()),
                None => return Parsed::UsageError("--request requires a value".into()),
            },
            "--config" => match iter.next() {
                Some(value) => config = Some(std::path::PathBuf::from(value)),
                None => return Parsed::UsageError("--config requires a path".into()),
            },
            "--policy-journal" => match iter.next() {
                Some(value) => policy_journal = Some(std::path::PathBuf::from(value)),
                None => return Parsed::UsageError("--policy-journal requires a path".into()),
            },
            "serve" if command.is_none() => command = Some("serve"),
            "probe" if command.is_none() => command = Some("probe"),
            other => {
                return Parsed::UsageError(if command.is_none() {
                    format!("unknown command '{other}'")
                } else {
                    format!("unexpected argument '{other}'")
                })
            }
        }
    }

    let Some(command) = command else {
        return Parsed::UsageError("missing command (serve or probe)".into());
    };
    match command {
        "serve" => Parsed::Run(
            Command::Serve {
                pipe_name,
                interval_ms,
                policy_journal,
            },
            config,
        ),
        _ => Parsed::Run(
            Command::Probe {
                pipe_name,
                subscribe_secs,
                request,
            },
            config,
        ),
    }
}

/// Normalize a user-supplied pipe name: bare names are prefixed with the
/// local pipe device; full paths pass through.
fn normalize_pipe_name(name: &str) -> String {
    // "\\.\pipe\" (regular string literal; raw strings cannot end in a
    // backslash).
    const PREFIX: &str = "\\\\.\\pipe\\";
    if name.starts_with(PREFIX) {
        name.to_string()
    } else {
        format!("{PREFIX}{name}")
    }
}

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

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match parse(args) {
        Parsed::Version => {
            println!("{TOOL_NAME} {VERSION}");
            ExitCode::SUCCESS
        }
        Parsed::Help => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Parsed::UsageError(message) => {
            eprintln!("error: {message}");
            eprintln!("run '{TOOL_NAME} --help' for details");
            ExitCode::from(2)
        }
        Parsed::Run(command, config_path) => run(command, config_path),
    }
}

fn run(command: Command, config_path: Option<std::path::PathBuf>) -> ExitCode {
    let config = match load_config(config_path.as_ref()) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };
    match command {
        Command::Serve {
            pipe_name,
            interval_ms,
            policy_journal,
        } => {
            #[cfg(windows)]
            return serve_command(pipe_name, interval_ms, policy_journal, config);
            #[cfg(not(windows))]
            {
                let _ = (pipe_name, interval_ms, config);
                eprintln!("error: ramshaper-agent requires Windows");
                ExitCode::from(1)
            }
        }
        Command::Probe {
            pipe_name,
            subscribe_secs,
            request,
        } => {
            #[cfg(windows)]
            return probe_command(pipe_name, subscribe_secs, request, config);
            #[cfg(not(windows))]
            {
                let _ = (pipe_name, subscribe_secs, request, config);
                eprintln!("error: ramshaper-agent requires Windows");
                ExitCode::from(1)
            }
        }
    }
}

#[cfg(windows)]
fn serve_command(
    pipe_name: Option<String>,
    interval_ms: Option<u64>,
    policy_journal: Option<std::path::PathBuf>,
    config: ramshaper_core::CoreConfig,
) -> ExitCode {
    use ramshaper_core::watch::WaitHandle as _;

    // The policy stage is opt-in by explicit journal path: no path, no
    // gate — the agent stays the read-only observer. The gate starts
    // DISABLED (contract 14 §5: startup is Observe Only); enabling is the
    // typed console consent below, never a flag, file or IPC command.
    let policy = match policy_journal {
        None => None,
        Some(path) => {
            match ramshaper_agent::consent::open_session_gate(
                &path,
                config.exclusions.processes.clone(),
            ) {
                Ok(gate) => Some(std::sync::Arc::new(gate)),
                Err(error) => {
                    eprintln!(
                        "error: cannot open the policy journal at {}: {error}",
                        path.display()
                    );
                    return ExitCode::from(1);
                }
            }
        }
    };

    let name = match pipe_name.or_else(|| config.agent.pipe_name.clone()) {
        Some(name) => normalize_pipe_name(&name),
        None => match ramshaper_agent::server::default_pipe_name() {
            Ok(name) => name,
            Err(f) => {
                eprintln!("error: cannot derive default pipe name: {f}");
                return ExitCode::from(1);
            }
        },
    };
    let cadence = interval_ms.unwrap_or(config.watch.interval_ms);
    let Ok(watch_config) = ramshaper_core::watch::WatchConfig::new(cadence) else {
        eprintln!("error: invalid interval {cadence} ms");
        return ExitCode::from(2);
    };

    let stop = Arc::new(match ramshaper_windows::event::ManualResetEvent::new() {
        Ok(event) => event,
        Err(f) => {
            eprintln!("error: cannot create stop event: {f}");
            return ExitCode::from(1);
        }
    });

    // Ctrl+C cancels the agent: the console handler triggers the stop event,
    // which wakes the accept loop and the collector.
    let cancel = match ramshaper_windows::console::ConsoleCancel::install() {
        Ok(cancel) => Arc::new(cancel),
        Err(f) => {
            eprintln!("error: cannot install Ctrl+C handler: {f}");
            return ExitCode::from(1);
        }
    };
    let stop_for_console = Arc::clone(&stop);
    std::thread::spawn(move || loop {
        if cancel.wait(u32::MAX) == ramshaper_core::watch::WaitOutcome::Signaled {
            stop_for_console.trigger();
            return;
        }
    });

    eprintln!("serving {TOOL_NAME} {VERSION} on {name} (interval {cadence} ms); Ctrl+C to stop");
    let config_summary = ramshaper_agent::dispatch::ConfigSummary::from_config(&config);
    match ramshaper_agent::server::serve(
        Arc::clone(&stop),
        name,
        watch_config,
        config_summary,
        policy.clone(),
    ) {
        Ok(handle) => {
            eprintln!("agent {} listening; collector running", handle.instance_id);
            // The consent act (contract 14 §5): the operator types the
            // exact word on the agent console. Nothing else — not a flag,
            // a profile, a reconnect or an IPC command — may enable the
            // session. End of input is a decline, never a guess.
            if let Some(gate) = &policy {
                eprintln!(
                    "policy stage present (journal-backed, restore loop active); the session is Observe Only"
                );
                eprintln!(
                    "type {} on this console to allow confirmed CPU-priority actions for THIS session (anything else keeps Observe Only)",
                    ramshaper_agent::consent::CONSENT_WORD,
                );
                let mut line = String::new();
                // Bounded read: the consent word is 6 bytes, so a hostile
                // or accidental oversized stdin line must not grow memory
                // without limit (the bounded-memory rule; anything past
                // 4 KiB is discarded unread).
                let read = std::io::stdin().lock().take(4_096).read_line(&mut line);
                let consented = match read {
                    Ok(0) => ramshaper_agent::consent::decide(None),
                    Ok(_) => ramshaper_agent::consent::decide(Some(&line)),
                    Err(_) => false,
                };
                if consented {
                    match gate.enable() {
                        Ok(outcome) => eprintln!(
                            "policy session enabled (this session only; {} record(s) reconciled \
                             by the pre-enable recovery pass; DisableActions or stopping the \
                             agent restores everything outstanding)",
                            outcome.reconciled
                        ),
                        Err(reason) => {
                            eprintln!("policy stays Observe Only despite the consent: {reason}")
                        }
                    }
                } else {
                    eprintln!("no consent given; the session stays Observe Only");
                }
            }
            // serve() returns immediately; block until stop, then join the
            // accept loop and collector for an orderly shutdown.
            wait_for_stop(&stop);
            if let Ok(slot) = handle.accept_error.lock() {
                if let Some(message) = slot.as_ref() {
                    eprintln!("warning: accept loop ended early: {message}");
                }
            }
            handle.stop();
            eprintln!("agent stopped; handles released");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
    }
}

/// Block until the stop event fires (Ctrl+C or programmatic).
#[cfg(windows)]
fn wait_for_stop(stop: &Arc<ramshaper_windows::event::ManualResetEvent>) {
    use ramshaper_core::watch::WaitHandle as _;
    loop {
        if stop.wait(u32::MAX) == ramshaper_core::watch::WaitOutcome::Signaled {
            return;
        }
    }
}

#[cfg(windows)]
fn probe_command(
    pipe_name: Option<String>,
    subscribe_secs: Option<u64>,
    request: Option<String>,
    config: ramshaper_core::CoreConfig,
) -> ExitCode {
    use ramshaper_core::ipc::RequestEnvelope;

    let name = match pipe_name.or_else(|| config.agent.pipe_name.clone()) {
        Some(name) => normalize_pipe_name(&name),
        None => match ramshaper_agent::server::default_pipe_name() {
            Ok(name) => name,
            Err(f) => {
                eprintln!("error: cannot derive default pipe name: {f}");
                return ExitCode::from(1);
            }
        },
    };

    let client = match ramshaper_windows::pipe::client_connect(&name) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("error: cannot connect to {name}: {e}");
            return ExitCode::from(1);
        }
    };

    let send = |command: &str| -> Option<serde_json::Value> {
        let envelope = RequestEnvelope {
            protocol_version: ramshaper_core::ipc::PROTOCOL_VERSION,
            request_id: format!("probe-{command}"),
            agent_instance_id: None,
            command: command.to_string(),
            payload: None,
        };
        let payload = serde_json::to_vec(&envelope).ok()?;
        if ramshaper_windows::pipe::write_frame(&client, &payload).is_err() {
            eprintln!("error: write failed");
            return None;
        }
        let response = ramshaper_windows::pipe::read_frame(&client).ok()?;
        serde_json::from_slice(&response).ok()
    };

    // Capabilities first (proves the connection and the protocol).
    match send("GetCapabilities") {
        Some(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            );
        }
        None => {
            eprintln!("error: no response to GetCapabilities");
            return ExitCode::from(1);
        }
    }

    let requested = request.as_deref().unwrap_or("GetSnapshot");
    if let Some(value) = send(requested) {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
    } else {
        eprintln!("error: no response to {requested}");
        return ExitCode::from(1);
    }

    if let Some(seconds) = subscribe_secs {
        // Subscribe and stream for the requested duration.
        let envelope = RequestEnvelope {
            protocol_version: ramshaper_core::ipc::PROTOCOL_VERSION,
            request_id: "probe-subscribe".to_string(),
            agent_instance_id: None,
            command: "SubscribeSnapshots".to_string(),
            payload: None,
        };
        if let Ok(payload) = serde_json::to_vec(&envelope) {
            if ramshaper_windows::pipe::write_frame(&client, &payload).is_err() {
                eprintln!("error: subscribe write failed");
                return ExitCode::from(1);
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
            let mut deliveries = 0u64;
            while std::time::Instant::now() < deadline {
                match ramshaper_windows::pipe::read_frame(&client) {
                    Ok(frame) => {
                        deliveries += 1;
                        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&frame) {
                            let snapshot = &value["data"]["snapshot"];
                            println!("{}", serde_json::to_string(&snapshot).unwrap_or_default());
                        }
                    }
                    Err(e) => {
                        eprintln!("stream ended: {e}");
                        break;
                    }
                }
            }
            eprintln!("received {deliveries} snapshot delivery(ies) in {seconds}s");
        }
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_args(list: &[&str]) -> Vec<std::ffi::OsString> {
        list.iter().map(std::ffi::OsString::from).collect()
    }

    #[test]
    fn parses_serve_and_probe() {
        assert_eq!(
            parse(os_args(&["serve"])),
            Parsed::Run(
                Command::Serve {
                    pipe_name: None,
                    interval_ms: None,
                    policy_journal: None,
                },
                None
            )
        );
        assert_eq!(
            parse(os_args(&[
                "serve",
                "--interval-ms",
                "500",
                "--pipe-name",
                r"\\.\pipe\x"
            ])),
            Parsed::Run(
                Command::Serve {
                    pipe_name: Some(r"\\.\pipe\x".to_string()),
                    interval_ms: Some(500),
                    policy_journal: None,
                },
                None
            )
        );
        assert_eq!(
            parse(os_args(&[
                "probe",
                "--subscribe",
                "3",
                "--request",
                "ListActions"
            ])),
            Parsed::Run(
                Command::Probe {
                    pipe_name: None,
                    subscribe_secs: Some(3),
                    request: Some("ListActions".to_string()),
                },
                None
            )
        );
    }

    #[test]
    fn usage_errors() {
        assert!(matches!(parse(os_args(&[])), Parsed::UsageError(_)));
        assert!(matches!(parse(os_args(&["bogus"])), Parsed::UsageError(_)));
        assert!(matches!(
            parse(os_args(&["serve", "--interval-ms", "5"])),
            Parsed::UsageError(_)
        ));
        assert!(matches!(
            parse(os_args(&["probe", "--subscribe", "0"])),
            Parsed::UsageError(_)
        ));
        assert!(matches!(
            parse(os_args(&["--pipe-name"])),
            Parsed::UsageError(_)
        ));
        assert!(matches!(
            parse(os_args(&["serve", "--policy-journal"])),
            Parsed::UsageError(_)
        ));
    }

    #[test]
    fn policy_journal_flag_selects_the_policy_stage() {
        // The explicit journal path is the only way to start with the
        // policy gate; there is no default location and no bare --policy
        // flag (contract 14 §5: nothing but the typed console consent
        // enables the session).
        assert_eq!(
            parse(os_args(&[
                "serve",
                "--policy-journal",
                r"C:\data\policy.sqlite3"
            ])),
            Parsed::Run(
                Command::Serve {
                    pipe_name: None,
                    interval_ms: None,
                    policy_journal: Some(std::path::PathBuf::from(r"C:\data\policy.sqlite3")),
                },
                None
            )
        );
        assert_eq!(
            parse(os_args(&["serve"])),
            Parsed::Run(
                Command::Serve {
                    pipe_name: None,
                    interval_ms: None,
                    policy_journal: None,
                },
                None
            )
        );
    }

    #[test]
    fn pipe_names_are_normalized() {
        assert_eq!(normalize_pipe_name("my-pipe"), r"\\.\pipe\my-pipe");
        assert_eq!(
            normalize_pipe_name(r"\\.\pipe\my-pipe"),
            r"\\.\pipe\my-pipe"
        );
    }

    #[test]
    fn version_and_help() {
        assert_eq!(parse(os_args(&["--version"])), Parsed::Version);
        assert_eq!(parse(os_args(&["-h"])), Parsed::Help);
    }
}
