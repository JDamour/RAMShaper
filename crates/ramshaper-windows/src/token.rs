//! Current-user SID lookup for pipe DACLs and peer validation (S26).
//!
//! The agent's named pipe carries an explicit DACL granting only the
//! serving user, and every accepted connection is re-validated against the
//! serving user's SID and logon session. Same-user malware remains outside
//! the isolation guarantee (R-07); the checks stop cross-user/cross-session
//! peers, which is all a non-elevated pipe can and should do.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetSecurityDescriptorLength, GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::error::win32_code;

/// A SID rendered as its SDDL string form (e.g. `S-1-5-21-...`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidString(pub String);

/// RAII token handle.
struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        // SAFETY: handle owned by this wrapper, closed exactly once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Read the user SID from an owned token handle (closed on return).
fn sid_from_token(token: TokenHandle) -> Result<SidString, ramshaper_core::snapshot::QueryFailure> {
    let function = "GetTokenInformation";
    let token = token.0;
    // SAFETY: sizing call with a null buffer is the documented pattern.
    let mut needed = 0u32;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    if needed == 0 {
        return Err(ramshaper_core::snapshot::QueryFailure::OsError { code: 0, function });
    }
    let mut buffer = vec![0u8; needed as usize];
    // SAFETY: buffer sized by the previous call; valid length pointer.
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .map_err(|e| ramshaper_core::snapshot::QueryFailure::OsError {
            code: win32_code(&e),
            function,
        })?;
    }
    // SAFETY: the buffer holds a TOKEN_USER per the successful call; the
    // User.Sid pointer is valid while the buffer lives.
    let user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let sid = user.User.Sid;
    let mut wide = windows::core::PWSTR::null();
    // SAFETY: valid SID from the token; out-param freed after reading.
    unsafe {
        ConvertSidToStringSidW(sid, &mut wide).map_err(|e| {
            ramshaper_core::snapshot::QueryFailure::OsError {
                code: win32_code(&e),
                function: "ConvertSidToStringSidW",
            }
        })?;
    }
    // SAFETY: `wide` points to a LocalAlloc'd string we free after copying.
    let sid_string = unsafe {
        let s = wide
            .to_string()
            .map_err(|_| ramshaper_core::snapshot::QueryFailure::OsError { code: 0, function })?;
        let _ = LocalFree(Some(HLOCAL(wide.as_ptr().cast())));
        s
    };
    Ok(SidString(sid_string))
}

/// The serving user's SID (from the agent's own process token).
pub fn current_user_sid() -> Result<SidString, ramshaper_core::snapshot::QueryFailure> {
    let mut token = HANDLE::default();
    // SAFETY: current-process pseudo-handle; TOKEN_QUERY only; valid
    // out-pointer; the token is closed by the RAII wrapper.
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| crate::error::to_query_failure("OpenProcessToken", &e))?;
    }
    sid_from_token(TokenHandle(token))
}

/// The user SID of another process (peer validation). A protected or
/// foreign process denies the open, which surfaces as a typed failure and
/// fails validation.
pub fn user_sid_of(pid: u32) -> Result<SidString, ramshaper_core::snapshot::QueryFailure> {
    // SAFETY: numeric PID; query-limited rights only; owned by RAII below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|e| crate::error::open_process_failure(&e))?;
    struct Owned(HANDLE);
    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: owned handle, closed once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let owned = Owned(process);
    let mut token = HANDLE::default();
    // SAFETY: owned query-limited handle; valid out-pointer.
    unsafe {
        OpenProcessToken(owned.0, TOKEN_QUERY, &mut token)
            .map_err(|e| crate::error::to_query_failure("OpenProcessToken", &e))?;
    }
    sid_from_token(TokenHandle(token))
}

/// The current logon session id.
pub fn current_session_id() -> Result<u32, ramshaper_core::snapshot::QueryFailure> {
    let mut session = 0u32;
    // SAFETY: valid out-pointer; current PID.
    unsafe {
        ProcessIdToSessionId(GetCurrentProcessId(), &mut session).map_err(|e| {
            ramshaper_core::snapshot::QueryFailure::OsError {
                code: win32_code(&e),
                function: "ProcessIdToSessionId",
            }
        })?;
    }
    Ok(session)
}

