//! Global physical/commit memory counters via `GetPerformanceInfo`.
//!
//! Returns raw page counts plus the page size from one API call, so derived
//! values describe one consistent snapshot. All conversion to bytes happens
//! in `ramshaper-core` with checked arithmetic (AT-02).

use ramshaper_core::snapshot::GlobalMemoryRaw;
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};

/// Query global memory counters. Fails only if the OS call fails (the error
/// is returned with its code; counters are never fabricated).
pub fn query_global_memory() -> Result<GlobalMemoryRaw, windows::core::Error> {
    // SAFETY: PERFORMANCE_INFORMATION is a plain C struct of integers; a
    // zeroed instance is valid for the API to fill. The cb parameter is the
    // documented size check — the call validates it and reports failure.
    let mut info: PERFORMANCE_INFORMATION = unsafe { std::mem::zeroed() };
    let cb = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
    unsafe { GetPerformanceInfo(&mut info, cb)? };
    Ok(GlobalMemoryRaw {
        physical_total_pages: info.PhysicalTotal as u64,
        physical_available_pages: info.PhysicalAvailable as u64,
        commit_total_pages: info.CommitTotal as u64,
        commit_limit_pages: info.CommitLimit as u64,
        page_size_bytes: info.PageSize as u64,
    })
}
