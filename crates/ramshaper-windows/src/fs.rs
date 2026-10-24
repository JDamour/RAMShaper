//! Atomic file persistence for configuration (contract 14 §7: persist
//! atomically). Writes a sibling temporary file, then replaces the target
//! with `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` — readers see either the
//! old or the new file, never a partial one.

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, MoveFileExW, WriteFile, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE,
    FILE_SHARE_NONE, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

use crate::handle::OwnedHandle;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Atomically write `bytes` to `path` (temp + replace). The error carries
/// the preserved Win32 code and the failing API name.
pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), (u32, &'static str)> {
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));

    // SAFETY: valid path string; no sharing; CREATE_ALWAYS truncates any
    // leftover temp file; the returned handle is validated by windows-rs.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide(&temp.to_string_lossy()).as_ptr()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|e| ((e.code().0 as u32) & 0xFFFF, "CreateFileW"))?;
    // SAFETY: valid owned file handle from CreateFileW.
    let handle = unsafe { OwnedHandle::adopt(handle) };

    let mut written = 0u32;
    // SAFETY: valid handle; the whole buffer is valid for the write.
    unsafe { WriteFile(handle.raw(), Some(bytes), Some(&mut written), None) }
        .map_err(|e| ((e.code().0 as u32) & 0xFFFF, "WriteFile"))?;
    if written as usize != bytes.len() {
        return Err((0, "WriteFile"));
    }
    drop(handle); // close before the replace

    // SAFETY: both paths are valid strings; flags request atomic replace.
    unsafe {
        MoveFileExW(
            PCWSTR(wide(&temp.to_string_lossy()).as_ptr()),
            PCWSTR(wide(&path.to_string_lossy()).as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| ((e.code().0 as u32) & 0xFFFF, "MoveFileExW"))?;
    Ok(())
}
