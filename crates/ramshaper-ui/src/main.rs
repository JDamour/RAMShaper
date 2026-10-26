//! `ramshaper-ui` — native UI feasibility shell (RS-010 / R-02 spike).
//!
//! An Overview-window prototype per the UI spec: it renders live data from
//! the session agent over protocol v1 (named pipe), with explicit
//! connecting/disconnected/stale states, accessibility annotations and
//! keyboard focus. No policy controls exist in this build (Observe Only).
//!
//! Transport: by default the client subscribes and receives pushed
//! snapshots; `--poll-ms <MS>` instead polls the agent with one-shot
//! `GetSnapshot` probes (the `ramshaper-agent probe` request pattern) every
//! MS milliseconds. State transitions are logged to stderr so connection
//! and restart behavior can be captured as evidence.
//!
//! Licensing note (R-02/ADR-012): this prototype links the `slint` crate,
//! which is GPL-3.0-only OR Slint Royalty-free-2.0 OR commercial licensed.
//! It is an internal feasibility artifact; no licensing decision is made
//! here and nothing is distributed.

#![deny(unsafe_code)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ramshaper_core::ipc::{
    RequestEnvelope, COMMAND_GET_CAPABILITIES, COMMAND_GET_SNAPSHOT, COMMAND_SUBSCRIBE_SNAPSHOTS,
    PROTOCOL_VERSION,
};
use ramshaper_core::units::format_bytes;

// Slint's generated accessors use unwrap internally; they are machine
// generated in OUT_DIR, so the workspace unwrap ban is lifted for exactly
// this module (not for hand-written code).
#[allow(clippy::unwrap_used)]
mod generated {
    slint::include_modules!();
}
use generated::*;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TOOL_NAME: &str = "ramshaper-ui";

const HELP: &str = "\
ramshaper-ui — RAMShaper native UI (feasibility spike: Overview + Applications)

USAGE:
    ramshaper-ui [--pipe-name <NAME>] [--poll-ms <MS>] [--page <NAME>]

Connects to the session agent (start 'ramshaper-agent serve' first; the
default pipe name is derived from your logon session). Read-only: Observe
Only, no policy controls in this build. State transitions (connect,
disconnect, reconnect, probe errors) are logged to stderr. The Applications
page polls the agent's GetProcesses on demand while visible.

OPTIONS:
    --pipe-name <NAME>   Pipe name override (default: derived from the
                         logon session)
    --poll-ms <MS>       Poll with one-shot GetSnapshot probes every MS ms
                         instead of subscribing to the snapshot stream
    --page <NAME>        Start page: 'overview' (default), 'applications',
                         'profiles' or 'actions'
";

/// Last successful update, for the freshness ("Last updated Xs ago") label.
struct LastUpdate(Option<Instant>);

/// Spike event log: one stderr line per connection-state transition, with
/// the elapsed time since process start. This is the restart-handling
/// evidence trail.
fn log_event(t0: Instant, msg: &str) {
    eprintln!("[ui +{:>7.1}s] {msg}", t0.elapsed().as_secs_f64());
}

/// Parse the `--poll-ms` value: a whole number of milliseconds, at least 1.
fn parse_poll_ms(value: &str) -> Result<u64, String> {
    match value.parse::<u64>() {
        Ok(0) => Err("--poll-ms must be at least 1".to_string()),
        Ok(ms) => Ok(ms),
        Err(_) => Err("--poll-ms requires a number of milliseconds".to_string()),
    }
}

fn main() {
    let t0 = Instant::now();
    let mut pipe_name: Option<String> = None;
    let mut poll_ms: Option<u64> = None;
    let mut initial_page: Option<String> = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--help" | "-h" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("{TOOL_NAME} {VERSION}");
                std::process::exit(0);
            }
            "--pipe-name" => match args.next() {
                Some(value) => pipe_name = Some(normalize_pipe_name(&value.to_string_lossy())),
                None => {
                    eprintln!("error: --pipe-name requires a value");
                    std::process::exit(2);
                }
            },
            "--page" => match args.next() {
                Some(value) => {
                    let page = value.to_string_lossy().to_string();
                    if page != "overview"
                        && page != "applications"
                        && page != "profiles"
                        && page != "actions"
                    {
                        eprintln!(
                            "error: --page must be 'overview', 'applications', 'profiles' or 'actions'"
                        );
                        std::process::exit(2);
                    }
                    initial_page = Some(page);
                }
                None => {
                    eprintln!("error: --page requires a value");
                    std::process::exit(2);
                }
            },
            "--poll-ms" => match args.next() {
                Some(value) => match parse_poll_ms(&value.to_string_lossy()) {
                    Ok(ms) => poll_ms = Some(ms),
                    Err(message) => {
                        eprintln!("error: {message}");
                        std::process::exit(2);
                    }
                },
                None => {
                    eprintln!("error: --poll-ms requires a value");
                    std::process::exit(2);
                }
            },
            other => {
                eprintln!("error: unexpected argument '{other}'");
                std::process::exit(2);
            }
        }
    }

    let pipe_name = pipe_name.unwrap_or_else(|| {
        #[cfg(windows)]
        {
            ramshaper_agent::server::default_pipe_name()
                .unwrap_or_else(|_| r"\\.\pipe\ramshaper-agent-v1".to_string())
        }
        #[cfg(not(windows))]
        {
            r"\\.\pipe\ramshaper-agent-v1".to_string()
        }
    });

    let mode = match poll_ms {
        Some(ms) => format!("probe-poll every {ms} ms"),
        None => "subscribe stream".to_string(),
    };
    log_event(
        t0,
        &format!("ramshaper-ui {VERSION}; pipe {pipe_name}; mode: {mode}"),
    );

    let app = AppWindow::new().expect("UI window");
    let weak = app.as_weak();
    let last_update = Arc::new(Mutex::new(LastUpdate(None)));
    // Doc 06 hidden-window rule: a minimized window must not drive
    // high-frequency updates. This flag is polled from a 1 s Slint timer
    // (the winit backend syncs the minimized state) and gates the snapshot
    // pushes, the freshness ticker and the Applications fetch.
    let minimized = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Agent client thread: connect, subscribe/probe, push updates into the UI.
    {
        let weak = weak.clone();
        let last_update = Arc::clone(&last_update);
        let minimized = Arc::clone(&minimized);
        let spawn_pipe = pipe_name.clone();
        std::thread::spawn(move || {
            agent_client_loop(spawn_pipe, poll_ms, weak, last_update, minimized, t0)
        });
    }

    // Freshness ticker: "last updated Xs ago" + staleness flag.
    start_freshness_ticker(
        weak.clone(),
        Arc::clone(&last_update),
        Arc::clone(&minimized),
    );

    app.global::<UiState<'_>>().on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    // Applications page (RS-011): navigation, sorting and the poll timer.
    let start_page = initial_page.unwrap_or_else(|| "overview".to_string());
    let page_state = Arc::new(Mutex::new(ProcessPage {
        page: start_page.clone(),
        ..ProcessPage::default()
    }));
    app.global::<UiState<'_>>()
        .set_current_page(start_page.as_str().into());
    let fetch_in_flight = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let config_fetched = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let weak = weak.clone();
        let page_state = Arc::clone(&page_state);
        app.global::<UiState<'_>>().on_navigate(move |page| {
            let page = page.to_string();
            if let Ok(mut guard) = page_state.lock() {
                guard.page = page.clone();
            }
            if let Some(app) = weak.upgrade() {
                app.global::<UiState<'_>>()
                    .set_current_page(page.as_str().into());
            }
        });
    }
    {
        let weak = weak.clone();
        let page_state = Arc::clone(&page_state);
        app.global::<UiState<'_>>().on_sort_by(move |key| {
            if let Ok(mut guard) = page_state.lock() {
                guard.sort_key = key.to_string();
            }
            if let Some(app) = weak.upgrade() {
                app.global::<UiState<'_>>()
                    .set_process_sort(key.as_str().into());
                push_process_rows(&app, &page_state);
            }
        });
    }
    let poll_timer = slint::Timer::default();
    {
        let weak = weak.clone();
        let page_state = Arc::clone(&page_state);
        let in_flight = Arc::clone(&fetch_in_flight);
        let pipe = pipe_name.clone();
        let minimized = Arc::clone(&minimized);
        // One fetch at a time; skips silently while a fetch is running, no
        // fetch is due (Applications page / first config), or the window is
        // minimized. The config is fetched once for the Overview indicator
        // and refreshed while the Profiles page is visible.
        poll_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(2000),
            move || {
                let page = page_state
                    .lock()
                    .map(|guard| guard.page.clone())
                    .unwrap_or_default();
                let want_processes = page == "applications";
                let want_actions = page == "actions";
                let want_config = page == "profiles"
                    || !config_fetched.load(std::sync::atomic::Ordering::Relaxed);
                if (!want_processes && !want_config && !want_actions)
                    || minimized.load(std::sync::atomic::Ordering::Relaxed)
                    || in_flight.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    return;
                }
                let kind = if want_processes {
                    FetchKind::Processes
                } else if want_actions {
                    FetchKind::Actions
                } else {
                    FetchKind::Config
                };
                let weak = weak.clone();
                let page_state = Arc::clone(&page_state);
                let in_flight = Arc::clone(&in_flight);
                let config_fetched = Arc::clone(&config_fetched);
                let pipe = pipe.clone();
                std::thread::spawn(move || {
                    let result = match kind {
                        FetchKind::Processes => fetch_processes(&pipe),
                        FetchKind::Actions => fetch_actions(&pipe),
                        FetchKind::Config => fetch_config(&pipe),
                    };
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = weak.upgrade() {
                            match kind {
                                FetchKind::Processes => {
                                    apply_process_result(&app, &page_state, result);
                                }
                                FetchKind::Actions => {
                                    apply_actions(&app, result);
                                }
                                FetchKind::Config => {
                                    if result.is_ok() {
                                        config_fetched
                                            .store(true, std::sync::atomic::Ordering::Relaxed);
                                    }
                                    apply_config(&app, result);
                                }
                            }
                        }
                        in_flight.store(false, std::sync::atomic::Ordering::SeqCst);
                    });
                });
            },
        );
    }

    // RS-013 controls: restore one action / disable everything. One-shot
    // requests on their own connections; the outcome lands in the note and
    // the list refreshes on the next poll tick.
    {
        let weak = weak.clone();
        let pipe = pipe_name.clone();
        app.global::<UiState<'_>>()
            .on_restore_action(move |action_id| {
                let action_id = action_id.to_string();
                let weak = weak.clone();
                let pipe = pipe.clone();
                std::thread::spawn(move || {
                    let result = run_restore(&pipe, &action_id);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = weak.upgrade() {
                            apply_action_command_result(&app, "Restore", result);
                        }
                    });
                });
            });
    }
    {
        let weak = weak.clone();
        let pipe = pipe_name.clone();
        app.global::<UiState<'_>>().on_disable_actions(move || {
            let weak = weak.clone();
            let pipe = pipe.clone();
            std::thread::spawn(move || {
                let result = run_disable(&pipe);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak.upgrade() {
                        apply_action_command_result(&app, "Disable", result);
                    }
                });
            });
        });
    }

    // Visibility poll on a PLAIN thread: the Slint event loop stops waking
    // while the window is occluded, so a timer on the loop can never see the
    // restore. IsIconic via the audited FFI wrapper is event-loop
    // independent. The flag gates every high-frequency update path (snapshot
    // pushes, freshness ticker, Applications fetch) so a minimized window
    // stops driving UI work (doc 06 hidden-window rule).
    {
        let minimized = Arc::clone(&minimized);
        std::thread::Builder::new()
            .name("ramshaper-visibility".to_string())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(500));
                let Some(hwnd) = ramshaper_windows::window::find_window_by_title("RAMShaper")
                else {
                    // Window gone (closing): leave the flag; the process
                    // exits with the event loop.
                    continue;
                };
                let now = ramshaper_windows::window::is_iconic(hwnd);
                let prev = minimized.swap(now, std::sync::atomic::Ordering::Relaxed);
                if now != prev {
                    if now {
                        log_event(t0, "window minimized: UI updates paused");
                    } else {
                        log_event(t0, "window restored: UI updates resumed");
                    }
                }
            })
            .expect("visibility thread");
    }

    app.run().expect("UI event loop");
    log_event(t0, "UI closed; client stops");
}

