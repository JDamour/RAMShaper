//! Windows integration tests (real OS calls, standard user).
//!
//! These run against the actual OS on the current machine — they prove the
//! FFI works on this build, not just in mocks. Environment-sensitive checks
//! use robust bounds; exact numbers are recorded in the G0 evidence file.

#![cfg(windows)]

use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ramshaper_core::identity::{IdentityVerification, ProcessIdentity};
use ramshaper_core::snapshot::QueryFailure;
use ramshaper_core::watch::{run_watch, WatchConfig};
use ramshaper_platform::ReadOnlySystemMonitor as _;
use ramshaper_windows::agent_instance_id;
use ramshaper_windows::console::ConsoleCancel;
use ramshaper_windows::identity::{
    current_process_identity, resolve_identity, session_id, verify_identity,
};
use ramshaper_windows::memnotify::MemoryResourceNotification;
use ramshaper_windows::osinfo::collect_os_info;
use ramshaper_windows::perfinfo::query_global_memory;
use ramshaper_windows::WindowsSystemMonitor;

/// ConsoleCancel uses a process-global handler slot; serialize tests that
/// install it.
static CONSOLE_LOCK: Mutex<()> = Mutex::new(());

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Spawn a short-lived `cmd` fixture we may observe and then terminate.
fn spawn_pinging_cmd() -> Child {
    Command::new("cmd")
        .args(["/c", "ping -n 20 127.0.0.1 > nul"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .expect("spawn cmd fixture")
}

/// Trigger `cancel` from another thread after `delay`. Returns a guard whose
/// drop joins the trigger thread, keeping the borrow sound.
fn trigger_after(cancel: Arc<ConsoleCancel>, delay: Duration) -> impl Drop {
    let join = std::thread::spawn(move || {
        std::thread::sleep(delay);
        cancel.trigger();
    });
    WaitGuard { join: Some(join) }
}

struct WaitGuard {
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[test]
fn performance_info_returns_sane_page_counters() {
    let raw = query_global_memory().expect("GetPerformanceInfo succeeds");
    assert!(raw.page_size_bytes.is_power_of_two(), "page size {raw:?}");
    assert!(
        (4096..=65_536).contains(&raw.page_size_bytes),
        "page size {raw:?}"
    );
    assert!(raw.physical_total_pages > 0);
    assert!(
        raw.physical_available_pages <= raw.physical_total_pages,
        "{raw:?}"
    );
    assert!(raw.commit_total_pages <= raw.commit_limit_pages, "{raw:?}");
}

#[test]
fn self_identity_is_stable_and_exact() {
    let first = current_process_identity().expect("self identity");
    let second = current_process_identity().expect("self identity again");
    assert_eq!(
        first, second,
        "creation FILETIME must not drift for a live process"
    );
    assert!(first.creation_filetime_100ns > 0);
    assert_eq!(verify_identity(&first), IdentityVerification::Matches);
    // Cross-check through the PID-based path (a different handle, same result).
    let via_pid = resolve_identity(first.pid).expect("resolve by pid");
    assert_eq!(via_pid, first);
}

#[test]
fn session_context_is_queryable_and_separate_from_identity() {
    let id = current_process_identity().expect("self identity");
    let session = session_id(id.pid).expect("session id");
    // Documented separately from identity: the ProcessIdentity type carries
    // no session field; context is its own query.
    assert!(session < u32::MAX);
}

#[test]
fn exited_process_identity_is_unreachable() {
    let mut child = spawn_pinging_cmd();
    let identity = resolve_identity(child.id()).expect("resolve child identity");
    assert!(identity.creation_filetime_100ns > 0);
    assert_eq!(
        verify_identity(&identity),
        IdentityVerification::Matches,
        "child must verify while alive"
    );

    // Terminate our fixture child and wait for full exit.
    child.kill().expect("kill fixture child");
    child.wait().expect("reap fixture child");

    // The PID may briefly remain resolvable; poll until it is not, then
    // require an unreachable/exited verdict — never a false Matches.
    let mut verification = verify_identity(&identity);
    for _ in 0..50 {
        if !matches!(verification, IdentityVerification::Matches) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        verification = verify_identity(&identity);
    }
    match verification {
        IdentityVerification::Unreachable {
            reason: QueryFailure::Exited { .. },
        } => {}
        other => panic!("expected Exited after process death, got {other:?}"),
    }
}

#[test]
fn mismatched_creation_marker_detects_reuse() {
    // Fake reuse trace on a live process (AT-03): the same PID with a wrong
    // creation FILETIME must NOT verify — exactly what PID reuse looks like
    // to delayed operations.
    let actual = current_process_identity().expect("self identity");
    let stale = ProcessIdentity::new(actual.pid, actual.creation_filetime_100ns + 1);
    assert_eq!(
        verify_identity(&stale),
        IdentityVerification::Mismatch { observed: actual },
        "a reused PID must never verify, and the replacement must be surfaced"
    );
}

#[test]
fn memory_notification_object_queries_state() {
    let notification = MemoryResourceNotification::create_low().expect("create notification");
    // Either signal value is valid; the typed query must succeed, repeatedly.
    let _first = notification.query().expect("first query");
    let _second = notification.query().expect("second query");
}

#[test]
fn thousand_start_stop_cycles_return_handles_to_baseline() {
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};

    // SAFETY: pseudo-handle of the current process; must not be closed.
    let process = unsafe { GetCurrentProcess() };
    let mut before: u32 = 0;
    // SAFETY: valid out pointer.
    unsafe { GetProcessHandleCount(process, &mut before) }.expect("handle count before");

    for _ in 0..1000 {
        let notification =
            MemoryResourceNotification::create_low().expect("create notification in cycle");
        let _ = notification.query().expect("query in cycle");
        drop(notification); // RAII close
    }

    let mut after: u32 = 0;
    // SAFETY: valid out pointer.
    unsafe { GetProcessHandleCount(process, &mut after) }.expect("handle count after");
    // Handles must return to baseline (small tolerance for incidental runtime
    // activity in the same process during parallel tests).
    let delta = after.abs_diff(before);
    assert!(
        delta <= 8,
        "handle count before={before} after={after} (delta {delta})"
    );
    println!("1000-cycle handle delta: {delta} (before={before}, after={after})");
}

#[test]
fn watch_is_cancellable_through_console_event() {
    let _guard = CONSOLE_LOCK.lock().expect("console lock");
    let cancel = Arc::new(ConsoleCancel::install().expect("install"));
    let monitor = WindowsSystemMonitor::new();

    let _trigger = trigger_after(Arc::clone(&cancel), Duration::from_millis(400));

    let config = WatchConfig::new(200).expect("interval");
    let clock = ramshaper_core::SystemClock;
    let started = Instant::now();
    let mut iterations = 0u64;
    let summary = run_watch(&config, &clock, &*cancel, |_| iterations += 1);
    let elapsed = started.elapsed();
    assert!(
        matches!(
            summary.termination,
            ramshaper_core::watch::WatchTermination::Cancelled
        ),
        "{summary:?}"
    );
    assert!((1..=10).contains(&iterations), "iterations {iterations}");
    assert!(
        elapsed < Duration::from_secs(10),
        "watch returned in {elapsed:?}"
    );
    // The monitor stays healthy after cancellation (handles intact).
    let sample = monitor.collect_system();
    assert!(
        sample.memory.is_ok(),
        "monitor still healthy: {:?}",
        sample.memory
    );
    println!("watch cancelled after {elapsed:?} with {iterations} sample(s)");
}

#[test]
fn watch_on_one_second_cadence_emits_no_burst() {
    // Rate-limited design: whatever the low-memory condition does, the loop
    // cadence is fixed by the interval. Real clock, real OS: ~2.3 s at 1 s
    // interval => 2..=4 samples. A spin loop would emit far more.
    let _guard = CONSOLE_LOCK.lock().expect("console lock");
    let cancel = Arc::new(ConsoleCancel::install().expect("install"));
    let _trigger = trigger_after(Arc::clone(&cancel), Duration::from_millis(2_300));

    let config = WatchConfig::new(1000).expect("interval");
    let clock = ramshaper_core::SystemClock;
    let started = Instant::now();
    let summary = run_watch(&config, &clock, &*cancel, |_| {});
    let elapsed = started.elapsed();
    assert!(
        summary.iterations >= 2 && summary.iterations <= 4,
        "iterations {} in {elapsed:?}",
        summary.iterations
    );
    assert!(elapsed >= Duration::from_millis(2_000));
    println!(
        "1s-cadence watch: {} samples in {elapsed:?}",
        summary.iterations
    );
}

#[test]
fn os_info_reports_windows_build() {
    let os = collect_os_info();
    assert!(os.build.is_some(), "RtlGetVersion failed: {os:?}");
    assert!(os.build.expect("checked") >= 10_000, "{os:?}");
    assert_eq!(os.arch.as_deref(), Some("x86_64"), "{os:?}");
    assert!(
        os.product_name
            .as_deref()
            .is_some_and(|n| n.contains("Windows")),
        "{os:?}"
    );
}

#[test]
fn monitor_collects_healthy_sample() {
    let monitor = WindowsSystemMonitor::new();
    let sample = monitor.collect_system();
    let raw = sample
        .memory
        .expect("memory counters on a real desktop system");
    assert!(raw.physical_total_pages > 0);
    let low = sample.low_memory.expect("low-memory query");
    println!("low-memory state on this machine: {low:?}");
    let caps = monitor.capabilities();
    assert!(caps.is_supported(ramshaper_platform::CAPABILITY_SYSTEM_MEMORY_COUNTERS));
    assert!(caps.is_supported(ramshaper_platform::CAPABILITY_LOW_MEMORY_NOTIFICATION));
    assert!(caps.is_supported(ramshaper_platform::CAPABILITY_PROCESS_IDENTITY));
    let instance = agent_instance_id();
    assert!(instance.starts_with("cli-"), "{instance}");
}

#[test]
fn process_scan_enumerates_and_queries_real_processes() {
    use ramshaper_platform::ReadOnlyProcessMonitor as _;
    let monitor = WindowsSystemMonitor::new();
    let scan = monitor.collect_processes();
    assert!(scan.scan_failure.is_none(), "{:?}", scan.scan_failure);
    assert!(!scan.truncated);
    assert!(scan.logical_cpu_count > 0);
    // A real interactive machine has dozens of processes; be conservative.
    assert!(
        scan.processes.len() > 20,
        "only {} processes",
        scan.processes.len()
    );
    // Our own process must be present with a real identity and counters.
    let self_id = ramshaper_windows::identity::current_process_identity().expect("self identity");
    let row = scan
        .processes
        .iter()
        .find(|raw| raw.identity == self_id)
        .expect("self present in scan");
    assert!(row.working_set_bytes.clone().expect("ws ok") > 0);
    assert!(row.private_commit_bytes.clone().expect("private ok") > 0);
    assert!(row.cpu_kernel_100ns.is_ok() && row.cpu_user_100ns.is_ok());
    assert!(row.io_read_bytes.is_ok() && row.io_write_bytes.is_ok());
    // Names are labels; every row has one.
    assert!(scan.processes.iter().all(|raw| !raw.name.is_empty()));
}

#[test]
fn process_rates_over_two_real_scans_are_sane() {
    use ramshaper_platform::ReadOnlyProcessMonitor as _;
    let monitor = WindowsSystemMonitor::new();
    let first = monitor.collect_processes();
    std::thread::sleep(Duration::from_millis(400));
    let second = monitor.collect_processes();
    let cpus = second.logical_cpu_count;
    let mut baselines = ramshaper_core::process::RateBaselines::new();
    let _ = baselines.advance(&first, 0);
    let outcome = baselines.advance(&second, 400);
    // Some churn is normal on a live machine; every rate that exists is
    // finite and nonnegative by construction — assert the invariants.
    for entry in &outcome.entries {
        if let Some(rate) = entry.cpu_percent.rate {
            assert!((0.0..=100.0).contains(&rate), "cpu {rate}");
            assert!(rate.is_finite());
        }
        if let Some(rate) = entry.io_read.rate {
            assert!(rate.is_finite() && rate >= 0.0);
        }
    }
    // Identities are unique within one scan.
    let mut ids: Vec<_> = outcome.entries.iter().map(|e| e.identity).collect();
    ids.sort_by_key(|id| (id.pid, id.creation_filetime_100ns));
    let len_before = ids.len();
    ids.dedup_by_key(|id| (id.pid, id.creation_filetime_100ns));
    assert_eq!(
        ids.len(),
        len_before,
        "duplicate identities in one scan (cpus {cpus})"
    );
}

#[test]
fn logical_cpu_count_matches_environment() {
    let cpus = ramshaper_windows::processes::logical_cpu_count();
    assert!(cpus > 0 && cpus <= 256);
}

#[test]
fn current_process_memory_bytes_reports_something() {
    let (working_set, private) =
        ramshaper_windows::processes::current_process_memory_bytes().expect("self memory query");
    assert!(working_set > 0);
    assert!(private > 0);
}
