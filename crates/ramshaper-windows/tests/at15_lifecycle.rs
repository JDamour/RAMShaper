//! AT-15 launcher-crash lifecycle trace (in-process portion; doc 15 AT-15;
//! doc 04 §D; RS-020/021/023 surfaces).
//!
//! AT-15: "Given supported and nested-job fixtures, when managed creation
//! fails or the launcher crashes at each stage, then no RAMShaper-created
//! suspended orphan, forced kill or automatic duplicate launch occurs."
//! The crash stages discretize into three:
//!
//! 1. **Before creation** — traced live here: a launcher force-terminated
//!    before creating anything leaves nothing (no spawn call, no process).
//! 2. **Mid-creation** — NOT externally interruptible by construction:
//!    with the creation-time `PROC_THREAD_ATTRIBUTE_JOB_LIST` (RS-020)
//!    membership and birth are one syscall, so no launcher crash can
//!    observe a half-created child. That absence IS the no-suspended-
//!    orphan guarantee (the superseded suspended-create/assign design had
//!    the window; this mechanism does not).
//! 3. **After creation** — traced live here: a launcher force-terminated
//!    right after a managed creation leaves the child alive, still
//!    jobbed, and completing unaided — with exactly one child, ever.
//!
//! The "launcher" is a re-exec of this very test binary (the standard
//! self-re-exec pattern): the helper run takes the env-var branch, creates
//! the managed child, reports its PID on stdout, and hangs; the parent
//! force-terminates it with `taskkill /F` (the same OS-level fact as a
//! crash — every handle closed by the OS — without an interactive WER
//! dialog, which a helper-side `abort()` could pop on this machine).
//!
//! The full instrumented-rig staged-failure matrix (a debugger-injected
//! crash at arbitrary instruction boundaries) remains AT-15 rig work.

use std::io::BufRead as _;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use ramshaper_windows::job::observe_membership;
use ramshaper_windows::job::AdvisoryJob;
use ramshaper_windows::job::ManagedChild;

/// Selects the helper branch of a re-exec'd test run.
const HELPER_ENV: &str = "RAMSHAPER_AT15_HELPER";
/// The line prefix the helper prints once the managed child exists.
const CHILD_PID_PREFIX: &str = "AT15_CHILD_PID:";
/// A quiet child the launcher creates and then abandons.
const CHILD_COMMAND: &str = "ping -n 6 127.0.0.1";
/// The child outlives its launcher by design; this is the unaided-
/// completion deadline (ping -n 6 needs ~5 s).
const CHILD_COMPLETION_DEADLINE_SECS: u64 = 20;