/// Build a self-relative security descriptor granting GENERIC_ALL to `sid`
/// only (explicit DACL — never the permissive default, per S26). The
/// returned bytes back `SECURITY_ATTRIBUTES.lpSecurityDescriptor`.
pub fn security_descriptor_for_user(
    sid: &SidString,
) -> Result<Vec<u8>, ramshaper_core::snapshot::QueryFailure> {
    let sddl = format!("D:(A;;GA;;;{})", sid.0);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: SDDL string in, descriptor out; copied to owned bytes and
    // freed below.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide(&sddl).as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|e| ramshaper_core::snapshot::QueryFailure::OsError {
            code: win32_code(&e),
            function: "ConvertStringSecurityDescriptorToSecurityDescriptorW",
        })?;
    }
    // SAFETY: length query, copy, then free the original allocation.
    unsafe {
        let length = GetSecurityDescriptorLength(descriptor);
        if length == 0 {
            let _ = LocalFree(Some(HLOCAL(descriptor.0.cast())));
            return Err(ramshaper_core::snapshot::QueryFailure::OsError {
                code: 0,
                function: "GetSecurityDescriptorLength",
            });
        }
        let slice = std::slice::from_raw_parts(descriptor.0.cast::<u8>(), length as usize);
        let owned = slice.to_vec();
        let _ = LocalFree(Some(HLOCAL(descriptor.0.cast())));
        Ok(owned)
    }
}

/// NUL-terminated UTF-16 conversion.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Restrict an existing file to the current user: owner and DACL are set
/// to a single grant-all ACE for the current user's SID (no inherited
/// ACEs). The action journal's privacy boundary (contract 14 §6: recovery
/// records are user-private, like the pipe's explicit user-only DACL).
///
/// # Errors
/// Typed [`QueryFailure`] with the failing API preserved.
pub fn restrict_file_to_current_user(
    path: &std::path::Path,
) -> Result<(), ramshaper_core::snapshot::QueryFailure> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::Security::Authorization::SetNamedSecurityInfoW;
    use windows::Win32::Security::Authorization::SE_FILE_OBJECT;
    use windows::Win32::Security::{
        GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION, OBJECT_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION,
    };

    // SetNamedSecurityInfoW rejects relative paths (ERROR_INVALID_PARAMETER);
    // canonicalize to the absolute form.
    let absolute = std::fs::canonicalize(path).map_err(|error| {
        ramshaper_core::snapshot::QueryFailure::Unsupported(format!(
            "cannot resolve the file path for hardening: {error}"
        ))
    })?;
    let sid = current_user_sid()?;
    let descriptor = security_descriptor_for_user(&sid)?;

    // SAFETY: valid self-relative descriptor produced above; out-pointers
    // valid; the returned pointers reference the descriptor's storage,
    // which outlives the SetNamedSecurityInfoW call below.
    let dacl = unsafe {
        let descriptor_boxed = PSECURITY_DESCRIPTOR(descriptor.as_ptr() as *mut core::ffi::c_void);
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut dacl_present = windows::Win32::Foundation::FALSE;
        let mut dacl_defaulted = windows::Win32::Foundation::FALSE;
        GetSecurityDescriptorDacl(
            descriptor_boxed,
            &mut dacl_present,
            &mut dacl,
            &mut dacl_defaulted,
        )
        .map_err(|e| ramshaper_core::snapshot::QueryFailure::OsError {
            code: win32_code(&e),
            function: "GetSecurityDescriptorDacl",
        })?;
        if !dacl_present.as_bool() {
            return Err(ramshaper_core::snapshot::QueryFailure::Unsupported(
                "security descriptor has no DACL".to_string(),
            ));
        }
        dacl
    };

    // The DACL alone is applied: the descriptor carries no owner (the
    // SDDL string is DACL-only), and the file's owner is already the
    // creating user. Owner+DACL with a null owner is ERROR_INVALID_PARAMETER.
    // SAFETY: valid wide path and DACL pointer from the descriptor.
    unsafe {
        let result = SetNamedSecurityInfoW(
            PCWSTR(wide(&absolute.to_string_lossy()).as_ptr()),
            SE_FILE_OBJECT,
            OBJECT_SECURITY_INFORMATION(
                (DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION).0,
            ),
            None,
            None,
            Some(dacl),
            None,
        );
        if result != ERROR_SUCCESS {
            return Err(ramshaper_core::snapshot::QueryFailure::OsError {
                code: result.0,
                function: "SetNamedSecurityInfoW",
            });
        }
    }
    Ok(())
}
