//! RAMShaper session agent library (RS-030).
//!
//! One non-elevated agent per interactive user/logon session, exposing the
//! read-only monitor over a session-scoped named pipe (protocol v1). This
//! crate contains no `unsafe`: all OS interop lives in `ramshaper-windows`.

#![deny(unsafe_code)]

pub mod dispatch;
pub mod server;
