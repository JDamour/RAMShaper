//! Named-pipe server and client wrappers (RS-030, contract 14 §7).
//!
//! The server creates instances with an **explicit** DACL (never the
//! permissive default) granting only the serving user; byte-mode framing is
//! the 4-byte length + JSON protocol from `ramshaper_core::ipc`, with the
//! 1 MiB cap enforced *before* buffer allocation. Peer validation (session
//! id + user SID) happens at accept time in the agent; this module exposes
//! the raw client-id queries.

use ramshaper_core::ipc::{decode_frame_prefix, encode_frame, FrameError, MAX_FRAME_BYTES};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_NONE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    GetNamedPipeClientSessionId, NAMED_PIPE_MODE, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
};

use crate::handle::OwnedHandle;

/// Pipe-mode flags: byte mode with blocking waits.
const PIPE_MODE_BYTE_WAIT: u32 = PIPE_TYPE_BYTE.0 | PIPE_READMODE_BYTE.0 | PIPE_WAIT.0;
/// Win32 code for a broken pipe (EOF / peer gone).
const ERROR_BROKEN_PIPE: u32 = 232;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn hresult_code(error: &windows::core::Error) -> u32 {
    (error.code().0 as u32) & 0xFFFF
}

/// Errors surfaced by pipe I/O.
#[derive(Debug)]
pub enum PipeError {
    /// The peer's frame violated the protocol (oversized/malformed).
    Frame(FrameError),
    /// An OS error with its preserved code.
    Os(u32, &'static str),
}

impl std::fmt::Display for PipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipeError::Frame(e) => write!(f, "{e}"),
            PipeError::Os(code, function) => write!(f, "OS error {code} from {function}"),
        }
    }
}

/// Create one server pipe instance with the explicit user-only DACL.
/// `first_instance` must be true only for the first instance of this name;
/// `max_instances` bounds concurrent instances (bounded memory).
pub fn create_instance(
    name: &str,
    security_descriptor: &[u8],
    first_instance: bool,
    max_instances: u32,
) -> Result<OwnedHandle, PipeError> {
    let attributes = windows::Win32::Security::SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<windows::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security_descriptor.as_ptr().cast_mut().cast(),
        bInheritHandle: false.into(),
    };
    let mut open_mode = PIPE_ACCESS_DUPLEX.0;
    if first_instance {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE.0;
    }
    // SAFETY: name/DACL pointers are valid for the call; the returned
    // handle is validated before adoption.
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(wide(name).as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(open_mode),
            NAMED_PIPE_MODE(PIPE_MODE_BYTE_WAIT),
            max_instances,
            65536,
            65536,
            0,
            Some(&attributes),
        )
    };
    if handle.is_invalid() {
        // SAFETY: capturing the immediate last-error after the failed call.
        return Err(PipeError::Os(
            unsafe { GetLastError() }.0,
            "CreateNamedPipeW",
        ));
    }
    // SAFETY: valid owned pipe-instance handle.
    Ok(unsafe { OwnedHandle::adopt(handle) })
}

/// Win32 code for I/O cancelled via `CancelSynchronousIo`.
const ERROR_OPERATION_ABORTED: u32 = 995;

/// Duplicate the calling thread's handle, for use with
/// [`cancel_synchronous_io`].
pub fn current_thread_handle() -> Result<OwnedHandle, PipeError> {
    use windows::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread};
    let mut duplicated = windows::Win32::Foundation::HANDLE::default();
    // SAFETY: duplicating the current thread's pseudo-handle into our own
    // process; the returned real handle is adopted for RAII.
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            GetCurrentThread(),
            GetCurrentProcess(),
            &mut duplicated,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .map_err(|e| PipeError::Os(hresult_code(&e), "DuplicateHandle"))?;
    // SAFETY: valid owned thread handle from DuplicateHandle.
    Ok(unsafe { OwnedHandle::adopt(duplicated) })
}

/// Cancel a blocked synchronous I/O call (such as `ConnectNamedPipe`) in
/// the target thread; used to make the accept loop stoppable.
pub fn cancel_synchronous_io(thread: &OwnedHandle) -> Result<(), PipeError> {
    // SAFETY: valid thread handle obtained from `current_thread_handle`.
    unsafe { windows::Win32::System::IO::CancelSynchronousIo(thread.raw()) }
        .map_err(|e| PipeError::Os(hresult_code(&e), "CancelSynchronousIo"))?;
    Ok(())
}

/// Await a client on `instance`, cancellably. The instance is a
/// synchronous-mode handle, so `ConnectNamedPipe` blocks the calling thread
/// until a client arrives; a stopper calls `cancel_synchronous_io` with the
/// accept thread's handle, which aborts the blocked call. Returns
/// `Ok(true)` when a client connected, `Ok(false)` when the call was
/// cancelled (treat as stop).
pub fn connect_cancellable(instance: &OwnedHandle) -> Result<bool, PipeError> {
    // SAFETY: valid instance handle. On a synchronous handle the OVERLAPPED
    // pointer is ignored; the call blocks until connect or cancellation.
    let connect_result = unsafe { ConnectNamedPipe(instance.raw(), None) };
    match connect_result {
        Ok(()) => Ok(true),
        Err(e) => {
            let code = hresult_code(&e);
            if code == ERROR_PIPE_CONNECTED.0 {
                Ok(true) // client attached between create and connect
            } else if code == ERROR_OPERATION_ABORTED {
                Ok(false) // cancelled by the stopper
            } else {
                Err(PipeError::Os(code, "ConnectNamedPipe"))
            }
        }
    }
}