#[test]
fn launcher_force_terminated_after_managed_creation_leaves_the_child_unaided() {
    if std::env::var(HELPER_ENV).is_ok() {
        helper_spawn_then_hang();
    }

    let mut helper = spawn_helper("spawn-then-hang");

    // Stream the helper's stdout: the launcher reports exactly one child
    // and is force-terminated the moment the first report arrives (the
    // collector performs the kill). Exactly one pid ever reported = the
    // no-automatic-duplicate-launch fact, traced across a real process
    // boundary rather than only pinned by the consumed-consent token.
    let child_pids =
        collect_child_pids(&mut helper).expect("the helper reported its managed child");
    assert_eq!(
        child_pids.len(),
        1,
        "exactly one child must ever be created: {child_pids:?}"
    );
    let child_pid = child_pids[0];

    // The crash already happened (on first report); reap it. Every handle
    // the launcher held (job + child) is closed by the OS.
    let status = helper.wait().expect("wait for the dead launcher");
    assert!(
        !status.success(),
        "a forceful termination is not a clean exit: {status:?}"
    );

    // The child is still alive right after its launcher died.
    let child = open_for_wait(child_pid).expect("the child survives the launcher's death");

    // ...and it is STILL in a job: the job object survives while a member
    // exists, so the association is not lost when the monitoring handles
    // close (the non-detachable fact, now across a launcher crash).
    let observed = observe_membership(child_pid, None).expect("observe the orphaned child");
    assert!(
        observed.in_any_job,
        "membership must persist after the launcher's death: {observed:?}"
    );

    // No suspended orphan: the abandoned child completes unaided (a
    // suspended child would never signal, and kill-on-close would have
    // ended it the instant the launcher died).
    let started = Instant::now();
    loop {
        use windows::Win32::Foundation::WAIT_FAILED;
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        // SAFETY: opened handle; a zero-timeout wait does not block.
        let event = unsafe { windows::Win32::System::Threading::WaitForSingleObject(child, 0) };
        if event == WAIT_OBJECT_0 {
            break;
        }
        assert_ne!(
            event, WAIT_FAILED,
            "the child handle became invalid before completion"
        );
        assert!(
            started.elapsed() < Duration::from_secs(CHILD_COMPLETION_DEADLINE_SECS),
            "the abandoned child never completed (suspended orphan?)"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // SAFETY: opened handle, done with it.
    let _ = unsafe { windows::Win32::Foundation::CloseHandle(child) };

    stop_pid(child_pid);
}

#[test]
fn launcher_force_terminated_before_creation_leaves_nothing() {
    if std::env::var(HELPER_ENV).is_ok() {
        helper_hang_only();
    }

    let mut helper = spawn_helper("hang-only");
    // The before-creation stage: kill the launcher before it could ever
    // create anything (this helper mode creates nothing at all).
    std::thread::sleep(Duration::from_millis(500));
    let _ = Command::new("taskkill")
        .args(["/PID", &helper.id().to_string(), "/F"])
        .status();
    let status = helper.wait().expect("wait for the dead launcher");
    assert!(!status.success(), "terminated, not exited cleanly");

    // Nothing it reported can exist: drain to EOF and assert the empty
    // report — no child pid line ever crossed the pipe.
    let child_pids = collect_child_pids(&mut helper).expect("the helper's stdout drained to EOF");
    assert!(
        child_pids.is_empty(),
        "a launcher killed before creation must report no children: {child_pids:?}"
    );
}

/// Re-exec this test binary as the "launcher" helper: libtest runs the
/// matching test, which takes the env-var branch. `--nocapture` is
/// required so the pid line reaches the pipe directly (libtest captures
/// test stdout by default and would only replay it after the kill).
fn spawn_helper(mode: &str) -> std::process::Child {
    let exe = std::env::current_exe().expect("current test binary");
    let test_name = if mode == "spawn-then-hang" {
        "launcher_force_terminated_after_managed_creation_leaves_the_child_unaided"
    } else {
        "launcher_force_terminated_before_creation_leaves_nothing"
    };
    Command::new(&exe)
        .args([test_name, "--exact", "--test-threads", "1", "--nocapture"])
        .env(HELPER_ENV, mode)
        .stdout(Stdio::piped())
        .spawn()
        .expect("re-exec this test binary as the AT-15 helper launcher")
}

/// One stdout event from the helper. Lines arrive as they are written
/// (streaming — the helper hangs by design, so waiting for EOF before
/// acting would deadlock: EOF requires the kill, the kill requires the
/// pid line).
enum HelperEvent {
    Line(String),
    Eof,
}

/// Start streaming the helper's stdout lines in the background.
fn spawn_reader(helper: &mut std::process::Child) -> mpsc::Receiver<HelperEvent> {
    let stdout = helper.stdout.take().expect("piped helper stdout");
    let (tx, rx) = mpsc::channel::<HelperEvent>();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            match line {
                Ok(text) => {
                    if tx.send(HelperEvent::Line(text)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(HelperEvent::Eof);
    });
    rx
}

/// Drain helper stdout until EOF (bounded by the watchdog), killing the
/// helper as soon as the first child-pid line arrives (so the crash lands
/// right after the creation, not at some later arbitrary time). Returns
/// every child pid the helper ever reported.
fn collect_child_pids(helper: &mut std::process::Child) -> Result<Vec<u32>, String> {
    let rx = spawn_reader(helper);
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut killed = false;
    let mut pids = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(HelperEvent::Line(text)) => {
                // The pid line may be CONCATENATED onto the harness's
                // partial "test <name> ... " line (libtest prints that
                // without a newline), so match the prefix anywhere.
                if let Some(idx) = text.find(CHILD_PID_PREFIX) {
                    if let Ok(pid) = text[idx + CHILD_PID_PREFIX.len()..].trim().parse::<u32>() {
                        if !killed {
                            // First report: the creation happened — crash
                            // the launcher NOW (the experiment's timing).
                            killed = true;
                            let _ = Command::new("taskkill")
                                .args(["/PID", &helper.id().to_string(), "/F"])
                                .status();
                        }
                        pids.push(pid);
                    }
                }
            }
            Ok(HelperEvent::Eof) => return Ok(pids),
            Err(_) => {
                if !killed {
                    let _ = Command::new("taskkill")
                        .args(["/PID", &helper.id().to_string(), "/F"])
                        .status();
                    let _ = helper.wait();
                }
                return Err("the helper never drained its stdout (watchdog fired)".to_string());
            }
        }
    }
}

/// The helper branch: create the fresh advisory job + managed child,
/// report the child's PID, then hang until force-terminated. The launcher
/// deliberately does NOT wait for the child or clean anything up — the
/// crash is the experiment.
fn helper_spawn_then_hang() -> ! {
    let job = AdvisoryJob::create().expect("helper: create advisory job");
    let child = ManagedChild::spawn(&job, CHILD_COMMAND).expect("helper: managed spawn");
    println!("{CHILD_PID_PREFIX}{}", child.pid());
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// The helper branch that never creates anything.
fn helper_hang_only() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// A wait-capable handle to a process we hold no handle to (the child of
/// the dead launcher): query-only rights plus SYNCHRONIZE.
fn open_for_wait(
    pid: u32,
) -> Result<windows::Win32::Foundation::HANDLE, ramshaper_windows::job::JobError> {
    // SAFETY: numeric PID; synchronize+query-limited rights, adopted by
    // the caller for an explicit CloseHandle.
    let handle = unsafe {
        windows::Win32::System::Threading::OpenProcess(
            windows::Win32::System::Threading::PROCESS_SYNCHRONIZE
                | windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        )
    }
    .map_err(|e| ramshaper_windows::job::JobError::Os {
        code: e.code().0 as u32,
        function: "OpenProcess",
    })?;
    Ok(handle)
}

/// Best-effort cleanup (the tests' own children; production has no kill).
fn stop_pid(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status();
}
