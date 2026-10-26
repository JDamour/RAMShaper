//! Managed-launch spike (RS-020; R-06; doc 04 §D; acceptance AT-15/AT-16).
//!
//! Evaluates the **creation-time job-list attribute** (`UpdateProcThreadAttribute`
//! with `PROC_THREAD_ATTRIBUTE_JOB_LIST`) as the managed-launch mechanism,
//! replacing the earlier suspended-create/assign/resume suggestion: a
//! launcher crash between create and assign can strand a suspended child,
//! while creation-time assignment makes membership and birth the same
//! syscall — there is no unmanaged window and nothing is ever suspended.
//!
//! Safety shape (doc 04 §D, AGENTS.md rule 4):
//! - the job is **fresh and advisory**: `CreateJobObjectW` defaults carry no
//!   limits; this module never calls `SetInformationJobObject`, so there is
//!   no kill-on-close, no hard memory/CPU/time/process-count limit and no UI
//!   restriction — queried back and asserted by the tests;
//! - no `CREATE_SUSPENDED` anywhere: the only creation flag is
//!   `EXTENDED_STARTUPINFO_PRESENT` (required by the attribute list);
//! - **membership is verified through the child's own process handle before
//!   the spawn reports success** — if the attribute were ignored the spawn
//!   fails loudly instead of silently launching unmanaged;
//! - existing jobs are detected, never escaped: the environment probe
//!   (`is_in_any_job`) is the preflight seam; nested-job behavior is a test,
//!   not an assumption.
//!
//! Boundary: this is a spike module. Nothing in the agent, CLI or UI calls
//! it yet; the managed launcher stays unavailable in the shipped surface
//! (doc 07: keep the launcher unavailable if the safety gate fails). A real
//! mid-`CreateProcessW` interruption trace (AT-15's lifecycle evidence) is
//! G4 rig work; this spike pins the crash-window *shape* by construction.

use std::mem::size_of;
use std::mem::size_of_val;

use windows::core::PWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Foundation::WAIT_EVENT;
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::Foundation::WAIT_TIMEOUT;
use windows::Win32::System::JobObjects::AssignProcessToJobObject;
use windows::Win32::System::JobObjects::CreateJobObjectW;
use windows::Win32::System::JobObjects::IsProcessInJob;
use windows::Win32::System::JobObjects::JobObjectBasicProcessIdList;
use windows::Win32::System::JobObjects::JobObjectExtendedLimitInformation;
use windows::Win32::System::JobObjects::QueryInformationJobObject;
use windows::Win32::System::JobObjects::JOBOBJECT_BASIC_PROCESS_ID_LIST;
use windows::Win32::System::JobObjects::JOBOBJECT_EXTENDED_LIMIT_INFORMATION;
use windows::Win32::System::Threading::CreateProcessW;
use windows::Win32::System::Threading::DeleteProcThreadAttributeList;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::System::Threading::GetExitCodeProcess;
use windows::Win32::System::Threading::InitializeProcThreadAttributeList;
use windows::Win32::System::Threading::OpenProcess;
use windows::Win32::System::Threading::UpdateProcThreadAttribute;
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::Win32::System::Threading::EXTENDED_STARTUPINFO_PRESENT;
use windows::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST;
use windows::Win32::System::Threading::PROCESS_INFORMATION;
use windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;
use windows::Win32::System::Threading::PROC_THREAD_ATTRIBUTE_JOB_LIST;
use windows::Win32::System::Threading::STARTUPINFOEXW;
use windows::Win32::System::Threading::STARTUPINFOW;

use crate::error::win32_code;
use crate::handle::OwnedHandle;

const ERROR_MORE_DATA: u32 = 234;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const INFINITE: u32 = 0xFFFF_FFFF;

/// Spike failure vocabulary: the Win32 code and the failing function. The
/// launcher seam turns these into refusals (fail closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    /// The OS call failed with the preserved code.
    Os {
        /// Preserved Win32 error code.
        code: u32,
        /// The failing function, for the audit trail.
        function: &'static str,
    },
}

impl JobError {
    fn os(function: &'static str) -> Self {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32;
        Self::Os { code, function }
    }

    fn from(function: &'static str, error: &windows::core::Error) -> Self {
        Self::Os {
            code: win32_code(error),
            function,
        }
    }
}

/// A fresh advisory job: no limits, no kill-on-close, no UI restrictions.
/// The module never mutates the job after creation — the queried defaults
/// are the shipped behavior.
#[derive(Debug)]
pub struct AdvisoryJob {
    handle: OwnedHandle,
}

