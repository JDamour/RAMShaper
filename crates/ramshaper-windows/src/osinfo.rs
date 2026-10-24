//! OS/build metadata (read-only).
//!
//! `RtlGetVersion` (the manifest-independent version API) provides
//! major/minor/build; the registry provides the product name, update revision
//! (UBR) and display version; `GetNativeSystemInfo` provides the native
//! architecture. Every source is optional: a failed lookup leaves an explicit
//! `None`/`null` field, never an invented value.

use ramshaper_core::snapshot::OsInfo;
use windows::core::{w, PCWSTR};
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows::Win32::System::SystemInformation::{
    GetNativeSystemInfo, OSVERSIONINFOW, PROCESSOR_ARCHITECTURE, PROCESSOR_ARCHITECTURE_AMD64,
    PROCESSOR_ARCHITECTURE_ARM64, PROCESSOR_ARCHITECTURE_INTEL, SYSTEM_INFO,
};

const OS_SOURCE: &str = "RtlGetVersion;GetNativeSystemInfo;HKLM CurrentVersion";

/// NUL-terminated UTF-16 conversion for registry value names (ASCII input).
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Collect OS metadata; missing sources degrade to `None` fields.
#[must_use]
pub fn collect_os_info() -> OsInfo {
    let (major, minor, build) = rtl_version().map_or((None, None, None), |(maj, min, bld)| {
        (Some(maj), Some(min), Some(bld))
    });
    let ubr = reg_dword("UBR");
    let version = match (major, minor, build) {
        (Some(maj), Some(min), Some(bld)) => {
            let ubr_part = ubr.map(|ubr| format!(".{ubr}")).unwrap_or_default();
            Some(format!("{maj}.{min}.{bld}{ubr_part}"))
        }
        _ => None,
    };
    OsInfo {
        product_name: reg_sz("ProductName"),
        version,
        build,
        ubr,
        display_version: reg_sz("DisplayVersion"),
        arch: native_arch(),
        source: OS_SOURCE,
    }
}

/// `(major, minor, build)` from `RtlGetVersion`, which is not subject to the
/// app-manifest version lie of `GetVersionEx`.
fn rtl_version() -> Option<(u32, u32, u32)> {
    // SAFETY: OSVERSIONINFOW zeroed is valid once dwOSVersionInfoSize is set
    // (documented contract); the call only writes into the struct.
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
    // SAFETY: valid pointer with the size field set.
    let status = unsafe { RtlGetVersion(&mut info) };
    if status.0 == 0 {
        Some((info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber))
    } else {
        None
    }
}

/// Native processor architecture as a stable string.
fn native_arch() -> Option<String> {
    // SAFETY: zeroed SYSTEM_INFO is valid for the API to fill.
    let mut info: SYSTEM_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: plain call with a valid out-pointer.
    unsafe { GetNativeSystemInfo(&mut info) };
    // SAFETY: the API fills the union through this arm (processor
    // architecture); reading the same arm is defined.
    let arch: PROCESSOR_ARCHITECTURE = unsafe { info.Anonymous.Anonymous.wProcessorArchitecture };
    let name = match arch {
        a if a == PROCESSOR_ARCHITECTURE_AMD64 => "x86_64",
        a if a == PROCESSOR_ARCHITECTURE_ARM64 => "aarch64",
        a if a == PROCESSOR_ARCHITECTURE_INTEL => "x86",
        _ => return None,
    };
    Some(name.to_string())
}

/// Read a REG_SZ value from `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`.
fn reg_sz(value: &str) -> Option<String> {
    let mut buffer = [0u16; 512];
    let mut size = (buffer.len() * 2) as u32;
    let name = wide(value);
    // SAFETY: buffer/size are valid in/out parameters; the API writes at
    // most `size` bytes into `buffer` and updates `size`.
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    let len = ((size / 2) as usize).min(buffer.len());
    let end = buffer[..len].iter().position(|&c| c == 0).unwrap_or(len);
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// Read a REG_DWORD value from `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`.
fn reg_dword(value: &str) -> Option<u32> {
    let mut data: u32 = 0;
    let mut size = 4u32;
    let name = wide(value);
    // SAFETY: valid DWORD-sized out buffer and size parameter.
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    (result == ERROR_SUCCESS).then_some(data)
}
