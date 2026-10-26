//! Session agent server (RS-030): collector + named-pipe listener.
//!
//! One agent per interactive user/logon session, no elevation. The pipe
//! carries an explicit user-only DACL; every accepted client is
//! re-validated (logon session + user SID) before dispatch. Protocol v1
//! frames per `ramshaper_core::ipc`; subscriber queues are bounded at two
//! pending frames with gap markers (contract 14 §3). All `unsafe` lives in
//! `ramshaper-windows`; this module is safe code.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use ramshaper_core::clock::{MonotonicClock as _, WallClock as _};
use ramshaper_core::ipc::{
    parse_request, response_frame, ErrorCode, Response, SnapshotDelivery,
    MAX_PENDING_SUBSCRIBER_FRAMES, PROTOCOL_VERSION,
};
use ramshaper_core::pressure::{PressureClassifier, PressureInput};
use ramshaper_core::snapshot::{SnapshotInputs, SystemSnapshot, ToolInfo};
use ramshaper_core::watch::{run_watch, WaitHandle, WatchConfig};
use ramshaper_platform::ReadOnlySystemMonitor as _;

use crate::dispatch::{dispatch, ConfigSummary, DispatchContext, LatestSnapshot, ProcessesContext};

/// Maximum simultaneously served connections (bounded memory).
pub const MAX_CONNECTIONS: usize = 8;

/// Bounded per-subscriber queue: at most two pending frames; overflow drops
/// the oldest and marks the next delivery with a gap.
#[derive(Debug)]
struct SubscriberQueue {
    frames: VecDeque<(u64, serde_json::Value)>,
    dropped: u32,
}

impl SubscriberQueue {
    fn new() -> Self {
        Self {
            frames: VecDeque::new(),
            dropped: 0,
        }
    }

    fn push(&mut self, sequence: u64, snapshot: serde_json::Value) {
        if self.frames.len() >= MAX_PENDING_SUBSCRIBER_FRAMES {
            self.frames.pop_front();
            self.dropped += 1;
        }
        self.frames.push_back((sequence, snapshot));
    }

    fn pop(&mut self) -> Option<SnapshotDelivery> {
        let (_, snapshot) = self.frames.pop_front()?;
        let gap_dropped = self.dropped;
        self.dropped = 0;
        Some(SnapshotDelivery {
            snapshot,
            gap_dropped,
        })
    }
}

/// Shared agent state.
struct Shared {
    latest: Mutex<Option<LatestSnapshot>>,
    subscribers: Mutex<Vec<(u64, SubscriberQueue)>>,
    notify: Condvar,
    /// Monotonic snapshot cadence (used for staleness limits).
    interval_ms: u64,
    /// Cross-request rate baselines for on-demand `GetProcesses` scans
    /// (RS-011). Shared by every connection so rates stay identity-keyed to
    /// the *previous* scan, wherever it came from.
    process_state: Mutex<ProcessScanState>,
    /// The policy gate when the agent was started with one (`None` =
    /// policy-stage commands answer `Unsupported`). Disabled at startup;
    /// startup reconciliation runs before the accept loop.
    policy: Option<Arc<crate::policy::PolicyGate>>,
}

/// Maximum rows served in one `GetProcesses` response (RS-011). The frame
/// cap is 1 MiB; ~4,096 identities × ~300 bytes of JSON would exceed it, so
/// the provider serves the heaviest processes first and notes the cap.
pub const MAX_IPC_PROCESS_ROWS: usize = 1024;

/// Rate baselines that survive across `GetProcesses` requests. The first
/// scan establishes them (honest `warming_up` rates); later scans compute
/// identity-keyed rates over the exact gap since the previous scan.
#[derive(Default)]
struct ProcessScanState {
    baselines: ramshaper_core::process::RateBaselines,
    last_scan_monotonic_ms: Option<u64>,
}