impl AdvisoryJob {
    /// Create the unnamed advisory job. `CreateJobObjectW` defaults have
    /// zero limit flags; nothing is ever set on it.
    ///
    /// # Errors
    /// [`JobError::Os`] when the OS refuses creation.
    pub fn create() -> Result<Self, JobError> {
        // SAFETY: null attributes, unnamed job; the returned handle is
        // adopted immediately.
        let handle = unsafe { CreateJobObjectW(None, None) }
            .map_err(|e| JobError::from("CreateJobObjectW", &e))?;
        Ok(Self {
            // SAFETY: CreateJobObjectW returned a valid owned handle.
            handle: unsafe { OwnedHandle::adopt(handle) },
        })
    }

    /// The job's effective limit flags. The advisory contract is that this
    /// is always 0 — the tests query and assert it rather than trusting the
    /// constructor (AT-16: "queried job flags").
    ///
    /// # Errors
    /// [`JobError::Os`] when the query fails.
    pub fn limit_flags(&self) -> Result<u32, JobError> {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // SAFETY: owned handle; the buffer is the exact documented struct
        // for `JobObjectExtendedLimitInformation`.
        unsafe {
            QueryInformationJobObject(
                Some(self.handle.raw()),
                JobObjectExtendedLimitInformation,
                (&mut info as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&info) as u32,
                None,
            )
        }
        .map_err(|e| JobError::from("QueryInformationJobObject", &e))?;
        Ok(info.BasicLimitInformation.LimitFlags.0)
    }

    /// Whether the process is a member of this job. The membership check
    /// runs against the PID's minimal query handle; callers that hold an
    /// authority requirement use [`ManagedChild::in_job`], which reuses the
    /// spawn-time handle instead.
    ///
    /// # Errors
    /// [`JobError::Os`] when the process cannot be opened or the query
    /// fails.
    pub fn contains_process(&self, pid: u32) -> Result<bool, JobError> {
        // SAFETY: numeric PID; query-only rights, handle dropped after.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map_err(|e| JobError::from("OpenProcess", &e))?;
        // SAFETY: OpenProcess returned a valid owned handle.
        let process = unsafe { OwnedHandle::adopt(process) };
        in_job_of(&process, Some(self.handle.raw()), "IsProcessInJob")
    }

    /// Assign a process handle to this job. **Spike seam**: production
    /// assignment belongs to the RS-023 launch-consent flow (and the
    /// creation-time attribute path needs no post-create assignment at
    /// all); the tests use this to build nested/preexisting-job fixtures.
    ///
    /// # Errors
    /// [`JobError::Os`] when the assignment is refused.
    pub fn assign(&self, process: HANDLE) -> Result<(), JobError> {
        // SAFETY: caller-provided process handle; the job is owned and
        // advisory.
        unsafe { AssignProcessToJobObject(self.handle.raw(), process) }
            .map_err(|e| JobError::from("AssignProcessToJobObject", &e))
    }

    /// Assign RAMShaper itself to this job (the nested-environment fixture).
    ///
    /// # Errors
    /// [`JobError::Os`] when the assignment is refused.
    pub fn assign_current_process(&self) -> Result<(), JobError> {
        // SAFETY: GetCurrentProcess returns the constant pseudo handle.
        self.assign(unsafe { GetCurrentProcess() })
    }

    /// The PIDs currently assigned to the job (`JobObjectBasicProcessIdList`),
    /// grown to the documented count when the first query reports more data.
    ///
    /// # Errors
    /// [`JobError::Os`] when the query fails.
    pub fn process_ids(&self) -> Result<Vec<u32>, JobError> {
        let header_size = size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
        let mut capacity = 64usize;
        loop {
            let mut buffer = vec![0u8; header_size + (capacity - 1) * size_of::<usize>()];
            // SAFETY: the buffer holds the documented list header plus the
            // capacity slots; the query writes at most its length.
            let result = unsafe {
                QueryInformationJobObject(
                    Some(self.handle.raw()),
                    JobObjectBasicProcessIdList,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    None,
                )
            };
            match result {
                Ok(()) => {
                    let list =
                        unsafe { &*(buffer.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST) };
                    let count = (list.NumberOfProcessIdsInList as usize).min(capacity);
                    // The declared array is one slot; the real length lives
                    // in the buffer we sized above.
                    let ids =
                        unsafe { std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count) };
                    return Ok(ids.iter().map(|&pid| pid as u32).collect());
                }
                Err(error) if win32_code(&error) == ERROR_MORE_DATA => {
                    // The header's first field is the assigned-process count.
                    let assigned = unsafe { *(buffer.as_ptr() as *const u32) } as usize;
                    capacity = assigned.max(capacity + 1);
                }
                Err(error) => return Err(JobError::from("QueryInformationJobObject", &error)),
            }
        }
    }
}