fn start_freshness_ticker(
    weak: slint::Weak<AppWindow>,
    last_update: Arc<Mutex<LastUpdate>>,
    minimized: Arc<std::sync::atomic::AtomicBool>,
) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let weak = weak.clone();
        let last_update = Arc::clone(&last_update);
        let minimized = Arc::clone(&minimized);
        let Ok(()) = slint::invoke_from_event_loop(move || {
            // While minimized the label is not seen; skip the update (the
            // age is recomputed from the same timestamp on restore, so the
            // stale flag stays truthful).
            if minimized.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let app = weak.unwrap();
            let ui = app.global::<UiState<'_>>();
            let guard = last_update.lock().expect("last update");
            if let Some(at) = guard.0 {
                let age = at.elapsed().as_secs();
                ui.set_updated_ago(if age == 0 {
                    "just updated".into()
                } else {
                    format!("last updated {age}s ago").into()
                });
                ui.set_stale(age > 5);
            }
        }) else {
            return; // event loop gone: stop ticking
        };
    });
}

/// Bare pipe names are normalized to the local pipe device.
fn normalize_pipe_name(name: &str) -> String {
    const PREFIX: &str = "\\\\.\\pipe\\";
    if name.starts_with(PREFIX) {
        name.to_string()
    } else {
        format!("{PREFIX}{name}")
    }
}

/// Show the connected state, including the serving agent instance id (the
/// instance changes when the agent restarts, which makes the reconnection
/// visible in the window).
fn set_connected(weak: &slint::Weak<AppWindow>, instance: &str) {
    let weak = weak.clone();
    let instance = instance.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        let app = weak.unwrap();
        let ui = app.global::<UiState<'_>>();
        ui.set_connection_state("connected".into());
        ui.set_agent_instance(format!("agent: {instance}").into());
    });
}

/// Push one snapshot into the UI thread. Returns false when the event loop
/// is gone (window closed): the client thread then ends.
fn push_display(
    snapshot: &serde_json::Value,
    weak: &slint::Weak<AppWindow>,
    last_update: &Arc<Mutex<LastUpdate>>,
) -> bool {
    let display = snapshot_to_display(snapshot);
    let weak = weak.clone();
    let last_update = Arc::clone(last_update);
    let Ok(()) = slint::invoke_from_event_loop(move || {
        let app = weak.unwrap();
        let ui = app.global::<UiState<'_>>();
        ui.set_os_label(display.os_label);
        ui.set_pressure_level(display.pressure_level);
        ui.set_pressure_heading(display.pressure_heading);
        ui.set_pressure_evidence(display.pressure_evidence);
        ui.set_pressure_missing(display.pressure_missing);
        ui.set_low_memory_state(display.low_memory_state);
        ui.set_phys_total(display.phys_total);
        ui.set_phys_available(display.phys_available);
        ui.set_phys_in_use(display.phys_in_use);
        ui.set_commit_used(display.commit_used);
        ui.set_commit_limit(display.commit_limit);
        ui.set_commit_headroom(display.commit_headroom);
        ui.set_cta_text(display.cta_text);
        ui.set_healthy(display.healthy);
        ui.set_elevated(display.elevated);
        if let Ok(mut guard) = last_update.lock() {
            guard.0 = Some(Instant::now());
        }
    }) else {
        return false;
    };
    true
}