impl ProcessScanState {
    /// Fold a completed scan (taken at `scan_monotonic_ms`) into the
    /// baselines; returns the rate outcome and the effective interval.
    fn advance(
        &mut self,
        scan: &ramshaper_core::process::ProcessScan,
        scan_monotonic_ms: u64,
    ) -> (ramshaper_core::process::RateOutcome, u64) {
        let elapsed = self
            .last_scan_monotonic_ms
            .map_or(0, |last| scan_monotonic_ms.saturating_sub(last));
        self.last_scan_monotonic_ms = Some(scan_monotonic_ms);
        (self.baselines.advance(scan, elapsed), elapsed)
    }
}

impl Shared {
    fn publish(&self, latest: LatestSnapshot) {
        {
            let mut guard = self.latest.lock().expect("latest lock");
            *guard = Some(latest.clone());
        }
        let mut subscribers = self.subscribers.lock().expect("subscribers lock");
        for (id, queue) in subscribers.iter_mut() {
            queue.push(latest.sequence, latest.snapshot.clone());
            let _ = id;
        }
        self.notify.notify_all();
    }
}

/// Handle to a running agent (stop + join).
pub struct AgentHandle {
    stop: Arc<ramshaper_windows::event::ManualResetEvent>,
    workers: Vec<std::thread::JoinHandle<()>>,
    /// Diagnostic slot for accept-loop failures (best-effort).
    pub accept_error: Arc<Mutex<Option<String>>>,
    /// The accept thread's handle, used to cancel its blocked
    /// `ConnectNamedPipe` (synchronous handles cannot wait on the stop
    /// event; `CancelSynchronousIo` is the documented cancellation).
    canceller: Arc<Mutex<Option<ramshaper_windows::handle::OwnedHandle>>>,
    pub pipe_name: String,
    pub instance_id: String,
    /// The policy gate when the agent was started with one. Held so
    /// `stop` can run the contract's clean-exit trigger (restore
    /// everything outstanding) before the workers shut down.
    policy: Option<Arc<crate::policy::PolicyGate>>,
}

impl AgentHandle {
    /// Signal stop, cancel any blocked accept call, and join the accept
    /// loop and collector threads. With a policy gate present, a **clean
    /// agent exit initiates restoration first** (contract 14 §6): the
    /// gate is disabled (no new applies), every outstanding record
    /// restores through the engine, and the counts are reported — while
    /// the pipe is still serving, so clients observe the disable.
    pub fn stop(self) {
        if let Some(gate) = &self.policy {
            let now = ramshaper_core::SystemClock.unix_epoch_ms();
            let outcome = gate.disable(&ramshaper_core::ipc::DisableActionsRequest {}, now);
            match outcome {
                crate::policy::ResponseData::Ok(summary) => eprintln!(
                    "clean-exit restoration: disabled; restored={}, unresolved={}",
                    summary["restored"], summary["unresolved"]
                ),
                crate::policy::ResponseData::Err { code, message, .. } => {
                    eprintln!("clean-exit restoration failed: {code:?}: {message}")
                }
            }
        }
        self.stop.trigger();
        if let Ok(mut slot) = self.canceller.lock() {
            if let Some(thread) = slot.take() {
                let _ = ramshaper_windows::pipe::cancel_synchronous_io(&thread);
            }
        }
        for worker in self.workers {
            let _ = worker.join();
        }
    }
}

/// Derived default pipe name, scoped to the logon session.
#[cfg(windows)]
pub fn default_pipe_name() -> Result<String, ramshaper_core::snapshot::QueryFailure> {
    let session = ramshaper_windows::token::current_session_id()?;
    Ok(format!(r"\\.\pipe\ramshaper-agent-s{session}-v1"))
}

/// On-demand process-inventory provider (RS-011). Each `GetProcesses`
/// request triggers exactly one fresh scan; rates come from the shared
/// cross-request baselines in `Shared`. An idle agent never scans for
/// processes, keeping the NFR-01 CPU budget for the snapshot collector only.
#[cfg(windows)]
#[derive(Clone)]
struct ProcessInventoryProvider {
    shared: Arc<Shared>,
}