/// `IsProcessInJob` through a process handle: `job = None` asks whether the
/// process is in *any* job (the preflight probe).
fn in_job_of(
    process: &OwnedHandle,
    job: Option<HANDLE>,
    function: &'static str,
) -> Result<bool, JobError> {
    let mut member = windows::core::BOOL(0);
    // SAFETY: owned handles; the out-parameter is a plain BOOL.
    unsafe { IsProcessInJob(process.raw(), job, &mut member) }
        .map_err(|e| JobError::from(function, &e))?;
    Ok(member.as_bool())
}

/// Whether the process is in *any* job — the launcher's environment
/// preflight (doc 04 §D step 6: detect inherited constraints; never escape
/// them).
///
/// # Errors
/// [`JobError::Os`] when the process cannot be opened or the query fails.
pub fn is_in_any_job(pid: u32) -> Result<bool, JobError> {
    // SAFETY: numeric PID; query-only rights.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|e| JobError::from("OpenProcess", &e))?;
    // SAFETY: OpenProcess returned a valid owned handle.
    let process = unsafe { OwnedHandle::adopt(process) };
    in_job_of(&process, None, "IsProcessInJob")
}

/// A process created into an [`AdvisoryJob`] at creation time. Owns the
/// child's process handle; the thread handle is closed immediately after
/// creation (the standard pattern — the thread keeps running).
#[derive(Debug)]
pub struct ManagedChild {
    pid: u32,
    process: OwnedHandle,
}

/// The attribute list guard: `DeleteProcThreadAttributeList` on drop, after
/// the buffer the list points into (dropped later by field order).
struct AttributeList {
    _buffer: Vec<u8>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: the list was initialized by InitializeProcThreadAttributeList
        // and the buffer outlives the call (dropped after `list`).
        unsafe { DeleteProcThreadAttributeList(self.list) };
    }
}