/// The agent connection loop: keeps retrying while the UI is open, pushes
/// every snapshot delivery into the UI thread, and marks disconnects.
#[cfg(windows)]
fn agent_client_loop(
    pipe_name: String,
    poll_ms: Option<u64>,
    weak: slint::Weak<AppWindow>,
    last_update: Arc<Mutex<LastUpdate>>,
    minimized: Arc<std::sync::atomic::AtomicBool>,
    t0: Instant,
) {
    use ramshaper_windows::pipe::{client_connect, read_frame, write_frame};

    loop {
        let set_disconnected = |message: String| {
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let app = weak.unwrap();
                let ui = app.global::<UiState<'_>>();
                ui.set_connection_state("disconnected".into());
                ui.set_cta_text(message.clone().into());
                ui.set_elevated(false);
                ui.set_healthy(false);
                ui.set_stale(true);
                ui.set_agent_instance("agent: -".into());
            });
        };

        log_event(t0, "connecting…");
        let client = match client_connect(&pipe_name) {
            Ok(client) => client,
            Err(error) => {
                log_event(
                    t0,
                    &format!("agent not reachable ({error}); retrying in 1.5 s"),
                );
                set_disconnected(
                    "Session agent not reachable — start 'ramshaper-agent serve'. Retrying…"
                        .to_string(),
                );
                std::thread::sleep(Duration::from_millis(1500));
                continue;
            }
        };
        log_event(t0, "pipe connected; handshaking GetCapabilities");

        // Handshake: capabilities prove protocol compatibility.
        let caps = RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: "ui-caps".to_string(),
            agent_instance_id: None,
            command: COMMAND_GET_CAPABILITIES.to_string(),
            payload: None,
        };
        let caps_payload = match serde_json::to_vec(&caps) {
            Ok(payload) => payload,
            Err(error) => {
                log_event(t0, &format!("handshake encode failed: {error}"));
                set_disconnected("Session agent connection lost. Retrying…".to_string());
                std::thread::sleep(Duration::from_millis(1500));
                continue;
            }
        };
        if write_frame(&client, &caps_payload).is_err() {
            log_event(t0, "handshake write failed");
            set_disconnected("Session agent connection lost. Retrying…".to_string());
            std::thread::sleep(Duration::from_millis(1500));
            continue;
        }
        let capabilities: serde_json::Value = match read_frame(&client)
            .ok()
            .and_then(|frame| serde_json::from_slice(&frame).ok())
        {
            Some(value) => value,
            None => {
                log_event(t0, "handshake read failed");
                set_disconnected("Session agent closed the connection. Retrying…".to_string());
                std::thread::sleep(Duration::from_millis(1500));
                continue;
            }
        };
        if capabilities["ok"] != true {
            log_event(
                t0,
                &format!(
                    "agent rejected the connection: {}",
                    capabilities["error"]["code"]
                ),
            );
            set_disconnected(format!(
                "Agent rejected the connection: {}",
                capabilities["error"]["code"]
            ));
            std::thread::sleep(Duration::from_millis(1500));
            continue;
        }
        let instance = capabilities["agent_instance_id"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        log_event(t0, &format!("connected: agent instance {instance}"));
        set_connected(&weak, &instance);

        let outcome = match poll_ms {
            Some(ms) => run_probe_poll(&client, ms, &weak, &last_update, &minimized, t0),
            None => run_subscription(&client, &weak, &last_update, &minimized, t0),
        };
        match outcome {
            // UI closed: the thread ends with the event loop.
            Ok(()) => return,
            Err(reason) => {
                log_event(t0, &format!("disconnected: {reason}"));
                set_disconnected("Session agent connection lost. Retrying…".to_string());
            }
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
}

#[cfg(not(windows))]
fn agent_client_loop(
    _pipe_name: String,
    _poll_ms: Option<u64>,
    _weak: slint::Weak<AppWindow>,
    _last_update: Arc<Mutex<LastUpdate>>,
    _minimized: Arc<std::sync::atomic::AtomicBool>,
    _t0: Instant,
) {
}

/// Subscribe mode (default): the agent pushes every snapshot on the watch
/// cadence after one `SubscribeSnapshots` request. Returns when the stream
/// ends (agent gone) or the UI closes. While the window is minimized the
/// stream is still drained (the agent's subscriber queue is bounded and
/// exposes gaps otherwise) but pushes are skipped: a hidden window drives
/// no UI updates (doc 06).
#[cfg(windows)]
fn run_subscription(
    client: &ramshaper_windows::handle::OwnedHandle,
    weak: &slint::Weak<AppWindow>,
    last_update: &Arc<Mutex<LastUpdate>>,
    minimized: &Arc<std::sync::atomic::AtomicBool>,
    t0: Instant,
) -> Result<(), String> {
    use ramshaper_windows::pipe::{read_frame, write_frame};

    let subscribe = RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: "ui-subscribe".to_string(),
        agent_instance_id: None,
        command: COMMAND_SUBSCRIBE_SNAPSHOTS.to_string(),
        payload: None,
    };
    let subscribe_payload =
        serde_json::to_vec(&subscribe).map_err(|error| format!("encode failed: {error}"))?;
    write_frame(client, &subscribe_payload).map_err(|error| format!("write failed: {error}"))?;
    let ack = read_frame(client).map_err(|error| format!("read failed: {error}"))?;
    match serde_json::from_slice::<serde_json::Value>(&ack) {
        Ok(value) if value["data"]["subscribed"] == true => {
            log_event(t0, "subscribed; waiting for pushed snapshots");
        }
        Ok(value) => log_event(t0, &format!("unexpected subscribe ack: {}", value["data"])),
        Err(error) => log_event(t0, &format!("malformed subscribe ack: {error}")),
    }

    loop {
        let frame = match read_frame(client) {
            Ok(frame) => frame,
            Err(error) => return Err(format!("snapshot stream read failed: {error}")),
        };
        // Hidden window: drain the stream, skip the UI update.
        if minimized.load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_slice(&frame) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let snapshot = &value["data"]["snapshot"];
        if snapshot.is_null() {
            continue;
        }
        if !push_display(snapshot, weak, last_update) {
            return Ok(()); // UI closed
        }
    }
}

/// Probe-poll mode (`--poll-ms`): one-shot `GetSnapshot` request/response
/// every `interval_ms` milliseconds on the same connection — the
/// `ramshaper-agent probe` request pattern. Returns when the connection
/// fails (agent gone) or the UI closes. While the window is minimized no
/// probe is sent at all; the next probe fires immediately on restore.
#[cfg(windows)]
fn run_probe_poll(
    client: &ramshaper_windows::handle::OwnedHandle,
    interval_ms: u64,
    weak: &slint::Weak<AppWindow>,
    last_update: &Arc<Mutex<LastUpdate>>,
    minimized: &Arc<std::sync::atomic::AtomicBool>,
    t0: Instant,
) -> Result<(), String> {
    use ramshaper_windows::pipe::{read_frame, write_frame};

    let interval = Duration::from_millis(interval_ms);
    log_event(
        t0,
        &format!("probe-polling GetSnapshot every {interval_ms} ms"),
    );
    let mut probe_seq: u64 = 0;
    loop {
        // Hidden window: no probes, no pushes (doc 06 hidden-window rule).
        // A short sleep keeps the restore reaction quick.
        if minimized.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        let tick_started = Instant::now();
        probe_seq += 1;
        let envelope = RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: format!("ui-probe-{probe_seq}"),
            agent_instance_id: None,
            command: COMMAND_GET_SNAPSHOT.to_string(),
            payload: None,
        };
        let payload =
            serde_json::to_vec(&envelope).map_err(|error| format!("encode failed: {error}"))?;
        write_frame(client, &payload).map_err(|error| format!("write failed: {error}"))?;
        let frame = read_frame(client).map_err(|error| format!("probe read failed: {error}"))?;
        let value: serde_json::Value = match serde_json::from_slice(&frame) {
            Ok(value) => value,
            Err(error) => {
                log_event(t0, &format!("malformed probe response: {error}"));
                continue;
            }
        };
        if value["ok"] == true {
            if let Some(snapshot) = value.get("data") {
                if snapshot.is_object() && !push_display(snapshot, weak, last_update) {
                    return Ok(()); // UI closed
                }
            }
        } else {
            // E.g. StaleSnapshot in the first moments after the agent starts:
            // the connection is fine, the agent just has no fresh sample yet.
            let code = value["error"]["code"].as_str().unwrap_or("unknown");
            log_event(t0, &format!("probe rejected: {code}; still connected"));
        }
        let spent = tick_started.elapsed();
        if spent < interval {
            std::thread::sleep(interval - spent);
        }
    }
}

