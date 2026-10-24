//! Read-only platform adapter contracts.
//!
//! These traits separate telemetry from (future) actuation so the first CLI
//! cannot accidentally acquire mutation functionality (architecture doc 03 §3).
//! Implementations live in `ramshaper-windows`; deterministic fakes for tests
//! live in [`fake`]. No trait in this crate mutates process or system state.

#![deny(unsafe_code)]

pub mod capabilities;
pub mod fake;
pub mod telemetry;

pub use capabilities::{
    CapabilityInfo, CapabilitySet, CAPABILITY_LOW_MEMORY_NOTIFICATION, CAPABILITY_PROCESS_IDENTITY,
    CAPABILITY_SYSTEM_MEMORY_COUNTERS,
};
pub use telemetry::{MemoryConditionSource, ReadOnlyProcessMonitor, ReadOnlySystemMonitor};