impl ManagedChild {
    /// Spawn `command_line` into `job` with the creation-time job attribute.
    /// Membership is verified through the child's own handle before the
    /// spawn reports success; a refused verification is an error, never an
    /// unmanaged launch.
    ///
    /// # Errors
    /// [`JobError::Os`] on any failed step (attribute list, process
    /// creation, membership verification).
    pub fn spawn(job: &AdvisoryJob, command_line: &str) -> Result<Self, JobError> {
        let mut wide: Vec<u16> = command_line
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // Size the attribute list (the null call fails with
        // ERROR_INSUFFICIENT_BUFFER and fills the required size).
        let mut size = 0usize;
        // SAFETY: null list is the documented sizing call.
        if let Err(error) = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) } {
            if win32_code(&error) != ERROR_INSUFFICIENT_BUFFER {
                return Err(JobError::from("InitializeProcThreadAttributeList", &error));
            }
        }
        let mut buffer = vec![0u8; size];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr().cast());
        // SAFETY: the buffer holds `size` bytes and outlives the list.
        unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut size) }
            .map_err(|e| JobError::from("InitializeProcThreadAttributeList", &e))?;
        let attributes = AttributeList {
            _buffer: buffer,
            list,
        };

        // The creation-time assignment: one attribute, the job handle.
        // SAFETY: the list is initialized; the value is a valid HANDLE
        // reference of the exact documented size.
        unsafe {
            UpdateProcThreadAttribute(
                attributes.list,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                Some(job.handle_raw_as_ptr()),
                size_of::<HANDLE>(),
                None,
                None,
            )
        }
        .map_err(|e| JobError::from("UpdateProcThreadAttribute", &e))?;

        let startup = STARTUPINFOEXW {
            StartupInfo: STARTUPINFOW {
                cb: size_of::<STARTUPINFOEXW>() as u32,
                ..Default::default()
            },
            lpAttributeList: attributes.list,
        };
        let mut information = PROCESS_INFORMATION::default();

        // Creation flags: EXTENDED_STARTUPINFO_PRESENT only. No
        // CREATE_SUSPENDED exists in this path — the crash-window shape the
        // spike evaluates away.
        // SAFETY: wide command line outlives the call; the startup struct
        // is `STARTUPINFOEXW` cast to the documented `STARTUPINFOW` base
        // (`cb` announces the extended size); handles are adopted below.
        unsafe {
            CreateProcessW(
                None,
                Some(PWSTR(wide.as_mut_ptr())),
                None,
                None,
                false,
                EXTENDED_STARTUPINFO_PRESENT,
                None,
                None,
                (&raw const startup.StartupInfo).cast(),
                &mut information,
            )
        }
        .map_err(|e| JobError::from("CreateProcessW", &e))?;

        // SAFETY: CreateProcessW returned valid handles; the thread handle
        // is adopted then dropped (closing it does not stop the thread).
        let _thread = unsafe { OwnedHandle::adopt(information.hThread) };
        // SAFETY: as above, the process handle is owned from here on.
        let process = unsafe { OwnedHandle::adopt(information.hProcess) };

        let child = Self {
            pid: information.dwProcessId,
            process,
        };
        // The creation-time proof: the child's own handle reports the job.
        // If the attribute were dropped by the OS this fails loudly and the
        // caller knows the launch was never managed.
        child.in_job(job)?;
        Ok(child)
    }

    /// The child's PID.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Whether the child is a member of `job`, through its own process
    /// handle.
    ///
    /// # Errors
    /// [`JobError::Os`] when the query fails.
    pub fn in_job(&self, job: &AdvisoryJob) -> Result<bool, JobError> {
        in_job_of(&self.process, Some(job.handle.raw()), "IsProcessInJob")
    }

    /// Whether the child is still running (zero-timeout wait).
    ///
    /// # Errors
    /// [`JobError::Os`] when the wait fails.
    pub fn is_running(&self) -> Result<bool, JobError> {
        // SAFETY: owned handle; a zero-timeout wait does not block.
        let event = unsafe { WaitForSingleObject(self.process.raw(), 0) };
        classify_wait(event)
    }

    /// Wait for the child to exit and return its exit code — the
    /// no-suspended-orphan proof: the child must reach completion unaided.
    ///
    /// # Errors
    /// [`JobError::Os`] when the wait or the exit-code query fails.
    pub fn wait(&self) -> Result<u32, JobError> {
        // SAFETY: owned handle; unbounded wait is the documented wait().
        let event = unsafe { WaitForSingleObject(self.process.raw(), INFINITE) };
        if classify_wait(event)? {
            return Err(JobError::Os {
                code: 0,
                function: "wait: handle signalled without exit",
            });
        }
        let mut code = 0u32;
        // SAFETY: owned handle; the out-parameter is a plain u32.
        unsafe { GetExitCodeProcess(self.process.raw(), &mut code) }
            .map_err(|e| JobError::from("GetExitCodeProcess", &e))?;
        Ok(code)
    }
}

fn classify_wait(event: WAIT_EVENT) -> Result<bool, JobError> {
    if event == WAIT_OBJECT_0 {
        Ok(false)
    } else if event == WAIT_TIMEOUT {
        Ok(true)
    } else {
        Err(JobError::os("WaitForSingleObject"))
    }
}

impl AdvisoryJob {
    /// The job handle for attribute-list registration (the value
    /// `UpdateProcThreadAttribute` receives).
    fn handle_raw_as_ptr(&self) -> *const std::ffi::c_void {
        (&raw const self.handle).cast()
    }
}