/// State of the Applications page (RS-011), shared between the UI thread
/// (sort callbacks, model push) and the poll timer.
#[derive(Default)]
struct ProcessPage {
    /// Which page is visible ("overview" | "applications").
    page: String,
    /// Current sort key ("working-set" | "private" | "cpu" | "name").
    sort_key: String,
    /// Parsed rows of the last successful fetch (numeric sort keys intact).
    rows: Vec<RowData>,
    /// The last fetch's rate window, shown in the note.
    rate_interval_ms: u64,
    /// The agent's row-cap note, if it capped the served rows.
    row_cap_note: Option<String>,
}

/// One parsed application row: numeric values for sorting; formatting to
/// display strings happens at the Slint boundary.
#[derive(Debug, Clone, PartialEq)]
struct RowData {
    pid: u32,
    name: String,
    working_set: Option<u64>,
    /// Short bracketed note for the working-set cell ("" when ok).
    working_set_note: &'static str,
    private_commit: Option<u64>,
    cpu_percent: Option<f64>,
    cpu_note: &'static str,
    io_read_rate: Option<u64>,
    io_read_note: &'static str,
    io_write_rate: Option<u64>,
    io_write_note: &'static str,
}

/// Status string → short bracketed note; `ok` carries no note.
fn status_note(status: &str) -> &'static str {
    match status {
        "ok" => "",
        "warming_up" => "[warming]",
        "access_denied" => "[denied]",
        "unsupported" => "[unsup]",
        "stale" => "[stale]",
        s if s.starts_with("exited") => "[exited]",
        _ => "[error]",
    }
}

