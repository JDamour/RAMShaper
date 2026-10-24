//! Mapping `windows` crate errors to typed query failures.

use ramshaper_core::snapshot::QueryFailure;

/// Extract the Win32 error code from a `windows` error (HRESULT with
/// `FACILITY_WIN32`). For HRESULTs in other facilities the low 16 bits are
/// still returned verbatim; callers use the value for diagnostics only.
#[must_use]
pub fn win32_code(error: &windows::core::Error) -> u32 {
    (error.code().0 as u32) & 0xFFFF
}

const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_PARAMETER: u32 = 87;

/// Map a failed API call to a `QueryFailure`, preserving the error code.
/// Access denial is an expected outcome (protected processes, rights).
#[must_use]
pub fn to_query_failure(function: &'static str, error: &windows::core::Error) -> QueryFailure {
    match win32_code(error) {
        ERROR_ACCESS_DENIED => QueryFailure::AccessDenied,
        code => QueryFailure::OsError { code, function },
    }
}

/// Map an `OpenProcess` failure: a vanished PID reports
/// `ERROR_INVALID_PARAMETER`, which means the process exited (or never
/// existed) — the identity-relevant case (AT-03).
#[must_use]
pub fn open_process_failure(error: &windows::core::Error) -> QueryFailure {
    match win32_code(error) {
        ERROR_ACCESS_DENIED => QueryFailure::AccessDenied,
        ERROR_INVALID_PARAMETER => QueryFailure::Exited {
            code: ERROR_INVALID_PARAMETER,
        },
        code => QueryFailure::OsError {
            code,
            function: "OpenProcess",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_denied_maps_to_typed_failure() {
        // HRESULT_FROM_WIN32(5) = 0x8007_0005.
        let err = windows::core::Error::from_hresult(windows::core::HRESULT(0x8007_0005u32 as i32));
        assert_eq!(win32_code(&err), 5);
        assert_eq!(
            to_query_failure("GetPerformanceInfo", &err),
            QueryFailure::AccessDenied
        );
    }

    #[test]
    fn other_codes_preserve_function_and_code() {
        let err = windows::core::Error::from_hresult(windows::core::HRESULT(0x8007_05AFu32 as i32));
        assert_eq!(win32_code(&err), 1455);
        assert_eq!(
            to_query_failure("GetPerformanceInfo", &err),
            QueryFailure::OsError {
                code: 1455,
                function: "GetPerformanceInfo"
            }
        );
    }

    #[test]
    fn open_process_invalid_parameter_means_exited() {
        let err = windows::core::Error::from_hresult(windows::core::HRESULT(0x8007_0057u32 as i32));
        assert_eq!(
            open_process_failure(&err),
            QueryFailure::Exited { code: 87 }
        );
    }
}
