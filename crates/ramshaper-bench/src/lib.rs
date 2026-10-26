//! `ramshaper-bench` library surface — the measurement harness modules,
//! exposed so integration tests (and the binary) share one implementation.
//!
//! - [`analysis`]: the AT-20 A/B analysis (nearest-rank endpoints,
//!   whole-block bootstrap, the four honest labels).
//! - [`prereg`]: executable trial preregistration (closed schema; every
//!   deviation named; the plan is the authority for the executed run).
//! - [`workload`]: the RS-024 Stage B counterbalanced CPU task-latency
//!   pilot (bounded, allocation-free; stop control).
//!
//! No `unsafe` in this crate.

pub mod analysis;
pub mod prereg;
pub mod workload;
