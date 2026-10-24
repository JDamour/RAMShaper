//! `ramshaper-agent` — session agent binary (RS-030).
//!
//! `ramshaper-agent serve` runs the read-only monitor behind a
//! session-scoped named pipe until Ctrl+C. `ramshaper-agent probe` is a
//! protocol client used for evidence and by the future UI as a reference.
//! The agent performs no process mutation and contains no network code:
//! the named pipe is a local machine object with an explicit user-only
//! DACL (contract 14 §7).

#![deny(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TOOL_NAME: &str = "ramshaper-agent";

const HELP: &str = "\
ramshaper-agent — RAMShaper session agent (read-only, current user)

USAGE:
    ramshaper-agent serve [--pipe-name <NAME>] [--interval-ms <MS>] [--config <PATH>]
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
    --version, -V        Print version
    --help, -h           Print this help

The agent is strictly read-only: no policy commands exist in this build
(ListActions always returns an empty list).
";

#[derive(Debug, PartialEq)]
enum Command {
    Serve {
        pipe_name: Option<String>,
        interval_ms: Option<u64>,
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
        } => {
            #[cfg(windows)]
            return serve_command(pipe_name, interval_ms, config);
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
    config: ramshaper_core::CoreConfig,
) -> ExitCode {
    use ramshaper_core::watch::WaitHandle as _;

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
    match ramshaper_agent::server::serve(Arc::clone(&stop), name, watch_config) {
        Ok(handle) => {
            eprintln!("agent {} listening; collector running", handle.instance_id);
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
                    interval_ms: None
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
