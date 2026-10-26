//! RAMShaper core: pure domain types and decisions for the read-only CLI.
//!
//! This crate makes no OS calls. Everything here is deterministic and testable
//! with fake clocks and fixtures; Windows API access lives in
//! `ramshaper-windows`, behind the contracts in `ramshaper-platform`.
//!
//! Contracts implemented here come from `docs/docs/14_RUNTIME_CONTRACTS.md`:
//! every metric carries `value`/`state`, `status`, `source`, `observed_at_utc`,
//! `sample_sequence` and `age_ms`; a non-OK metric is never zero; bytes are the
//! internal unit and MiB/GiB exist only at presentation boundaries.

#![deny(unsafe_code)]

pub mod clock;
pub mod cohort;
pub mod config;
pub mod coordinator;
pub mod eligibility;
pub mod envelope;
pub mod history;
pub mod identity;
pub mod ipc;
pub mod journal;
pub mod policy;
pub mod pressure;
pub mod process;
pub mod render;
pub mod rfc3339;
pub mod snapshot;
pub mod status;
pub mod units;
pub mod watch;

pub use clock::{FakeClock, MonotonicClock, SystemClock, WallClock};
pub use cohort::{
    verify_declared, CohortError, HardwareCohort, UnknownCohortDeclaration, COHORT_16GIB_BYTES,
    COHORT_4GIB_BYTES, COHORT_8GIB_BYTES,
};
pub use config::{
    ConfigError, CoreConfig, WatchSettings, CONFIG_SCHEMA_VERSION, MODE_OBSERVE_ONLY,
};
pub use coordinator::{
    ActionJournal, ActuatorError, ApplyOutcome, ConfirmOutcome, Coordinator, JournalFault,
    PendingPreview, PreviewRequest, PreviewVerdict, PriorityActuator,
};
pub use eligibility::{
    evaluate, Allowlist, CheckId, CheckResult, ChildObservation, DenyReason, EligibilityInputs,
    EligibilityReport, FocusObservation, Outcome, OwnerSessionObservation, PriorityCapability,
    PriorityClassObservation, ProtectedCategory, SafetyClassification, APPLY_INPUT_FRESHNESS_MS,
    ELIGIBILITY_RULES_VERSION,
};
pub use envelope::{LowMemorySignal, MetricEnvelope};
pub use history::{BoundedHistory, HistoryStats, DEFAULT_BYTE_BUDGET, DEFAULT_RETENTION_MS};
pub use identity::{IdentityVerification, ProcessIdentity};
pub use journal::{
    reconcile, retention, transition, ActionIntent, ActionState, Event, Observed, Property,
    ReconcileDecision, Retention, TransitionError, ACTION_KIND_CPU_PRIORITY,
    JOURNAL_SCHEMA_VERSION,
};
pub use policy::{
    dry_run, DryRunConfig, DryRunLog, DryRunSummary, NoActionReason, PolicyDecision,
    PolicyRuleError, PolicyRules, ResolveCause, StepDecision, TargetObservation, TraceStep,
    DEFAULT_TTL_MS, DRY_RUN_SCHEMA, MAX_TTL_MS, MIN_TTL_MS, POLICY_RULES_VERSION,
    PREVIEW_VALIDITY_MS,
};
pub use pressure::{
    missing_inputs as missing_pressure_inputs, EvidenceQuality, PressureClassifier, PressureInput,
    PressureLevel, PressureObservation, PRESSURE_INPUTS, PRESSURE_RULE_VERSION,
};
pub use process::{
    build_inventory, ChurnSummary, ProcessCountersRaw, ProcessEntry, ProcessInventory,
    ProcessMetric, ProcessScan, RateBaselines, MAX_PROCESS_IDENTITIES,
};
pub use snapshot::{
    build_snapshot, classify_snapshot, GlobalMemoryRaw, OsInfo, QueryFailure, SnapshotHealth,
    SnapshotInputs, SystemMetrics, SystemSample, SystemSnapshot, ToolInfo,
};
pub use status::MetricStatus;
pub use units::{format_bytes, pages_to_bytes, Bytes, PageConversionError};
pub use watch::{run_watch, WaitHandle, WaitOutcome, WatchConfig, WatchSummary, WatchTermination};