#[cfg(windows)]
impl ProcessInventoryProvider {
    fn scan(&self) -> Result<serde_json::Value, String> {
        use ramshaper_core::clock::MonotonicClock as _;
        use ramshaper_platform::ReadOnlyProcessMonitor as _;

        let monitor = ramshaper_windows::WindowsSystemMonitor::new();
        let clock = ramshaper_core::SystemClock;
        let scan = monitor.collect_processes();
        if let Some(failure) = &scan.scan_failure {
            return Err(format!("process scan failed: {failure}"));
        }
        let (outcome, elapsed) = {
            let mut state = self
                .shared
                .process_state
                .lock()
                .expect("process state lock");
            state.advance(&scan, clock.monotonic_ms())
        };
        let mut entries = outcome.entries;
        // Serve the heaviest processes first; the row cap drops the tail and
        // is reported explicitly, never silently.
        entries.sort_by(|a, b| {
            b.working_set
                .value
                .cmp(&a.working_set.value)
                .then_with(|| a.identity.pid.cmp(&b.identity.pid))
        });
        let row_capped = entries.len() > MAX_IPC_PROCESS_ROWS;
        entries.truncate(MAX_IPC_PROCESS_ROWS);
        let observed_at = ramshaper_core::rfc3339::format_utc_from_unix_ms(clock.unix_epoch_ms());
        let inventory = ramshaper_core::process::build_inventory(
            entries,
            scan.logical_cpu_count,
            elapsed,
            observed_at,
            1,
            scan.truncated,
            Some(outcome.churn),
        );
        let mut value = serde_json::to_value(&inventory)
            .map_err(|e| format!("inventory serialization failed: {e}"))?;
        if row_capped {
            if let serde_json::Value::Object(map) = &mut value {
                map.insert(
                    "row_cap_note".to_string(),
                    serde_json::json!(format!(
                        "served the top {MAX_IPC_PROCESS_ROWS} processes by working set (1 MiB frame cap); heavier processes only"
                    )),
                );
            }
        }
        Ok(value)
    }
}

/// Validation verdict for an accepted pipe client.
pub enum PeerVerdict {
    /// Peer matches the serving user and logon session.
    Accepted,
    /// Peer rejected (cross-session, foreign user or unqueryable).
    Rejected(String),
}

/// Pure decision over the observed peer facts (unit-testable without a
/// second session). Same-user malware in the same session is explicitly
/// *not* isolated (R-07) — the DACL and this check stop other
/// users/sessions only.
pub fn validate_peer(
    client_session: u32,
    client_sid: Option<&str>,
    our_session: u32,
    our_sid: &str,
) -> PeerVerdict {
    if client_session != our_session {
        return PeerVerdict::Rejected(format!(
            "client session {client_session} does not match agent session {our_session}"
        ));
    }
    match client_sid {
        Some(sid) if sid == our_sid => PeerVerdict::Accepted,
        Some(_) => PeerVerdict::Rejected("client user does not match the agent user".to_string()),
        None => PeerVerdict::Rejected("client user could not be determined".to_string()),
    }
}

