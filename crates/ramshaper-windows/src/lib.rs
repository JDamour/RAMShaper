//! RAMShaper Windows adapter (read-only).
//!
//! Every `unsafe` FFI call in the workspace lives in the modules below, each
//! of which is small enough to audit:
//!
//! - [`handle`] — RAII ownership for OS handles (`CloseHandle` on drop).
//! - [`perfinfo`] — global memory/commit page counters (`GetPerformanceInfo`).
//! - [`memnotify`] — low-memory resource notification
//!   (`CreateMemoryResourceNotification` / `QueryMemoryResourceNotification`).
//! - [`identity`] — process identity via PID + exact creation FILETIME
//!   (`OpenProcess` with `PROCESS_QUERY_LIMITED_INFORMATION`, `GetProcessTimes`).
//! - [`job`] — managed-launch spike (RS-020): fresh advisory job + creation-time
//!   job assignment via the attribute list; no suspension, no kill-on-close.
//! - [`console`] — Ctrl+C cancellation event (`SetConsoleCtrlHandler`).
//! - [`eligibility`] — eligibility signal-source adapters (R-03): owner/session,
//!   foreground, priority class, children, capability probe, classification.
//! - [`osinfo`] — OS/build metadata (`RtlGetVersion`, registry, `GetNativeSystemInfo`).
//! - [`monitor`] — read-only adapter implementing the platform contracts.
//!
//! Design rules (docs 04 §B, contract 14): minimal access rights, checked
//! return values with preserved Win32 error codes, expected failures (access
//! denied, process exit) mapped to typed `QueryFailure`s, never panics.

pub mod console;
pub mod eligibility;
pub mod error;
pub mod event;
pub mod fs;
pub mod handle;
pub mod identity;
pub mod job;
pub mod memnotify;
pub mod monitor;
pub mod osinfo;
pub mod perfinfo;
pub mod pipe;
pub mod priority;
pub mod processes;
pub mod token;
pub mod window;

pub use monitor::{agent_instance_id, WindowsSystemMonitor};