/// Parse the agent's `GetProcesses` inventory payload into sortable rows
/// (pure; synthetic-fixture tested). Unknown or missing fields stay
/// unavailable — never zero.
fn parse_process_rows(inventory: &serde_json::Value) -> Vec<RowData> {
    inventory["processes"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    let byte_metric = |key: &str| -> Option<u64> { row[key]["value"].as_u64() };
                    // Rate metrics (cpu, io) carry `rate` only when OK and
                    // defined; any other status becomes a bracketed note.
                    let rate_metric = |key: &str| -> (Option<f64>, &'static str) {
                        let envelope = &row[key];
                        let status = envelope["status"].as_str().unwrap_or("error");
                        match envelope["rate"].as_f64() {
                            Some(rate) => (Some(rate), ""),
                            None => (None, status_note(status)),
                        }
                    };
                    let (cpu_percent, cpu_note) = rate_metric("cpu_percent");
                    let (io_read_f, io_read_note) = rate_metric("io_read");
                    let (io_write_f, io_write_note) = rate_metric("io_write");
                    RowData {
                        pid: row["identity"]["pid"].as_u64().unwrap_or(0) as u32,
                        name: row["name"]
                            .as_str()
                            .unwrap_or("unknown")
                            .chars()
                            .take(28)
                            .collect(),
                        working_set: byte_metric("working_set"),
                        working_set_note: status_note(
                            row["working_set"]["status"].as_str().unwrap_or("error"),
                        ),
                        private_commit: byte_metric("private_commit"),
                        cpu_percent,
                        cpu_note,
                        io_read_rate: io_read_f.map(|r| r as u64),
                        io_read_note,
                        io_write_rate: io_write_f.map(|r| r as u64),
                        io_write_note,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Sort rows in place. Numeric keys sort descending with unavailable values
/// sinking to the end; the name key sorts ascending, case-insensitively.
fn sort_process_rows(rows: &mut [RowData], key: &str) {
    match key {
        "private" => rows.sort_by(|a, b| b.private_commit.cmp(&a.private_commit)),
        "cpu" => rows.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "name" => rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        // Default and unknown keys: working set.
        _ => rows.sort_by(|a, b| b.working_set.cmp(&a.working_set)),
    }
}

fn sort_label(key: &str) -> &'static str {
    match key {
        "private" => "private commit",
        "cpu" => "cpu",
        "name" => "name",
        _ => "working set",
    }
}

/// Convert parsed rows into Slint model rows with units and honest
/// unavailable notes.
fn to_model_rows(rows: &[RowData]) -> Vec<ProcessRow> {
    use ramshaper_core::units::format_bytes;
    rows.iter()
        .map(|row| {
            let cell = |value: Option<u64>| -> String {
                value.map_or_else(|| "[unavailable]".to_string(), format_bytes)
            };
            let working_set = match row.working_set {
                Some(value) => format_bytes(value),
                None => "[unavailable]".to_string(),
            };
            let cpu = match row.cpu_percent {
                Some(rate) => format!("{rate:.2}"),
                None => row.cpu_note.to_string(),
            };
            let io_read = match row.io_read_rate {
                Some(rate) => format_bytes(rate),
                None => row.io_read_note.to_string(),
            };
            let io_write = match row.io_write_rate {
                Some(rate) => format_bytes(rate),
                None => row.io_write_note.to_string(),
            };
            ProcessRow {
                pid: row.pid as i32,
                name: row.name.clone().into(),
                working_set: working_set.clone().into(),
                private: cell(row.private_commit).into(),
                cpu: cpu.clone().into(),
                io_read: io_read.clone().into(),
                io_write: io_write.clone().into(),
                note: row.working_set_note.into(),
                accessible: format!(
                    "Process {}: {}. Working set {}. Private commit {}. CPU {} percent of machine capacity. IO read {} per second, write {} per second.",
                    row.pid, row.name, working_set, cell(row.private_commit), cpu, io_read, io_write
                )
                .into(),
            }
        })
        .collect()
}

/// Build the human note line under the sort controls.
fn process_note(page: &ProcessPage, row_count: usize) -> String {
    let mut note = format!(
        "{} processes · sorted by {} · rate window {} ms",
        row_count,
        sort_label(&page.sort_key),
        page.rate_interval_ms
    );
    if let Some(cap) = &page.row_cap_note {
        note.push_str(&format!(" · {cap}"));
    }
    note
}

/// Sort the page's authoritative rows by the current key and push them into
/// the UI model with a refreshed note.
fn push_process_rows(app: &AppWindow, page: &Mutex<ProcessPage>) {
    let (rows, note) = {
        let mut guard = match page.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let key = guard.sort_key.clone();
        sort_process_rows(&mut guard.rows, &key);
        let note = process_note(&guard, guard.rows.len());
        (guard.rows.clone(), note)
    };
    let model = to_model_rows(&rows);
    let ui = app.global::<UiState<'_>>();
    ui.set_process_rows(std::rc::Rc::new(slint::VecModel::from(model)).into());
    ui.set_process_note(note.into());
}

/// Apply one fetch outcome on the UI thread.
#[cfg(windows)]
fn apply_process_result(
    app: &AppWindow,
    page: &Mutex<ProcessPage>,
    result: Result<serde_json::Value, String>,
) {
    match result {
        Ok(inventory) => {
            if let Ok(mut guard) = page.lock() {
                guard.rows = parse_process_rows(&inventory);
                guard.rate_interval_ms = inventory["rate_interval_ms"].as_u64().unwrap_or(0);
                guard.row_cap_note = inventory["row_cap_note"].as_str().map(str::to_string);
            }
            push_process_rows(app, page);
        }
        Err(reason) => {
            if let Ok(mut guard) = page.lock() {
                guard.rows.clear();
            }
            let ui = app.global::<UiState<'_>>();
            ui.set_process_rows(
                std::rc::Rc::new(slint::VecModel::from(Vec::<ProcessRow>::new())).into(),
            );
            ui.set_process_note(format!("Process table unavailable: {reason}").into());
        }
    }
}

/// One-shot `GetProcesses` on a fresh connection (RS-011). Deliberately not
/// the subscribe stream: a UI that is not looking at the table costs the
/// agent nothing.
/// Which page material the shared poll fetches this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchKind {
    Processes,
    Actions,
    Config,
}

/// One ListActions response row rendered in the Actions page.
#[derive(Debug, Clone, PartialEq)]
struct ActionRowData {
    action_id: String,
    target: String,
    state: String,
    deadline: String,
    badge: String,
    restorable: bool,
    accessible: String,
}

/// Parse a `ListActions` payload into rows (pure; testable). Restorable =
/// unresolved records in a state a restore still applies to; terminal rows
/// render as history.
#[cfg(windows)]
fn actions_rows_from(data: &serde_json::Value) -> Vec<ActionRowData> {
    use ramshaper_core::clock::WallClock;
    let mut rows = Vec::new();
    let Some(actions) = data["actions"].as_array() else {
        return rows;
    };
    for action in actions {
        let state = action["state"].as_str().unwrap_or("Unknown").to_string();
        let unresolved = action["unresolved"].as_bool().unwrap_or(false);
        let deadline = action["applied_deadline_unix_ms"].as_u64().map_or_else(
            || "no deadline".to_string(),
            |deadline| {
                let now = ramshaper_core::SystemClock.unix_epoch_ms();
                if now >= deadline {
                    "expired".to_string()
                } else {
                    format!("expires in {} s", (deadline - now) / 1_000)
                }
            },
        );
        let badge = if unresolved { "pending" } else { "history" }.to_string();
        let restorable = unresolved && matches!(state.as_str(), "Applied" | "RestorePending");
        let action_id = action["action_id"].as_str().unwrap_or("?").to_string();
        rows.push(ActionRowData {
            accessible: format!(
                "Action {action_id} on {}, state {state}, {badge}, deadline {deadline}",
                action["target_name"].as_str().unwrap_or("unknown target"),
            ),
            action_id,
            target: action["target_name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            state,
            deadline,
            badge,
            restorable,
        });
    }
    rows
}

#[cfg(windows)]
fn fetch_actions(pipe_name: &str) -> Result<serde_json::Value, String> {
    use ramshaper_core::ipc::{RequestEnvelope, COMMAND_LIST_ACTIONS, PROTOCOL_VERSION};
    use ramshaper_windows::pipe::{client_connect, read_frame, write_frame};

    let client =
        client_connect(pipe_name).map_err(|error| format!("agent not reachable ({error})"))?;
    let envelope = RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: "ui-actions".to_string(),
        agent_instance_id: None,
        command: COMMAND_LIST_ACTIONS.to_string(),
        payload: None,
    };
    let payload =
        serde_json::to_vec(&envelope).map_err(|error| format!("encode failed: {error}"))?;
    write_frame(&client, &payload).map_err(|error| format!("write failed: {error}"))?;
    let frame = read_frame(&client).map_err(|error| format!("read failed: {error}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&frame).map_err(|error| format!("malformed response: {error}"))?;
    if value["ok"] == true {
        Ok(value["data"].clone())
    } else {
        let code = value["error"]["code"].as_str().unwrap_or("unknown");
        let message = value["error"]["message"].as_str().unwrap_or("");
        Err(format!("{code}: {message}"))
    }
}

#[cfg(not(windows))]
fn fetch_actions(_pipe_name: &str) -> Result<serde_json::Value, String> {
    Err("the UI requires the Windows adapter".to_string())
}

/// Push a ListActions result into the Actions page (RS-013). An agent
/// without the policy gate answers the gateless stub (an empty list with
/// the observe-only note) — rendered as data, never as an error.
#[cfg(windows)]
fn apply_actions(app: &AppWindow, result: Result<serde_json::Value, String>) {
    let ui = app.global::<UiState<'_>>();
    match result {
        Ok(data) => {
            let rows = actions_rows_from(&data);
            let pending = rows.iter().filter(|r| r.badge == "pending").count();
            let note = format!(
                "{} action record(s), {} pending. {}",
                rows.len(),
                pending,
                data["note"].as_str().unwrap_or("")
            );
            let model: Vec<ActionRow> = rows
                .iter()
                .map(|row| ActionRow {
                    action_id: row.action_id.as_str().into(),
                    target: row.target.as_str().into(),
                    state: row.state.as_str().into(),
                    deadline: row.deadline.as_str().into(),
                    badge: row.badge.as_str().into(),
                    restorable: row.restorable,
                    accessible: row.accessible.as_str().into(),
                })
                .collect();
            ui.set_action_rows(std::rc::Rc::new(slint::VecModel::from(model)).into());
            ui.set_actions_note(note.into());
        }
        Err(reason) => {
            ui.set_action_rows(
                std::rc::Rc::new(slint::VecModel::from(Vec::<ActionRow>::new())).into(),
            );
            ui.set_actions_note(format!("Action records unavailable: {reason}").into());
        }
    }
}

/// One-shot RestoreAction / DisableActions request (RS-013 controls).
#[cfg(windows)]
fn run_policy_command(
    pipe_name: &str,
    command: &str,
    payload: Option<serde_json::Value>,
    request_id: &str,
) -> Result<serde_json::Value, String> {
    use ramshaper_core::ipc::{RequestEnvelope, PROTOCOL_VERSION};
    use ramshaper_windows::pipe::{client_connect, read_frame, write_frame};

    let client =
        client_connect(pipe_name).map_err(|error| format!("agent not reachable ({error})"))?;
    let envelope = RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: request_id.to_string(),
        agent_instance_id: None,
        command: command.to_string(),
        payload,
    };
    let payload_bytes =
        serde_json::to_vec(&envelope).map_err(|error| format!("encode failed: {error}"))?;
    write_frame(&client, &payload_bytes).map_err(|error| format!("write failed: {error}"))?;
    let frame = read_frame(&client).map_err(|error| format!("read failed: {error}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&frame).map_err(|error| format!("malformed response: {error}"))?;
    if value["ok"] == true {
        Ok(value["data"].clone())
    } else {
        let code = value["error"]["code"].as_str().unwrap_or("unknown");
        let message = value["error"]["message"].as_str().unwrap_or("");
        Err(format!("{code}: {message}"))
    }
}

#[cfg(windows)]
fn run_restore(pipe_name: &str, action_id: &str) -> Result<serde_json::Value, String> {
    use ramshaper_core::ipc::{RestoreActionRequest, COMMAND_RESTORE_ACTION};
    run_policy_command(
        pipe_name,
        COMMAND_RESTORE_ACTION,
        Some(
            serde_json::to_value(RestoreActionRequest {
                action_id: action_id.to_string(),
            })
            .map_err(|error| format!("encode failed: {error}"))?,
        ),
        "ui-restore",
    )
}

#[cfg(windows)]
fn run_disable(pipe_name: &str) -> Result<serde_json::Value, String> {
    use ramshaper_core::ipc::{DisableActionsRequest, COMMAND_DISABLE_ACTIONS};
    run_policy_command(
        pipe_name,
        COMMAND_DISABLE_ACTIONS,
        Some(
            serde_json::to_value(DisableActionsRequest {})
                .map_err(|error| format!("encode failed: {error}"))?,
        ),
        "ui-disable",
    )
}

/// Render a control outcome in the note (never a silent no-op; an agent
/// without the policy gate answers Unsupported and that is shown verbatim).
#[cfg(windows)]
fn apply_action_command_result(
    app: &AppWindow,
    control: &str,
    result: Result<serde_json::Value, String>,
) {
    let ui = app.global::<UiState<'_>>();
    let note = match result {
        Ok(data) => format!(
            "{control} result: {}",
            serde_json::to_string(&data).unwrap_or_else(|_| "ok".to_string())
        ),
        Err(reason) => format!("{control} refused: {reason}"),
    };
    ui.set_actions_note(note.into());
}

#[cfg(windows)]
fn fetch_processes(pipe_name: &str) -> Result<serde_json::Value, String> {
    use ramshaper_core::ipc::{RequestEnvelope, COMMAND_GET_PROCESSES, PROTOCOL_VERSION};
    use ramshaper_windows::pipe::{client_connect, read_frame, write_frame};

    let client =
        client_connect(pipe_name).map_err(|error| format!("agent not reachable ({error})"))?;
    let envelope = RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: "ui-processes".to_string(),
        agent_instance_id: None,
        command: COMMAND_GET_PROCESSES.to_string(),
        payload: None,
    };
    let payload =
        serde_json::to_vec(&envelope).map_err(|error| format!("encode failed: {error}"))?;
    write_frame(&client, &payload).map_err(|error| format!("write failed: {error}"))?;
    let frame = read_frame(&client).map_err(|error| format!("read failed: {error}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&frame).map_err(|error| format!("malformed response: {error}"))?;
    if value["ok"] == true {
        Ok(value["data"].clone())
    } else {
        let code = value["error"]["code"].as_str().unwrap_or("unknown");
        let message = value["error"]["message"].as_str().unwrap_or("");
        if code == "Unsupported" {
            Ok(serde_json::json!({
                "schema": "ramshaper.processes.v1",
                "processes": [],
                "agent_unsupported": true,
            }))
            .map(|mut v| {
                v["row_cap_note"] = serde_json::json!(
                    "this agent build does not serve process tables; restart the agent (GetProcesses unsupported)"
                );
                v
            })
        } else {
            Err(format!("{code}: {message}"))
        }
    }
}

#[cfg(not(windows))]
fn fetch_processes(_pipe_name: &str) -> Result<serde_json::Value, String> {
    Err("the UI requires the Windows adapter".to_string())
}

/// One-shot `GetConfig` on a fresh connection (RS-012 UI portion): the
/// serving agent's inert mode/profile/exclusions summary.
#[cfg(windows)]
fn fetch_config(pipe_name: &str) -> Result<serde_json::Value, String> {
    use ramshaper_core::ipc::{RequestEnvelope, COMMAND_GET_CONFIG, PROTOCOL_VERSION};
    use ramshaper_windows::pipe::{client_connect, read_frame, write_frame};

    let client =
        client_connect(pipe_name).map_err(|error| format!("agent not reachable ({error})"))?;
    let envelope = RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: "ui-config".to_string(),
        agent_instance_id: None,
        command: COMMAND_GET_CONFIG.to_string(),
        payload: None,
    };
    let payload =
        serde_json::to_vec(&envelope).map_err(|error| format!("encode failed: {error}"))?;
    write_frame(&client, &payload).map_err(|error| format!("write failed: {error}"))?;
    let frame = read_frame(&client).map_err(|error| format!("read failed: {error}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&frame).map_err(|error| format!("malformed response: {error}"))?;
    if value["ok"] == true {
        Ok(value["data"].clone())
    } else {
        let code = value["error"]["code"].as_str().unwrap_or("unknown");
        let message = value["error"]["message"].as_str().unwrap_or("");
        Err(format!("{code}: {message}"))
    }
}

#[cfg(not(windows))]
fn fetch_config(_pipe_name: &str) -> Result<serde_json::Value, String> {
    Err("the UI requires the Windows adapter".to_string())
}

/// Push a `GetConfig` result into the UI properties (RS-012 UI portion).
#[cfg(windows)]
fn apply_config(app: &AppWindow, result: Result<serde_json::Value, String>) {
    let ui = app.global::<UiState<'_>>();
    match result {
        Ok(config) => {
            ui.set_current_profile(config["profile"].as_str().unwrap_or("unknown").into());
            ui.set_config_note(
                format!(
                    "Mode: {}. Exclusions stored: {}.",
                    config["mode"].as_str().unwrap_or("unknown"),
                    config["exclusions"].as_array().map(Vec::len).unwrap_or(0)
                )
                .into(),
            );
        }
        Err(reason) => {
            ui.set_current_profile("unavailable".into());
            ui.set_config_note(format!("Profile information unavailable: {reason}").into());
        }
    }
}

/// One formatted screen update (all strings built here; the .slint file
/// only presents them).
struct Display {
    os_label: slint::SharedString,
    pressure_level: slint::SharedString,
    pressure_heading: slint::SharedString,
    pressure_evidence: slint::SharedString,
    pressure_missing: slint::SharedString,
    low_memory_state: slint::SharedString,
    phys_total: slint::SharedString,
    phys_available: slint::SharedString,
    phys_in_use: slint::SharedString,
    commit_used: slint::SharedString,
    commit_limit: slint::SharedString,
    commit_headroom: slint::SharedString,
    cta_text: slint::SharedString,
    healthy: bool,
    elevated: bool,
}

/// Pure formatting from an agent snapshot payload to display values.
/// Exported for unit tests.
fn snapshot_to_display(snapshot: &serde_json::Value) -> Display {
    let metrics = &snapshot["metrics"];
    let metric = |key: &str| -> (Option<u64>, String) {
        let envelope = &metrics[key];
        match envelope["value"].as_u64() {
            Some(value) => (Some(value), format_bytes(value)),
            None => (None, unavailable_text(&envelope["status"])),
        }
    };

    let (_, phys_total_text) = metric("physical_total_bytes");
    let (phys_available, phys_available_text) = metric("physical_available_bytes");
    let (_, phys_in_use_text) = metric("physical_in_use_bytes");
    let (_, commit_used_text) = metric("commit_used_bytes");
    let (_, commit_limit_text) = metric("commit_limit_bytes");
    let (_, commit_headroom_text) = metric("commit_headroom_bytes");

    let low_memory = &metrics["low_memory"];
    let low_memory_state = match low_memory["state"].as_str() {
        Some("signaled") => "signaled".to_string(),
        Some("not_signaled") => "not signaled".to_string(),
        _ => format!(
            "unavailable ({})",
            low_memory["status"].as_str().unwrap_or("unknown")
        ),
    };

    let pressure = &snapshot["pressure"];
    let level = pressure["level"].as_str().unwrap_or("unknown");
    let (level_word, heading, healthy, elevated) = match level {
        "normal" => (
            "Normal",
            "No low-memory signal observed".to_string(),
            true,
            false,
        ),
        "elevated" => (
            "Elevated",
            "Windows is reporting low available memory".to_string(),
            false,
            true,
        ),
        _ => (
            "Unknown",
            "Not enough valid observations to classify memory pressure".to_string(),
            false,
            false,
        ),
    };
    let missing = pressure["missing_inputs"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let pressure_missing = if missing.is_empty() {
        String::new()
    } else {
        format!("Evidence incomplete — missing: {missing}")
    };
    let consecutive_signal = pressure["consecutive_signal"].as_u64().unwrap_or(0);
    let consecutive_clear = pressure["consecutive_clear"].as_u64().unwrap_or(0);
    let pressure_evidence = format!(
        "Rule {} · pending evidence: {consecutive_signal} signal / {consecutive_clear} clear observations",
        pressure["rule_version"].as_str().unwrap_or("pressure_v1"),
    );

    // CTA per spec: healthy => "Nothing to optimize right now."
    let cta_text = if level == "elevated" {
        "Windows is reporting low available memory — no action is available in this build (Observe Only)."
    } else if level == "normal" {
        "Nothing to optimize right now."
    } else if phys_available.is_none() {
        "Memory readings unavailable — see the status shown for each metric."
    } else {
        "Collecting observations…"
    };
    let os_label = format!(
        "{} · {}",
        snapshot["os"]["product_name"]
            .as_str()
            .unwrap_or("unknown product"),
        snapshot["os"]["version"]
            .as_str()
            .unwrap_or("unknown version"),
    );

    Display {
        os_label: os_label.into(),
        pressure_level: level_word.into(),
        pressure_heading: heading.into(),
        pressure_evidence: pressure_evidence.into(),
        pressure_missing: pressure_missing.into(),
        low_memory_state: low_memory_state.into(),
        phys_total: phys_total_text.into(),
        phys_available: phys_available_text.into(),
        phys_in_use: phys_in_use_text.into(),
        commit_used: commit_used_text.into(),
        commit_limit: commit_limit_text.into(),
        commit_headroom: commit_headroom_text.into(),
        cta_text: cta_text.into(),
        healthy,
        elevated,
    }
}

fn unavailable_text(status: &serde_json::Value) -> String {
    format!("unavailable ({})", status.as_str().unwrap_or("error"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> serde_json::Value {
        serde_json::json!({
            "schema": "ramshaper.snapshot.v1",
            "os": {"product_name": "Windows 11 Pro", "version": "10.0.26200.9711"},
            "metrics": {
                "physical_total_bytes": {"value": 68448034816u64, "status": "ok"},
                "physical_available_bytes": {"value": 38400000000u64, "status": "ok"},
                "physical_in_use_bytes": {"value": 30048034816u64, "status": "ok"},
                "commit_used_bytes": {"value": 33400000000u64, "status": "ok"},
                "commit_limit_bytes": {"value": 72743002112u64, "status": "ok"},
                "commit_headroom_bytes": {"value": 39343002112u64, "status": "ok"},
                "low_memory": {"state": "not_signaled", "status": "ok"}
            },
            "pressure": {
                "rule_version": "pressure_v1",
                "level": "normal",
                "missing_inputs": [],
                "consecutive_signal": 0,
                "consecutive_clear": 7
            }
        })
    }

    #[test]
    fn formats_healthy_snapshot_per_spec() {
        let display = snapshot_to_display(&sample_snapshot());
        assert_eq!(display.pressure_level, "Normal");
        assert_eq!(display.pressure_heading, "No low-memory signal observed");
        assert_eq!(display.cta_text, "Nothing to optimize right now.");
        assert_eq!(display.low_memory_state, "not signaled");
        assert!(display.healthy);
        assert!(display.phys_available.starts_with(char::is_numeric));
        assert!(display.pressure_missing.is_empty());
        assert!(display.pressure_evidence.contains("pressure_v1"));
    }

    #[test]
    fn actions_rows_mark_pending_and_history_and_restore_scope() {
        let data: serde_json::Value = serde_json::from_str(
            r#"{"actions":[
                {"action_id":"act-1","state":"Applied","target_name":"leaf.exe",
                 "applied_deadline_unix_ms":999999999999999,"unresolved":true},
                {"action_id":"act-2","state":"Conflict","target_name":"other.exe",
                 "applied_deadline_unix_ms":1,"unresolved":true},
                {"action_id":"act-3","state":"Restored","target_name":"old.exe",
                 "applied_deadline_unix_ms":1,"unresolved":false}
            ]}"#,
        )
        .expect("payload");
        let rows = actions_rows_from(&data);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].badge, "pending");
        assert!(rows[0].restorable, "an Applied pending row is restorable");
        assert!(
            rows[0].deadline.starts_with("expires in"),
            "future deadline renders as time left: {}",
            rows[0].deadline
        );
        assert_eq!(rows[1].state, "Conflict");
        assert!(
            !rows[1].restorable,
            "a Conflict is preserved, not restorable over"
        );
        assert_eq!(rows[1].deadline, "expired");
        assert_eq!(rows[2].badge, "history");
        assert!(!rows[2].restorable);
        assert!(
            rows.iter().all(|r| r.accessible.contains("state")),
            "every row announces its state as text (no color-only info)"
        );
    }

    #[test]
    fn actions_rows_of_an_empty_stub_render_nothing() {
        let data: serde_json::Value =
            serde_json::from_str(r#"{"actions":[],"note":"no policy actions exist"}"#)
                .expect("payload");
        let rows = actions_rows_from(&data);
        assert!(rows.is_empty());
    }

    #[test]
    fn formats_elevated_and_unavailable_states() {
        let mut snapshot = sample_snapshot();
        snapshot["pressure"]["level"] = serde_json::json!("elevated");
        let display = snapshot_to_display(&snapshot);
        assert_eq!(
            display.pressure_heading,
            "Windows is reporting low available memory"
        );
        assert!(display.elevated && !display.healthy);
        assert!(display.cta_text.contains("Observe Only"));

        let mut broken = sample_snapshot();
        broken["metrics"]["physical_available_bytes"] =
            serde_json::json!({"status": "access_denied"});
        broken["pressure"]["level"] = serde_json::json!("unknown");
        broken["pressure"]["missing_inputs"] = serde_json::json!(["physical_available"]);
        let display = snapshot_to_display(&broken);
        assert_eq!(display.phys_available, "unavailable (access_denied)");
        assert_eq!(display.pressure_level, "Unknown");
        assert!(display.pressure_missing.contains("physical_available"));
        assert!(!display.healthy && !display.elevated);
    }

    #[test]
    fn pipe_names_are_normalized() {
        assert_eq!(normalize_pipe_name("x"), r"\\.\pipe\x");
        assert_eq!(normalize_pipe_name(r"\\.\pipe\x"), r"\\.\pipe\x");
    }

    #[test]
    fn poll_ms_is_a_positive_whole_number() {
        assert_eq!(parse_poll_ms("1000"), Ok(1000));
        assert_eq!(parse_poll_ms("1"), Ok(1));
        assert!(parse_poll_ms("0").is_err());
        assert!(parse_poll_ms("abc").is_err());
        assert!(parse_poll_ms("-5").is_err());
        assert!(parse_poll_ms("1.5").is_err());
    }
}

#[cfg(test)]
mod process_page_tests {
    use super::*;

    /// Synthetic inventory fixture (marked synthetic per contract 14 §8:
    /// JSON fixtures in tests are never real machine data).
    fn sample_inventory() -> serde_json::Value {
        serde_json::json!({
            "schema": "ramshaper.processes.v1",
            "observed_at_utc": "2026-10-24T12:00:00.000Z",
            "rate_interval_ms": 2000,
            "processes": [
                {
                    "identity": {"pid": 301, "creation_filetime_100ns": "133000000000000000"},
                    "name": "alpha.exe",
                    "working_set": {"value": 104857600u64, "status": "ok"},
                    "private_commit": {"value": 52428800u64, "status": "ok"},
                    "cpu_percent": {"rate": 12.5, "status": "ok"},
                    "io_read": {"rate": 4096u64, "status": "ok"},
                    "io_write": {"rate": 2048u64, "status": "ok"}
                },
                {
                    "identity": {"pid": 202, "creation_filetime_100ns": "133000000000000001"},
                    "name": "beta.exe",
                    "working_set": {"value": 209715200u64, "status": "ok"},
                    "private_commit": {"value": 104857600u64, "status": "ok"},
                    "cpu_percent": {"rate": 3.25, "status": "ok"},
                    "io_read": {"rate": 8192u64, "status": "ok"},
                    "io_write": {"rate": 1024u64, "status": "ok"}
                },
                {
                    "identity": {"pid": 103, "creation_filetime_100ns": "133000000000000002"},
                    "name": "protected.sys",
                    "working_set": {"status": "access_denied"},
                    "private_commit": {"status": "access_denied"},
                    "cpu_percent": {"status": "access_denied"},
                    "io_read": {"status": "access_denied"},
                    "io_write": {"status": "access_denied"}
                }
            ]
        })
    }

    #[test]
    fn parsing_keeps_numeric_keys_and_marks_denied_rows() {
        let rows = parse_process_rows(&sample_inventory());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].pid, 301);
        assert_eq!(rows[0].working_set, Some(104_857_600));
        assert_eq!(rows[0].cpu_percent, Some(12.5));
        assert_eq!(rows[0].io_read_rate, Some(4096));
        assert_eq!(rows[0].io_write_rate, Some(2048));
        assert_eq!(rows[0].working_set_note, "");
        assert_eq!(rows[0].io_read_note, "");
        // The protected row is unavailable with a reason, never zero.
        assert_eq!(rows[2].working_set, None);
        assert_eq!(rows[2].working_set_note, "[denied]");
        assert_eq!(rows[2].cpu_percent, None);
        assert_eq!(rows[2].cpu_note, "[denied]");
        assert_eq!(rows[2].io_read_note, "[denied]");
        assert_eq!(rows[2].name, "protected.sys");
    }

    #[test]
    fn sorting_covers_all_keys_and_sinks_unavailable() {
        let mut rows = parse_process_rows(&sample_inventory());
        sort_process_rows(&mut rows, "working-set");
        assert_eq!(rows[0].name, "beta.exe"); // 200 MiB > 100 MiB > None
        assert_eq!(rows[2].name, "protected.sys");
        sort_process_rows(&mut rows, "private");
        assert_eq!(rows[0].name, "beta.exe");
        sort_process_rows(&mut rows, "cpu");
        assert_eq!(rows[0].name, "alpha.exe"); // 12.5 > 3.25 > None
        sort_process_rows(&mut rows, "name");
        assert_eq!(rows[0].name, "alpha.exe");
        assert_eq!(rows[2].name, "protected.sys");
        // Unknown keys fall back to the working-set sort.
        sort_process_rows(&mut rows, "bogus");
        assert_eq!(rows[0].name, "beta.exe");
    }

    #[test]
    fn model_rows_carry_units_and_whole_row_accessibility() {
        let mut rows = parse_process_rows(&sample_inventory());
        sort_process_rows(&mut rows, "working-set");
        let model = to_model_rows(&rows);
        assert_eq!(model.len(), 3);
        let beta = &model[0];
        assert_eq!(beta.pid, 202);
        assert!(beta.working_set.contains("200"), "{}", beta.working_set);
        assert!(beta.working_set.contains("MiB"), "{}", beta.working_set);
        assert_eq!(beta.cpu, "3.25");
        assert_eq!(beta.note, "");
        let protected = &model[2];
        assert_eq!(protected.working_set, "[unavailable]");
        assert_eq!(protected.note, "[denied]");
        assert_eq!(protected.io_read, "[denied]");
        assert_eq!(protected.io_write, "[denied]");
        assert!(
            protected
                .accessible
                .starts_with("Process 103: protected.sys."),
            "{}",
            protected.accessible
        );
        assert!(protected.accessible.contains("unavailable"));
    }

    #[test]
    fn note_reports_count_sort_and_cap() {
        let mut page = ProcessPage {
            sort_key: "working-set".to_string(),
            rate_interval_ms: 2000,
            ..ProcessPage::default()
        };
        assert_eq!(
            process_note(&page, 440),
            "440 processes · sorted by working set · rate window 2000 ms"
        );
        page.row_cap_note = Some("served the top 1024 processes".to_string());
        assert!(process_note(&page, 1024).contains("served the top 1024"));
    }

    #[test]
    fn missing_processes_array_is_an_empty_table_not_a_panic() {
        let rows = parse_process_rows(&serde_json::json!({ "schema": "x" }));
        assert!(rows.is_empty());
        let rows = parse_process_rows(&serde_json::json!(null));
        assert!(rows.is_empty());
    }
}

#[cfg(test)]
mod contrast_audit {
    /// WCAG 2.x relative luminance of an sRGB color (0xRRGGBB).
    fn relative_luminance(rgb: u32) -> f64 {
        let channel = |shift: u32| -> f64 {
            let c = ((rgb >> shift) & 0xFF) as f64 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    /// WCAG 2.x contrast ratio between two colors, in [1, 21].
    fn contrast_ratio(a: u32, b: u32) -> f64 {
        let (l1, l2) = {
            let (la, lb) = (relative_luminance(a), relative_luminance(b));
            if la >= lb {
                (la, lb)
            } else {
                (lb, la)
            }
        };
        (l1 + 0.05) / (l2 + 0.05)
    }

    fn parse_hex(text: &str) -> u32 {
        let digits = text.trim_start_matches('#');
        u32::from_str_radix(digits, 16).expect("valid #RRGGBB color")
    }

    /// Parse the `Palette` global out of the live `app-window.slint`, so the
    /// audit tracks token edits instead of a drifted copy.
    fn palette_from_slint() -> std::collections::BTreeMap<String, String> {
        let source = std::fs::read_to_string("ui/app-window.slint")
            .expect("read ui/app-window.slint (tests run from the crate root)");
        let start = source
            .find("export global Palette {")
            .expect("Palette global present in app-window.slint");
        let block = &source[start..];
        let end = block.find('}').expect("Palette block close");
        let block = &block[..end];
        let mut map = std::collections::BTreeMap::new();
        for line in block.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("out property <color> ") {
                if let Some(colon) = rest.find(':') {
                    let name = rest[..colon].trim().trim_end_matches(';').to_string();
                    let value = rest[colon + 1..]
                        .trim()
                        .trim_end_matches(';')
                        .trim()
                        .to_string();
                    map.insert(name, value);
                }
            }
        }
        assert!(
            map.contains_key("text-primary") && map.contains_key("accent"),
            "palette parse must find the core tokens; found {map:?}"
        );
        map
    }

    /// One audited usage: foreground on background at a WCAG level.
    /// WCAG 1.4.3: 4.5:1 for normal text, 3:1 for large text (≥ 24 px, or
    /// ≥ 18.66 px bold) and for meaningful non-text UI boundaries (1.4.11).
    struct Pair {
        fg: String,
        bg: String,
        minimum: f64,
        usage: &'static str,
    }

    fn audited_pairs(palette: &std::collections::BTreeMap<String, String>) -> Vec<Pair> {
        let color = |name: &str| -> String {
            palette
                .get(name)
                .cloned()
                .unwrap_or_else(|| panic!("palette token '{name}' missing"))
        };
        vec![
            // Page and card body text (normal size → 4.5).
            Pair {
                fg: color("text-primary"),
                bg: color("page-background"),
                minimum: 4.5,
                usage: "body text on page",
            },
            Pair {
                fg: color("text-primary"),
                bg: color("card-background"),
                minimum: 4.5,
                usage: "body text on cards",
            },
            Pair {
                fg: color("text-secondary"),
                bg: color("page-background"),
                minimum: 4.5,
                usage: "secondary/footnote text on page (11-12 px)",
            },
            Pair {
                fg: color("text-secondary"),
                bg: color("card-background"),
                minimum: 4.5,
                usage: "secondary text on cards (table headers)",
            },
            Pair {
                fg: color("accent"),
                bg: color("page-background"),
                minimum: 4.5,
                usage: "CTA line on page",
            },
            Pair {
                fg: color("warning"),
                bg: color("page-background"),
                minimum: 4.5,
                usage: "stale freshness label on page (12 px)",
            },
            // Navigation sidebar.
            Pair {
                fg: "#FFFFFF".to_string(),
                bg: color("nav-background"),
                minimum: 4.5,
                usage: "brand title on nav",
            },
            Pair {
                fg: "#C9D7EA".to_string(),
                bg: color("nav-background"),
                minimum: 4.5,
                usage: "nav inactive labels + mode chip on nav",
            },
            Pair {
                fg: "#9FB2CC".to_string(),
                bg: color("nav-background"),
                minimum: 4.5,
                usage: "nav tagline on nav (11 px)",
            },
            Pair {
                fg: "#FFFFFF".to_string(),
                bg: "#0F1B2E".to_string(),
                minimum: 4.5,
                usage: "active nav item + mode chip face",
            },
            // PaletteButton (AT-19 fix for the invisible stock Button).
            Pair {
                fg: "#FFFFFF".to_string(),
                bg: color("accent"),
                minimum: 4.5,
                usage: "active button face",
            },
            Pair {
                fg: color("text-primary"),
                bg: color("card-background"),
                minimum: 4.5,
                usage: "inactive button face",
            },
            Pair {
                fg: color("text-secondary"),
                bg: color("card-background"),
                minimum: 3.0,
                usage: "inactive button border (non-text boundary, 1.4.11)",
            },
            // Large-text statuses (26 px weight 800 → 3.0 allowed; kept as
            // explicit large-text pairs so a size change re-audits).
            Pair {
                fg: color("healthy"),
                bg: color("card-background"),
                minimum: 3.0,
                usage: "large OK glyph on pressure card",
            },
            Pair {
                fg: color("warning"),
                bg: color("card-background"),
                minimum: 3.0,
                usage: "large ATTN glyph on pressure card",
            },
        ]
    }

    #[test]
    fn wcag_math_matches_published_check_values() {
        assert!((contrast_ratio(0x000000, 0xFFFFFF) - 21.0).abs() < 1e-9);
        // The classic "just barely AA" gray on white.
        let ratio = contrast_ratio(0x767676, 0xFFFFFF);
        assert!((ratio - 4.54).abs() < 0.02, "ratio {ratio}");
        // Order independence.
        assert!((contrast_ratio(0xFFFFFF, 0x000000) - 21.0).abs() < 1e-9);
    }

    #[test]
    fn every_ui_color_pair_meets_its_wcag_minimum() {
        let palette = palette_from_slint();
        let pairs = audited_pairs(&palette);
        assert!(pairs.len() >= 14, "audit coverage shrank");
        let mut failures = Vec::new();
        for pair in &pairs {
            let ratio = contrast_ratio(parse_hex(&pair.fg), parse_hex(&pair.bg));
            if ratio + 1e-9 < pair.minimum {
                failures.push(format!(
                    "{}: {} on {} = {ratio:.2} (needs {})",
                    pair.usage, pair.fg, pair.bg, pair.minimum
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "WCAG contrast failures (AT-19 1.4.3/1.4.11):\n{}",
            failures.join("\n")
        );
    }
}