/// Run the agent until `stop` fires. Blocks the calling thread (the accept
/// loop); returns a handle for orderly shutdown.
#[cfg(windows)]
pub fn serve(
    stop: Arc<ramshaper_windows::event::ManualResetEvent>,
    pipe_name: String,
    config: WatchConfig,
    config_summary: ConfigSummary,
    policy: Option<Arc<crate::policy::PolicyGate>>,
) -> Result<AgentHandle, String> {
    let instance_id = ramshaper_windows::agent_instance_id();

    let sid = ramshaper_windows::token::current_user_sid()
        .map_err(|f| format!("cannot determine serving user SID: {f}"))?;
    let our_session = ramshaper_windows::token::current_session_id()
        .map_err(|f| format!("cannot determine session id: {f}"))?;
    let security_descriptor = ramshaper_windows::token::security_descriptor_for_user(&sid)
        .map_err(|f| format!("cannot build pipe DACL: {f}"))?;

    let capabilities = {
        let monitor = ramshaper_windows::WindowsSystemMonitor::new();
        let caps = monitor.capabilities();
        serde_json::json!({
            "protocol_version": PROTOCOL_VERSION,
            "snapshot_interval_ms": config.interval_ms(),
            "capabilities": caps.entries.iter().map(|c| serde_json::json!({
                "name": c.name,
                "supported": c.supported,
                "reason": c.reason,
            })).collect::<Vec<_>>(),
        })
    };

    let shared = Arc::new(Shared {
        latest: Mutex::new(None),
        subscribers: Mutex::new(Vec::new()),
        notify: Condvar::new(),
        interval_ms: config.interval_ms(),
        process_state: Mutex::new(ProcessScanState::default()),
        policy: policy.clone(),
    });

    // Startup reconciliation (contract 14 §6): restore outstanding records
    // BEFORE serving, through the restore-direction loop (`policy_loop`).
    // The policy gate itself is constructed disabled, and nothing in the
    // agent enables it — startup is Observe Only.
    if let Some(gate) = &shared.policy {
        let mut loop_ = crate::policy_loop::PolicyLoop::new(
            crate::policy_loop::PolicyLoopConfig { enabled: true },
            gate.journal_clone(),
            ramshaper_windows::priority::WindowsPriorityActuator,
        );
        let resolved = loop_.startup_recovery(ramshaper_core::SystemClock.unix_epoch_ms());
        eprintln!("state=startup_recovery resolved={resolved}");
    }

    // Collector: samples on the watch cadence, publishes to shared state.
    let collector_shared = Arc::clone(&shared);
    let collector_stop = Arc::clone(&stop);
    let collector_instance_id = instance_id.clone();
    let collector = std::thread::Builder::new()
        .name("ramshaper-collector".to_string())
        .spawn(move || {
            let monitor = ramshaper_windows::WindowsSystemMonitor::new();
            let clock = ramshaper_core::SystemClock;
            let mut classifier = PressureClassifier::new();
            let mut sequence = 0u64;
            run_watch(&config, &clock, &*collector_stop, |_| {
                sequence += 1;
                let latest = collect_snapshot(
                    &monitor,
                    &clock,
                    &collector_instance_id,
                    sequence,
                    &mut classifier,
                );
                collector_shared.publish(latest);
            });
        })
        .map_err(|e| format!("cannot start collector: {e}"))?;

    let active = Arc::new(AtomicUsize::new(0));
    let accept_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let canceller: Arc<Mutex<Option<ramshaper_windows::handle::OwnedHandle>>> =
        Arc::new(Mutex::new(None));

    // The accept loop runs on its own thread; serve() returns immediately
    // with a handle that joins it and the collector on stop.
    let accept = {
        let stop = Arc::clone(&stop);
        let accept_error = Arc::clone(&accept_error);
        let accept_canceller = Arc::clone(&canceller);
        let accept_shared = Arc::clone(&shared);
        let accept_active = Arc::clone(&active);
        let accept_name = pipe_name.clone();
        let accept_id = instance_id.clone();
        let accept_sid = sid.0.clone();
        let accept_caps = capabilities;
        let accept_config = config_summary;
        let accept_sd = security_descriptor;
        std::thread::Builder::new()
            .name("ramshaper-accept".to_string())
            .spawn(move || {
                // Register this thread so stop() can cancel its blocked accept.
                if let Ok(thread) = ramshaper_windows::pipe::current_thread_handle() {
                    if let Ok(mut slot) = accept_canceller.lock() {
                        *slot = Some(thread);
                    }
                }
                let ctx = AcceptContext {
                    stop: &stop,
                    pipe_name: accept_name,
                    security_descriptor: accept_sd,
                    our_sid: accept_sid,
                    our_session,
                    instance_id: accept_id,
                    capabilities: accept_caps,
                    config_summary: accept_config,
                    shared: accept_shared,
                    active: accept_active,
                    process_provider: ProcessInventoryProvider {
                        shared: Arc::clone(&shared),
                    },
                };
                if let Err(message) = accept_loop(ctx) {
                    if let Ok(mut slot) = accept_error.lock() {
                        *slot = Some(message);
                    }
                    // Stop serving: wake the collector too.
                    stop.trigger();
                }
            })
            .map_err(|e| format!("cannot start accept loop: {e}"))?
    };

    Ok(AgentHandle {
        stop,
        workers: vec![collector, accept],
        accept_error,
        canceller,
        pipe_name,
        instance_id,
        policy,
    })
}