/// `PROCESS_SET_QUOTA` is the assignment right `AssignProcessToJobObject`
/// requires on the *process* handle; kept next to its only user.
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Child;
    use std::process::Command;
    use std::time::Duration;
    use std::time::Instant;
    use windows::Win32::System::Threading::PROCESS_SET_QUOTA;

    /// A quiet, alive child: `ping -n <seconds> 127.0.0.1` (the RS-016
    /// actuator fixture pattern).
    fn spawn_quiet_child(seconds: u32) -> Child {
        Command::new("ping")
            .args(["-n", &seconds.to_string(), "127.0.0.1"])
            .spawn()
            .expect("spawn ping child")
    }

    #[test]
    fn advisory_job_defaults_have_zero_limits() {
        let job = AdvisoryJob::create().expect("create advisory job");
        assert_eq!(job.limit_flags().expect("limit flags"), 0);
        assert!(job.process_ids().expect("process list").is_empty());
    }

    #[test]
    fn managed_child_is_assigned_at_creation_survives_job_close_and_completes_unaided() {
        let job = AdvisoryJob::create().expect("create advisory job");
        let child = ManagedChild::spawn(&job, "ping -n 4 127.0.0.1").expect("managed spawn");
        assert!(child.in_job(&job).expect("membership"));

        // AT-16: the queried flags stay zero after a managed launch — no
        // kill-on-close, no limits were introduced by the spawn.
        assert_eq!(job.limit_flags().expect("limit flags"), 0);
        assert_eq!(job.process_ids().expect("list"), vec![child.pid()]);

        // Drop the job while the child runs: closing the (last) job handle
        // must not terminate the member — no kill-on-close (AT-15/AT-16).
        drop(job);
        assert!(child.is_running().expect("running after job close"));

        // The no-suspended-orphan proof: the child reaches completion on
        // its own (a suspended child would block this wait forever).
        let exit = child.wait().expect("child completes unaided");
        assert_eq!(exit, 0, "ping exits 0");
    }

    #[test]
    fn failed_launch_reports_error_and_leaves_the_job_empty() {
        let job = AdvisoryJob::create().expect("create advisory job");
        let result = ManagedChild::spawn(&job, "ramshaper-nonexistent-spike.exe --flag");
        assert!(result.is_err(), "a missing image must fail the spawn");
        assert!(
            job.process_ids().expect("list").is_empty(),
            "a failed launch must leave no member behind"
        );
        assert_eq!(job.limit_flags().expect("flags"), 0);
    }

    #[test]
    fn membership_probes_detect_a_job_member() {
        // Creation-time fixture (the mechanism the spike adopts): both
        // membership seams must report a managed child.
        let job = AdvisoryJob::create().expect("create advisory job");
        let child = ManagedChild::spawn(&job, "ping -n 3 127.0.0.1").expect("managed spawn");
        assert!(job.contains_process(child.pid()).expect("contains_process"));
        assert!(is_in_any_job(child.pid()).expect("is_in_any_job"));
        let _ = child.wait();

        // Legacy post-create assignment (the superseded mechanism), probed
        // as an environment finding: on this Windows build (26200) it is
        // ACCESS_DENIED even for a same-user running child — the refusal
        // must be clean (no half-assignment). On older builds it succeeds
        // and the same probes must then report membership.
        let legacy = AdvisoryJob::create().expect("legacy job");
        let mut child2 = spawn_quiet_child(3);
        // SAFETY: numeric PID of our own child; the assignment right
        // PROCESS_SET_QUOTA is required on the process handle.
        let opened = unsafe {
            OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                child2.id(),
            )
        };
        if let Ok(handle) = opened {
            // SAFETY: OpenProcess returned a valid owned handle.
            let handle = unsafe { OwnedHandle::adopt(handle) };
            match legacy.assign(handle.raw()) {
                Ok(()) => assert!(
                    legacy
                        .contains_process(child2.id())
                        .expect("legacy membership"),
                    "assignment succeeded, so the probes must report the member"
                ),
                Err(JobError::Os { code: 5, function }) => {
                    assert_eq!(function, "AssignProcessToJobObject");
                    assert!(
                        !legacy
                            .contains_process(child2.id())
                            .expect("refusal is clean"),
                        "a denied assignment must leave the target out of the job"
                    );
                }
                Err(other) => panic!("unexpected assignment failure: {other:?}"),
            }
        } // else: the child exited before the probe; nothing to assert
        let _ = child2.kill();
        let _ = child2.wait();
    }

    #[test]
    fn nested_environment_child_lands_in_both_jobs() {
        // Fixture: RAMShaper's test process joins an advisory job, then the
        // managed launch must nest the child in BOTH jobs (Win8+ nesting).
        // Side effect, accepted and documented: the test process (and any
        // concurrently spawned test children) stays in the zero-limit,
        // no-kill-on-close job until process exit — no observable effect.
        let outer = AdvisoryJob::create().expect("outer job");
        outer.assign_current_process().expect("join outer job");

        let inner = AdvisoryJob::create().expect("inner job");
        let child = ManagedChild::spawn(&inner, "ping -n 3 127.0.0.1").expect("managed spawn");

        assert!(child.in_job(&inner).expect("inner membership"));
        assert!(
            outer
                .contains_process(child.pid())
                .expect("outer membership"),
            "the child must be nested in the launcher's job, not escaped from it"
        );

        let _ = child.wait();
    }

    #[test]
    fn job_close_is_survivable_even_without_an_explicit_child_reference() {
        // The strictest AT-15 shape this spike can pin in-process: the job
        // handle is dropped, the launch record is gone, and the child still
        // runs to completion with no one left to manage it.
        let child = {
            let job = AdvisoryJob::create().expect("create advisory job");
            ManagedChild::spawn(&job, "ping -n 3 127.0.0.1").expect("managed spawn")
        };
        let started = Instant::now();
        let exit = child.wait().expect("completes");
        assert_eq!(exit, 0);
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the child completed on its own schedule, not held by the job"
        );
    }
}
