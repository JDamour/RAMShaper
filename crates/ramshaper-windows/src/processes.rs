//! Process enumeration and per-process counters (RS-007).
//!
//! Enumeration: `CreateToolhelp32Snapshot`/`Process32FirstW`/`Process32NextW`
//! (documented, standard-user). Per-process queries go through one
//! `PROCESS_QUERY_LIMITED_INFORMATION` handle per process: `GetProcessTimes`
//! (identity + CPU), `GetProcessMemoryInfo` (working set / private commit),
//! `GetProcessIoCounters` (process I/O). Every query result is preserved as
//! a typed `Result`; denial and exit are expected outcomes. The inventory is
//! capped at [`MAX_PROCESS_IDENTITIES`] with an explicit truncation flag
//! (contract 14 §3).

use ramshaper_core::identity::ProcessIdentity;
use ramshaper_core::process::{ProcessCountersRaw, ProcessScan, MAX_PROCESS_IDENTITIES};
use ramshaper_core::snapshot::QueryFailure;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::GetNativeSystemInfo;
use windows::Win32::System::Threading::{
    GetProcessIoCounters, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::error::to_query_failure;
use crate::handle::OwnedHandle;

/// Logical processor count from the native system info (CPU-topology marker
/// for rate resets).
pub fn logical_cpu_count() -> u32 {
    // SAFETY: zeroed SYSTEM_INFO is valid for the API to fill.
    let mut info =
        unsafe { std::mem::zeroed::<windows::Win32::System::SystemInformation::SYSTEM_INFO>() };
    // SAFETY: plain call with a valid out-pointer.
    unsafe { GetNativeSystemInfo(&mut info) };
    info.dwNumberOfProcessors
}

/// Snapshot-enum failure mapped to a typed query failure.
fn snapshot_failure(error: &windows::core::Error) -> QueryFailure {
    to_query_failure("CreateToolhelp32Snapshot", error)
}

/// Enumerate PIDs with names and parents (labels only; identity comes from
/// the per-process query through a real handle).
fn enumerate_pids() -> Result<Vec<(u32, String, u32)>, QueryFailure> {
    // SAFETY: TH32CS_SNAPPROCESS with pid 0 snapshots the whole table;
    // windows-rs validates the returned handle.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|e| snapshot_failure(&e))?;
    // SAFETY: valid owned snapshot handle.
    let snapshot = unsafe { OwnedHandle::adopt(snapshot) };

    // SAFETY: PROCESSENTRY32W zeroed then size-set is the documented
    // contract for Process32FirstW.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    let mut rows = Vec::new();
    // SAFETY: valid out-pointer with dwSize set; handle owned.
    let mut more = unsafe { Process32FirstW(snapshot.raw(), &mut entry) };
    while more.is_ok() {
        let name_len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..name_len]);
        rows.push((entry.th32ProcessID, name, entry.th32ParentProcessID));
        // SAFETY: same contract as First.
        more = unsafe { Process32NextW(snapshot.raw(), &mut entry) };
        if rows.len() >= MAX_PROCESS_IDENTITIES {
            // Bounded inventory: stop enumerating and report truncation.
            return Ok(rows);
        }
    }
    Ok(rows)
}