/// Accept clients until stop. Connection workers are detached: each ends on
/// peer disconnect or the stop signal; on stop this loop waits briefly for
/// active connections to drain before returning.
#[cfg(windows)]
struct AcceptContext<'a> {
    stop: &'a Arc<ramshaper_windows::event::ManualResetEvent>,
    pipe_name: String,
    security_descriptor: Vec<u8>,
    our_sid: String,
    our_session: u32,
    instance_id: String,
    capabilities: serde_json::Value,
    config_summary: ConfigSummary,
    shared: Arc<Shared>,
    active: Arc<AtomicUsize>,
    process_provider: ProcessInventoryProvider,
}

fn accept_loop(ctx: AcceptContext<'_>) -> Result<(), String> {
    let AcceptContext {
        stop,
        pipe_name,
        security_descriptor,
        our_sid,
        our_session,
        instance_id,
        capabilities,
        config_summary,
        shared,
        active,
        process_provider,
    } = ctx;
    let stop: &Arc<_> = stop;
    let pipe_name: &str = &pipe_name;
    let security_descriptor: &[u8] = &security_descriptor;
    let our_sid: &str = &our_sid;
    let instance_id: &str = &instance_id;
    let capabilities: &serde_json::Value = &capabilities;
    let config_summary: &ConfigSummary = &config_summary;
    let shared: &Arc<Shared> = &shared;
    let active: &Arc<AtomicUsize> = &active;
    let mut first = true;
    let mut next_subscriber_id: u64 = 0;
    loop {
        let instance = ramshaper_windows::pipe::create_instance(
            pipe_name,
            security_descriptor,
            first,
            MAX_CONNECTIONS as u32,
        )
        .map_err(|e| format!("CreateNamedPipeW failed: {e}"))?;
        first = false;

        if !ramshaper_windows::pipe::connect_cancellable(&instance)
            .map_err(|e| format!("ConnectNamedPipe failed: {e}"))?
        {
            // Stop requested while waiting: drain briefly, then exit.
            ramshaper_windows::pipe::disconnect(&instance);
            drop(instance);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while active.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            return Ok(());
        }

        // Bounded connections: shed load with an explicit Busy frame.
        if active.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            let busy = Response::error(
                "",
                instance_id,
                ErrorCode::Busy,
                "connection cap reached; retry shortly".to_string(),
            );
            if let Ok(frame) = response_frame(&busy) {
                let _ = ramshaper_windows::pipe::write_frame(&instance, &frame);
            }
            ramshaper_windows::pipe::disconnect(&instance);
            continue;
        }

        // Peer validation: session + user SID (R-07 boundary).
        let client_session = ramshaper_windows::pipe::client_session_id(&instance)
            .map_err(|e| format!("GetNamedPipeClientSessionId failed: {e}"))?;
        let peer_pid = ramshaper_windows::pipe::client_process_id(&instance)
            .map_err(|e| format!("GetNamedPipeClientProcessId failed: {e}"))?;
        let peer_sid = ramshaper_windows::token::user_sid_of(peer_pid)
            .ok()
            .map(|s| s.0);
        match validate_peer(client_session, peer_sid.as_deref(), our_session, our_sid) {
            PeerVerdict::Accepted => {}
            PeerVerdict::Rejected(reason) => {
                let denied = Response::error("", instance_id, ErrorCode::AccessDenied, reason);
                if let Ok(frame) = response_frame(&denied) {
                    let _ = ramshaper_windows::pipe::write_frame(&instance, &frame);
                }
                ramshaper_windows::pipe::disconnect(&instance);
                continue;
            }
        }

        // Hand the connected instance to a detached worker thread.
        active.fetch_add(1, Ordering::SeqCst);
        next_subscriber_id += 1;
        let worker_shared = Arc::clone(shared);
        let worker_active = Arc::clone(active);
        let worker_stop = Arc::clone(stop);
        let worker_material = ConnectionMaterial {
            instance_id: instance_id.to_string(),
            capabilities: capabilities.clone(),
            config_summary: config_summary.clone(),
        };
        let worker_processes = process_provider.clone();
        let worker = std::thread::Builder::new()
            .name("ramshaper-connection".to_string())
            .spawn(move || {
                handle_connection(
                    instance,
                    worker_material,
                    worker_shared,
                    worker_stop,
                    next_subscriber_id,
                    worker_processes,
                );
                worker_active.fetch_sub(1, Ordering::SeqCst);
            })
            .map_err(|e| format!("cannot start connection worker: {e}"))?;
        let _ = worker; // detached: ends on peer disconnect or stop
    }
}