/// Open a client handle to an existing pipe.
pub fn client_connect(name: &str) -> Result<OwnedHandle, PipeError> {
    // SAFETY: valid name; no sharing; open-existing only; windows-rs
    // validates the returned handle.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide(name).as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|e| PipeError::Os(hresult_code(&e), "CreateFileW"))?;
    // SAFETY: valid owned file handle from CreateFileW.
    Ok(unsafe { OwnedHandle::adopt(handle) })
}

/// Read exactly `buffer.len()` bytes, looping over partial reads. Zero-byte
/// reads or a broken pipe mean the peer is gone.
fn read_exact(handle: &OwnedHandle, buffer: &mut [u8]) -> Result<(), PipeError> {
    let mut filled = 0usize;
    while filled < buffer.len() {
        let mut read = 0u32;
        // SAFETY: valid handle; the slice covers exactly the unfilled part.
        let result = unsafe {
            ReadFile(
                handle.raw(),
                Some(&mut buffer[filled..]),
                Some(&mut read),
                None,
            )
        };
        match result {
            Ok(()) if read > 0 => filled += read as usize,
            Ok(()) => return Err(PipeError::Os(ERROR_BROKEN_PIPE, "ReadFile")),
            Err(e) => return Err(PipeError::Os(hresult_code(&e), "ReadFile")),
        }
    }
    Ok(())
}

/// Read one protocol frame (length-checked before allocation).
pub fn read_frame(handle: &OwnedHandle) -> Result<Vec<u8>, PipeError> {
    let mut prefix = [0u8; 4];
    read_exact(handle, &mut prefix)?;
    let length = decode_frame_prefix(prefix).map_err(PipeError::Frame)?;
    let mut payload = vec![0u8; length];
    read_exact(handle, &mut payload)?;
    Ok(payload)
}

/// Write raw bytes (no framing). Used by tests to inject hostile prefixes.
pub fn write_bytes(handle: &OwnedHandle, bytes: &[u8]) -> Result<(), PipeError> {
    let mut written = 0u32;
    // SAFETY: valid handle; the buffer is valid for the write.
    unsafe { WriteFile(handle.raw(), Some(bytes), Some(&mut written), None) }
        .map_err(|e| PipeError::Os(hresult_code(&e), "WriteFile"))?;
    if written as usize != bytes.len() {
        return Err(PipeError::Os(ERROR_BROKEN_PIPE, "WriteFile"));
    }
    Ok(())
}

/// Write one protocol frame.
pub fn write_frame(handle: &OwnedHandle, payload: &[u8]) -> Result<(), PipeError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(PipeError::Frame(FrameError::Oversized(
            payload.len() as u32,
            MAX_FRAME_BYTES,
        )));
    }
    let frame = encode_frame(payload);
    write_bytes(handle, &frame)
}

/// Wait until the peer has read everything written so far, so a final
/// frame is not discarded by a subsequent disconnect (documented pipe
/// semantics: `FlushFileBuffers` blocks until the client consumes).
pub fn flush(handle: &OwnedHandle) -> Result<(), PipeError> {
    // SAFETY: owned valid handle.
    unsafe { FlushFileBuffers(handle.raw()) }
        .map_err(|e| PipeError::Os(hresult_code(&e), "FlushFileBuffers"))?;
    Ok(())
}

/// Disconnect the client of a server instance (before reuse/close).
pub fn disconnect(instance: &OwnedHandle) {
    // SAFETY: owned instance handle; best-effort disconnect.
    unsafe {
        let _ = DisconnectNamedPipe(instance.raw());
    }
}

/// The client process id of a connected server instance.
pub fn client_process_id(instance: &OwnedHandle) -> Result<u32, PipeError> {
    let mut pid = 0u32;
    // SAFETY: valid out-pointer; owned instance handle.
    unsafe { GetNamedPipeClientProcessId(instance.raw(), &mut pid) }
        .map_err(|e| PipeError::Os(hresult_code(&e), "GetNamedPipeClientProcessId"))?;
    Ok(pid)
}

/// The client logon session id of a connected server instance.
pub fn client_session_id(instance: &OwnedHandle) -> Result<u32, PipeError> {
    let mut session = 0u32;
    // SAFETY: valid out-pointer; owned instance handle.
    unsafe { GetNamedPipeClientSessionId(instance.raw(), &mut session) }
        .map_err(|e| PipeError::Os(hresult_code(&e), "GetNamedPipeClientSessionId"))?;
    Ok(session)
}