/// Query one process's raw counters through a single limited handle.
fn query_process(pid: u32, name: String, parent_pid: u32) -> ProcessCountersRaw {
    // SAFETY: plain call with a numeric PID; no inherit.
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(handle) => {
            // SAFETY: valid owned handle from OpenProcess.
            unsafe { OwnedHandle::adopt(handle) }
        }
        Err(error) => {
            let failure = crate::error::open_process_failure(&error);
            return ProcessCountersRaw {
                identity: ProcessIdentity::new(pid, 0),
                name,
                parent_pid,
                working_set_bytes: Err(failure.clone()),
                private_commit_bytes: Err(failure.clone()),
                cpu_kernel_100ns: Err(failure.clone()),
                cpu_user_100ns: Err(failure.clone()),
                io_read_bytes: Err(failure.clone()),
                io_write_bytes: Err(failure),
            };
        }
    };

    let (creation, kernel, user) = match process_times(&handle) {
        Ok((creation, _exit, kernel, user)) => (creation, kernel, user),
        Err(failure) => {
            return ProcessCountersRaw {
                identity: ProcessIdentity::new(pid, 0),
                name,
                parent_pid,
                working_set_bytes: Err(failure.clone()),
                private_commit_bytes: Err(failure.clone()),
                cpu_kernel_100ns: Err(failure.clone()),
                cpu_user_100ns: Err(failure.clone()),
                io_read_bytes: Err(failure.clone()),
                io_write_bytes: Err(failure),
            };
        }
    };

    // Memory counters: PROCESS_MEMORY_COUNTERS_EX with cb set to the EX size
    // (documented superset pattern for GetProcessMemoryInfo).
    let mut memory: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    memory.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    // SAFETY: valid out-pointer with the documented size; owned handle.
    let memory_result = unsafe {
        GetProcessMemoryInfo(
            handle.raw(),
            &mut memory as *mut PROCESS_MEMORY_COUNTERS_EX
                as *mut windows::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
    };
    let (working_set, private_commit) = match memory_result {
        Ok(()) => (
            Ok(memory.WorkingSetSize as u64),
            Ok(memory.PrivateUsage as u64),
        ),
        Err(error) => {
            let failure = to_query_failure("GetProcessMemoryInfo", &error);
            (Err(failure.clone()), Err(failure))
        }
    };

    // Process I/O counters (process accounting; not physical-disk/paging).
    let mut io = windows::Win32::System::Threading::IO_COUNTERS::default();
    // SAFETY: valid out-pointer of the documented type; owned handle.
    let io_result = unsafe { GetProcessIoCounters(handle.raw(), &mut io) };
    let (io_read, io_write) = match io_result {
        Ok(()) => (Ok(io.ReadTransferCount), Ok(io.WriteTransferCount)),
        Err(error) => {
            let failure = to_query_failure("GetProcessIoCounters", &error);
            (Err(failure.clone()), Err(failure))
        }
    };

    ProcessCountersRaw {
        identity: ProcessIdentity::new(pid, creation),
        name,
        parent_pid,
        working_set_bytes: working_set,
        private_commit_bytes: private_commit,
        cpu_kernel_100ns: Ok(kernel),
        cpu_user_100ns: Ok(user),
        io_read_bytes: io_read,
        io_write_bytes: io_write,
    }
}

/// `(creation, exit, kernel, user)` FILETIME values converted to u64.
fn process_times(handle: &OwnedHandle) -> Result<(u64, u64, u64, u64), QueryFailure> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: four valid out-pointers; owned query-only handle.
    unsafe {
        GetProcessTimes(
            handle.raw(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
        .map_err(|e| QueryFailure::OsError {
            code: crate::error::win32_code(&e),
            function: "GetProcessTimes",
        })?;
    }
    let combine =
        |ft: &FILETIME| (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
    Ok((
        combine(&creation),
        combine(&exit),
        combine(&kernel),
        combine(&user),
    ))
}

/// One full scan: enumerate, then query every process. Truncation beyond
/// the identity cap is reported, not silently dropped.
pub fn scan_processes() -> ProcessScan {
    let cpus = logical_cpu_count();
    let rows = match enumerate_pids() {
        Ok(rows) => rows,
        Err(failure) => {
            return ProcessScan {
                processes: Vec::new(),
                truncated: false,
                scan_failure: Some(failure),
                logical_cpu_count: cpus,
            };
        }
    };
    let truncated = rows.len() >= MAX_PROCESS_IDENTITIES;
    let mut processes = Vec::with_capacity(rows.len());
    for (pid, name, parent_pid) in rows {
        processes.push(query_process(pid, name, parent_pid));
    }
    ProcessScan {
        processes,
        truncated,
        scan_failure: None,
        logical_cpu_count: cpus,
    }
}

/// The current process's own `(working_set, private_commit)` bytes, for
/// observer-overhead reporting. `None` if the query fails (never invented).
pub fn current_process_memory_bytes() -> Option<(u64, u64)> {
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut memory: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    memory.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    // SAFETY: pseudo-handle, valid out-pointer, documented size.
    let result = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut memory as *mut PROCESS_MEMORY_COUNTERS_EX
                as *mut windows::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
    };
    result
        .ok()
        .map(|()| (memory.WorkingSetSize as u64, memory.PrivateUsage as u64))
}