/// Collect one snapshot with pressure classification embedded, as the
/// published JSON payload.
#[cfg(windows)]
fn collect_snapshot(
    monitor: &ramshaper_windows::WindowsSystemMonitor,
    clock: &ramshaper_core::SystemClock,
    instance_id: &str,
    sequence: u64,
    classifier: &mut PressureClassifier,
) -> LatestSnapshot {
    let sample = monitor.collect_system();
    let snapshot = ramshaper_core::snapshot::build_snapshot(SnapshotInputs {
        sample,
        os: monitor.os_info(),
        tool: ToolInfo {
            name: "ramshaper-agent".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        agent_instance_id: instance_id.to_string(),
        sequence,
        observed_at_unix_ms: clock.unix_epoch_ms(),
        monotonic_ms: clock.monotonic_ms(),
    });
    let pressure = classifier.observe(pressure_input_for(&snapshot));
    let mut value = serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null);
    if let serde_json::Value::Object(map) = &mut value {
        map.insert(
            "pressure".to_string(),
            serde_json::to_value(&pressure).unwrap_or_default(),
        );
    }
    LatestSnapshot {
        snapshot: value,
        sequence,
        monotonic_ms: snapshot.collected_monotonic_ms,
    }
}

#[cfg(windows)]
fn pressure_input_for(snapshot: &SystemSnapshot) -> Option<PressureInput> {
    let m = &snapshot.metrics;
    let missing = ramshaper_core::pressure::missing_inputs(
        m.physical_total.status.is_ok(),
        m.physical_available.status.is_ok(),
        m.commit_used.status.is_ok(),
        m.commit_limit.status.is_ok(),
        m.low_memory.status.is_ok(),
    );
    if missing.is_empty() {
        Some(PressureInput {
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

/// Serve one connected client: request/response until error, or a snapshot
/// stream after `SubscribeSnapshots`.
#[cfg(windows)]
/// Per-agent static material handed to every connection worker: identity,
/// the capability payload and the `GetConfig` summary (RS-030/011/012).
struct ConnectionMaterial {
    instance_id: String,
    capabilities: serde_json::Value,
    config_summary: ConfigSummary,
}

fn handle_connection(
    instance: ramshaper_windows::handle::OwnedHandle,
    material: ConnectionMaterial,
    shared: Arc<Shared>,
    stop: Arc<ramshaper_windows::event::ManualResetEvent>,
    subscriber_id: u64,
    process_provider: ProcessInventoryProvider,
) {
    // Peer gone or framing violations end the connection via `break` below.
    while let Ok(payload) = ramshaper_windows::pipe::read_frame(&instance) {
        let envelope = match parse_request(&payload) {
            Ok(envelope) => envelope,
            Err(_) => {
                // Malformed frame: one explicit error, then close (the frame
                // could not be correlated to a request id).
                let response = Response::error(
                    "",
                    &material.instance_id,
                    ErrorCode::Unsupported,
                    "malformed request frame".to_string(),
                );
                if let Ok(frame) = response_frame(&response) {
                    if ramshaper_windows::pipe::write_frame(&instance, &frame).is_ok() {
                        // Ensure the peer received the frame before the
                        // disconnect discards the buffer.
                        let _ = ramshaper_windows::pipe::flush(&instance);
                    }
                }
                break;
            }
        };
        let subscribing = envelope.command == "SubscribeSnapshots"
            && envelope.protocol_version == PROTOCOL_VERSION;

        // Dispatch one-shot commands with the current shared state.
        let response = {
            let latest_guard = shared.latest.lock().expect("latest lock");
            let latest = latest_guard.as_ref();
            // On-demand process collection (RS-011): exactly one fresh scan
            // per GetProcesses request. The version check still happens in
            // dispatch first, so a wrong-version request never triggers a scan.
            enum Scanned {
                NotRequested,
                Done(Result<serde_json::Value, String>),
            }
            let scanned = if envelope.command == ramshaper_core::ipc::COMMAND_GET_PROCESSES
                && envelope.protocol_version == PROTOCOL_VERSION
            {
                Scanned::Done(process_provider.scan())
            } else {
                Scanned::NotRequested
            };
            let processes = match &scanned {
                Scanned::Done(Ok(inventory)) => ProcessesContext::Ready(inventory),
                Scanned::Done(Err(reason)) => ProcessesContext::Failed(reason),
                Scanned::NotRequested => ProcessesContext::NotSupported,
            };
            let context = DispatchContext {
                instance_id: &material.instance_id,
                capabilities: material.capabilities.clone(),
                latest,
                max_age_ms: shared.interval_ms.saturating_mul(2),
                now_monotonic_ms: ramshaper_core::SystemClock.monotonic_ms(),
                processes,
                config: material.config_summary.clone(),
                policy: shared.policy.as_deref(),
            };
            dispatch(&envelope, &context)
        };
        let frame = match response_frame(&response) {
            Ok(frame) => frame,
            Err(_) => break,
        };
        if ramshaper_windows::pipe::write_frame(&instance, &frame).is_err() {
            break;
        }
        if subscribing {
            stream_snapshots(
                &instance,
                &material.instance_id,
                &shared,
                &stop,
                subscriber_id,
            );
            break; // subscription owns the connection until it ends
        }
    }
    ramshaper_windows::pipe::disconnect(&instance);
}

/// Stream snapshot deliveries to a subscriber until the peer disconnects or
/// the agent stops. Bounded queue with gap markers (contract 14 §3).
#[cfg(windows)]
fn stream_snapshots(
    instance: &ramshaper_windows::handle::OwnedHandle,
    instance_id: &str,
    shared: &Shared,
    stop: &ramshaper_windows::event::ManualResetEvent,
    subscriber_id: u64,
) {
    {
        let mut subscribers = shared.subscribers.lock().expect("subscribers lock");
        subscribers.push((subscriber_id, SubscriberQueue::new()));
    }
    loop {
        let delivery = {
            let mut subscribers = shared.subscribers.lock().expect("subscribers lock");
            let queue = subscribers
                .iter_mut()
                .find(|(id, _)| *id == subscriber_id)
                .expect("subscriber registered above");
            match queue.1.pop() {
                Some(delivery) => delivery,
                None => {
                    // Nothing pending: wait for a publish or the next tick,
                    // then re-check both the queue and the stop signal.
                    let _ = shared
                        .notify
                        .wait_timeout(subscribers, std::time::Duration::from_millis(250))
                        .expect("condvar");
                    if stop_is_set(stop) {
                        let _ = remove_subscriber(shared, subscriber_id);
                        return;
                    }
                    continue;
                }
            }
        };
        let response = Response::ok(
            "snapshot",
            instance_id,
            serde_json::to_value(&delivery).unwrap_or(serde_json::Value::Null),
        );
        match response_frame(&response) {
            Ok(frame) => {
                if ramshaper_windows::pipe::write_frame(instance, &frame).is_err() {
                    break; // peer gone
                }
            }
            Err(_) => break,
        }
        if stop_is_set(stop) {
            // Deliver what was queued, then end the stream.
            break;
        }
    }
    let _ = remove_subscriber(shared, subscriber_id);
}

#[cfg(windows)]
fn stop_is_set(stop: &ramshaper_windows::event::ManualResetEvent) -> bool {
    stop.wait(0) == ramshaper_core::watch::WaitOutcome::Signaled
}

#[cfg(windows)]
fn remove_subscriber(shared: &Shared, subscriber_id: u64) -> Option<()> {
    let mut subscribers = shared.subscribers.lock().expect("subscribers lock");
    subscribers.retain(|(id, _)| *id != subscriber_id);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic scan fixture (mirrors the core process-test helper; the
    /// values are synthetic, never real machine data).
    fn scan_row(pid: u32, filetime: u64, cpu_100ns: u64) -> ramshaper_core::process::ProcessScan {
        use ramshaper_core::process::ProcessCountersRaw;
        use ramshaper_core::ProcessIdentity;
        ramshaper_core::process::ProcessScan {
            processes: vec![ProcessCountersRaw {
                identity: ProcessIdentity::new(pid, filetime),
                name: "app.exe".to_string(),
                parent_pid: 4,
                working_set_bytes: Ok(10 * 1024 * 1024),
                private_commit_bytes: Ok(20 * 1024 * 1024),
                cpu_kernel_100ns: Ok(cpu_100ns / 2),
                cpu_user_100ns: Ok(cpu_100ns - cpu_100ns / 2),
                io_read_bytes: Ok(4096),
                io_write_bytes: Ok(2048),
            }],
            truncated: false,
            scan_failure: None,
            logical_cpu_count: 8,
        }
    }

    #[test]
    fn process_scan_state_warms_then_rates_span_the_real_gap() {
        use ramshaper_core::status::MetricStatus;
        let mut state = ProcessScanState::default();
        // First request: baselines established, elapsed 0 (warming rates).
        let (outcome, elapsed) = state.advance(&scan_row(7, 1000, 10_000_000), 5_000);
        assert_eq!(elapsed, 0);
        assert_eq!(
            outcome.entries[0].cpu_percent.status,
            MetricStatus::WarmingUp
        );
        // Second request 2 s later, same identity, +5,000,000 × 100 ns CPU:
        // 0.5 s of CPU over 2 s on 8 CPUs = 3.125%.
        let (outcome, elapsed) = state.advance(&scan_row(7, 1000, 15_000_000), 7_000);
        assert_eq!(elapsed, 2_000);
        assert_eq!(outcome.entries[0].cpu_percent.status, MetricStatus::Ok);
        let rate = outcome.entries[0].cpu_percent.rate.expect("rate");
        assert!((rate - 3.125).abs() < 1e-9, "rate {rate}");
        assert_eq!(outcome.churn.appeared, 0);
        assert_eq!(outcome.churn.vanished, 0);
    }

    #[test]
    fn subscriber_queue_is_bounded_with_gap_markers() {
        let mut queue = SubscriberQueue::new();
        for seq in 1..=5 {
            queue.push(seq, serde_json::json!({ "seq": seq }));
        }
        // Cap is 2: only the newest two remain; three were dropped.
        let first = queue.pop().expect("delivery");
        assert_eq!(first.snapshot["seq"], 4);
        assert_eq!(first.gap_dropped, 3);
        let second = queue.pop().expect("delivery");
        assert_eq!(second.snapshot["seq"], 5);
        assert_eq!(second.gap_dropped, 0);
        assert!(queue.pop().is_none());
    }

    #[test]
    fn peer_validation_decides_by_session_and_user() {
        let sid = "S-1-5-21-x";
        assert!(matches!(
            validate_peer(3, Some(sid), 3, sid),
            PeerVerdict::Accepted
        ));
        assert!(matches!(
            validate_peer(4, Some(sid), 3, sid),
            PeerVerdict::Rejected(_)
        ));
        assert!(matches!(
            validate_peer(3, Some("S-1-5-21-other"), 3, sid),
            PeerVerdict::Rejected(_)
        ));
        assert!(matches!(
            validate_peer(3, None, 3, sid),
            PeerVerdict::Rejected(_)
        ));
    }
}
