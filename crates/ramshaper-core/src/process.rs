//! Process telemetry types and rate computation (RS-007).
//!
//! Contract 14 §2: process CPU is the delta of kernel+user time over a
//! monotonic interval divided by the logical CPU count (percent of total
//! machine capacity); process I/O is the delta of `GetProcessIoCounters`
//! byte counters (process I/O, not physical-disk or paging throughput);
//! working set and private commit are distinct quantities. The first sample
//! of an identity, a counter decrease, a changed identity, a nonpositive
//! interval or a topology change yields `warming_up` and resets the baseline
//! (AT-06). No negative/NaN/infinite rates are ever produced.

use std::collections::HashMap;

use serde::Serialize;

use crate::identity::ProcessIdentity;
use crate::snapshot::QueryFailure;
use crate::status::MetricStatus;

/// Maximum identities retained per inventory pass (contract 14 §3: at most
/// 4,096 identities; report truncation).
pub const MAX_PROCESS_IDENTITIES: usize = 4096;

/// One process's raw counters from a single scan. Every query is a `Result`
/// so denial/exit are per-fact, never zeros.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessCountersRaw {
    /// Exact OS identity (PID + creation FILETIME).
    pub identity: ProcessIdentity,
    /// Image name (label only; never identity or authorization).
    pub name: String,
    /// Parent PID (label only; the parent may itself be gone or reused).
    pub parent_pid: u32,
    /// `WorkingSetSize` in bytes (resident pages; shared memory can be
    /// counted in multiple processes).
    pub working_set_bytes: Result<u64, QueryFailure>,
    /// `PrivateUsage` in bytes (private committed, not private resident).
    pub private_commit_bytes: Result<u64, QueryFailure>,
    /// Kernel time in 100-ns units.
    pub cpu_kernel_100ns: Result<u64, QueryFailure>,
    /// User time in 100-ns units.
    pub cpu_user_100ns: Result<u64, QueryFailure>,
    /// Cumulative process I/O read bytes.
    pub io_read_bytes: Result<u64, QueryFailure>,
    /// Cumulative process I/O write bytes.
    pub io_write_bytes: Result<u64, QueryFailure>,
}

/// One scan pass over the process table.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessScan {
    /// Raw counters per process (bounded by [`MAX_PROCESS_IDENTITIES`]).
    pub processes: Vec<ProcessCountersRaw>,
    /// True when the table exceeded the identity cap (truncation reported).
    pub truncated: bool,
    /// Whole-scan failure (snapshot/enum failed). Per-process failures live
    /// inside each entry's `Result`s.
    pub scan_failure: Option<QueryFailure>,
    /// Logical processor count at scan time (topology marker).
    pub logical_cpu_count: u32,
}

/// Per-process metric value with status (same invariants as
/// [`MetricEnvelope`]: non-OK never carries a value).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessMetric {
    /// Collection status.
    pub status: MetricStatus,
    /// Byte count, present only when OK.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<u64>,
    /// Rate per second (B/s) or percent of machine capacity, present only
    /// when OK and defined (never negative/NaN/infinite).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
    /// Machine-readable error code when not OK.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl ProcessMetric {
    fn ok_value(bytes: u64) -> Self {
        Self {
            status: MetricStatus::Ok,
            value: Some(bytes),
            rate: None,
            error_code: None,
        }
    }
    fn ok_rate(rate: f64) -> Self {
        Self {
            status: MetricStatus::Ok,
            value: None,
            rate: Some(rate),
            error_code: None,
        }
    }
    fn failed(failure: &QueryFailure) -> Self {
        Self {
            status: failure.to_status(),
            value: None,
            rate: None,
            error_code: Some(failure.error_code()),
        }
    }
    fn warming_up() -> Self {
        Self {
            status: MetricStatus::WarmingUp,
            value: None,
            rate: None,
            error_code: Some("warming_up".to_string()),
        }
    }
}

/// One process row in the rendered inventory.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessEntry {
    /// Exact identity; JSON carries the creation FILETIME as a decimal string.
    pub identity: ProcessIdentity,
    /// Image name (label).
    pub name: String,
    /// Parent PID (label).
    pub parent_pid: u32,
    /// Resident pages, bytes.
    pub working_set: ProcessMetric,
    /// Private committed memory, bytes.
    pub private_commit: ProcessMetric,
    /// CPU as percent of total machine capacity.
    pub cpu_percent: ProcessMetric,
    /// Process I/O read rate, bytes/s.
    pub io_read: ProcessMetric,
    /// Process I/O write rate, bytes/s.
    pub io_write: ProcessMetric,
}

/// Changes between two scans (churn is reported, never guessed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct ChurnSummary {
    /// Identities present now but not in the previous scan.
    pub appeared: u32,
    /// Previous identities no longer present.
    pub vanished: u32,
}

/// A rendered process inventory.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessInventory {
    /// Schema identifier.
    pub schema: String,
    /// RFC 3339 UTC observation time.
    pub observed_at_utc: String,
    /// Snapshot sequence.
    pub sample_sequence: u64,
    /// Logical processor count used for CPU normalization.
    pub logical_cpu_count: u32,
    /// Interval between the two scans used for rates, ms.
    pub rate_interval_ms: u64,
    /// Process rows (bounded; see `truncated`).
    pub processes: Vec<ProcessEntry>,
    /// True when the OS table exceeded the identity cap.
    pub truncated: bool,
    /// Truncation explanation when `truncated`.
    pub truncation_note: Option<String>,
    /// Churn vs the previous scan, when there was one.
    pub churn: Option<ChurnSummary>,
}

/// Baselines for rate computation, keyed by exact identity (AT-06:
/// identity-safe rates — a reused PID has a different key and starts cold).
#[derive(Debug, Clone, PartialEq)]
pub struct RateBaselines {
    inner: HashMap<ProcessIdentity, Baseline>,
    // Identity set of the previous scan (all rows, including rows whose
    // queries failed): churn compares presence, not query success.
    seen: std::collections::HashSet<ProcessIdentity>,
    topology: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Baseline {
    cpu_100ns: u64,
    io_read: u64,
    io_write: u64,
}

/// Outcome of feeding a scan into the rate engine.
#[derive(Debug, Clone, PartialEq)]
pub struct RateOutcome {
    /// Rows in the same order as the scan.
    pub entries: Vec<ProcessEntry>,
    /// Churn vs the previous scan.
    pub churn: ChurnSummary,
}

impl RateBaselines {
    /// Empty baseline set (first scan: everything warms up).
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
            seen: std::collections::HashSet::new(),
            topology: 0,
        }
    }

    /// Advance the baseline state with `scan`, producing rendered entries.
    /// `interval_ms` is the monotonic interval since the previous scan;
    /// the previous scan's topology is compared for reset detection.
    pub fn advance(&mut self, scan: &ProcessScan, interval_ms: u64) -> RateOutcome {
        let topology_changed = self.topology != 0 && self.topology != scan.logical_cpu_count;
        self.topology = scan.logical_cpu_count;

        let mut entries = Vec::with_capacity(scan.processes.len());
        let mut new_baselines: HashMap<ProcessIdentity, Baseline> =
            HashMap::with_capacity(scan.processes.len());
        let mut new_seen: std::collections::HashSet<ProcessIdentity> =
            std::collections::HashSet::with_capacity(scan.processes.len());
        let mut appeared: u32 = 0;

        for raw in &scan.processes {
            let previous = self.inner.get(&raw.identity);
            if !self.seen.contains(&raw.identity) {
                appeared += 1;
            }
            new_seen.insert(raw.identity);
            let entry = render_entry(
                raw,
                previous,
                interval_ms,
                scan.logical_cpu_count,
                topology_changed,
            );
            // Update the baseline from this scan regardless of whether a
            // rate was computable: this scan is now the baseline.
            if let (Ok(k), Ok(u), Ok(r), Ok(w)) = (
                raw.cpu_kernel_100ns.clone(),
                raw.cpu_user_100ns.clone(),
                raw.io_read_bytes.clone(),
                raw.io_write_bytes.clone(),
            ) {
                new_baselines.insert(
                    raw.identity,
                    Baseline {
                        cpu_100ns: k + u,
                        io_read: r,
                        io_write: w,
                    },
                );
            }
            entries.push(entry);
        }

        // Churn compares the identity sets of consecutive scans: a process
        // whose queries failed still exists and must not count as churn.
        let vanished = self.seen.difference(&new_seen).count() as u32;
        self.seen = new_seen;
        self.inner = new_baselines;
        RateOutcome {
            entries,
            churn: ChurnSummary { appeared, vanished },
        }
    }
}

impl Default for RateBaselines {
    fn default() -> Self {
        Self::new()
    }
}

fn render_entry(
    raw: &ProcessCountersRaw,
    previous: Option<&Baseline>,
    interval_ms: u64,
    logical_cpus: u32,
    topology_changed: bool,
) -> ProcessEntry {
    let working_set = match &raw.working_set_bytes {
        Ok(bytes) => ProcessMetric::ok_value(*bytes),
        Err(failure) => ProcessMetric::failed(failure),
    };
    let private_commit = match &raw.private_commit_bytes {
        Ok(bytes) => ProcessMetric::ok_value(*bytes),
        Err(failure) => ProcessMetric::failed(failure),
    };

    let rate_state = rate_computation_state(previous, interval_ms, topology_changed);
    let (cpu_percent, io_read, io_write) = match rate_state {
        RateState::WarmingUp => {
            // No usable baseline yet: queries that failed report their
            // failure; successful queries report warming up.
            let cpu = match (&raw.cpu_kernel_100ns, &raw.cpu_user_100ns) {
                (Err(f), _) | (_, Err(f)) => ProcessMetric::failed(f),
                (Ok(_), Ok(_)) => ProcessMetric::warming_up(),
            };
            let io_read = match &raw.io_read_bytes {
                Ok(_) => ProcessMetric::warming_up(),
                Err(f) => ProcessMetric::failed(f),
            };
            let io_write = match &raw.io_write_bytes {
                Ok(_) => ProcessMetric::warming_up(),
                Err(f) => ProcessMetric::failed(f),
            };
            (cpu, io_read, io_write)
        }
        RateState::Ready { baseline } => {
            let cpu = match (raw.cpu_kernel_100ns.clone(), raw.cpu_user_100ns.clone()) {
                (Ok(kernel), Ok(user)) => {
                    let delta_100ns = (kernel + user).saturating_sub(baseline.cpu_100ns);
                    if kernel + user < baseline.cpu_100ns {
                        // Counter decrease: reset to warming up, never negative.
                        ProcessMetric::warming_up()
                    } else {
                        ProcessMetric::ok_rate(cpu_percent(delta_100ns, interval_ms, logical_cpus))
                    }
                }
                (Err(f), _) | (_, Err(f)) => ProcessMetric::failed(&f),
            };
            let io_read = match raw.io_read_bytes.clone() {
                Ok(current) => {
                    if current < baseline.io_read {
                        ProcessMetric::warming_up()
                    } else {
                        ProcessMetric::ok_rate(per_second(current - baseline.io_read, interval_ms))
                    }
                }
                Err(f) => ProcessMetric::failed(&f),
            };
            let io_write = match raw.io_write_bytes.clone() {
                Ok(current) => {
                    if current < baseline.io_write {
                        ProcessMetric::warming_up()
                    } else {
                        ProcessMetric::ok_rate(per_second(current - baseline.io_write, interval_ms))
                    }
                }
                Err(f) => ProcessMetric::failed(&f),
            };
            (cpu, io_read, io_write)
        }
    };

    ProcessEntry {
        identity: raw.identity,
        name: raw.name.clone(),
        parent_pid: raw.parent_pid,
        working_set,
        private_commit,
        cpu_percent,
        io_read,
        io_write,
    }
}

enum RateState<'a> {
    WarmingUp,
    Ready { baseline: &'a Baseline },
}

fn rate_computation_state<'a>(
    previous: Option<&'a Baseline>,
    interval_ms: u64,
    topology_changed: bool,
) -> RateState<'a> {
    // First sample, nonpositive interval, topology change: warming up and
    // the baseline resets (the caller stores the fresh baseline).
    if topology_changed || interval_ms == 0 {
        return RateState::WarmingUp;
    }
    match previous {
        Some(baseline) => RateState::Ready { baseline },
        None => RateState::WarmingUp,
    }
}

/// CPU as percent of total machine capacity: one fully busy core on N
/// logical cores ≈ 100/N %. Clamped to [0, 100]; never NaN/infinite.
fn cpu_percent(delta_100ns: u64, interval_ms: u64, logical_cpus: u32) -> f64 {
    if logical_cpus == 0 || interval_ms == 0 {
        return 0.0;
    }
    let interval_100ns = interval_ms as f64 * 10_000.0;
    let busy = delta_100ns as f64 / interval_100ns; // cores-worth in [0, ncpus]
    let percent = busy / f64::from(logical_cpus) * 100.0;
    percent.clamp(0.0, 100.0)
}

/// Byte delta to bytes/second over the interval; nonnegative and finite by
/// construction (inputs are nonnegative u64 and interval > 0).
fn per_second(delta_bytes: u64, interval_ms: u64) -> f64 {
    if interval_ms == 0 {
        return 0.0;
    }
    delta_bytes as f64 * 1000.0 / interval_ms as f64
}

/// Compose the full inventory document.
#[must_use]
pub fn build_inventory(
    entries: Vec<ProcessEntry>,
    logical_cpu_count: u32,
    rate_interval_ms: u64,
    observed_at_utc: String,
    sequence: u64,
    truncated: bool,
    churn: Option<ChurnSummary>,
) -> ProcessInventory {
    ProcessInventory {
        schema: "ramshaper.processes.v1".to_string(),
        observed_at_utc,
        sample_sequence: sequence,
        logical_cpu_count,
        rate_interval_ms,
        truncation_note: truncated.then(|| {
            format!("process table exceeded the identity cap ({MAX_PROCESS_IDENTITIES}); excess identities omitted")
        }),
        processes: entries,
        truncated,
        churn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(identity: ProcessIdentity, cpu: u64, io: u64) -> ProcessCountersRaw {
        ProcessCountersRaw {
            identity,
            name: "app.exe".to_string(),
            parent_pid: 4,
            working_set_bytes: Ok(10 * 1024 * 1024),
            private_commit_bytes: Ok(20 * 1024 * 1024),
            cpu_kernel_100ns: Ok(cpu / 2),
            cpu_user_100ns: Ok(cpu - cpu / 2),
            io_read_bytes: Ok(io),
            io_write_bytes: Ok(io / 2),
        }
    }

    fn scan(procs: Vec<ProcessCountersRaw>, cpus: u32) -> ProcessScan {
        ProcessScan {
            processes: procs,
            truncated: false,
            scan_failure: None,
            logical_cpu_count: cpus,
        }
    }

    #[test]
    fn first_scan_is_all_warming_up() {
        let mut baselines = RateBaselines::new();
        let outcome = baselines.advance(
            &scan(vec![raw(ProcessIdentity::new(1, 100), 5_000, 1_000)], 8),
            1000,
        );
        let entry = &outcome.entries[0];
        assert_eq!(entry.cpu_percent.status, MetricStatus::WarmingUp);
        assert_eq!(entry.io_read.status, MetricStatus::WarmingUp);
        assert!(entry.working_set.value.is_some()); // instantaneous values still OK
        assert_eq!(outcome.churn.appeared, 1);
    }

    #[test]
    fn second_scan_of_same_identity_yields_exact_rates() {
        let mut baselines = RateBaselines::new();
        let identity = ProcessIdentity::new(7, 1000);
        let _ = baselines.advance(&scan(vec![raw(identity, 10_000_000, 4096)], 8), 1000);
        // 0.25 s of CPU (2,500,000 × 100 ns) over 1 s on 8 CPUs = 3.125 %.
        let outcome = baselines.advance(&scan(vec![raw(identity, 12_500_000, 2_048_000)], 8), 1000);
        let entry = &outcome.entries[0];
        let cpu = entry.cpu_percent.rate.expect("cpu rate");
        assert!((cpu - 3.125).abs() < 1e-9, "{cpu}");
        let io_read = entry.io_read.rate.expect("io read rate");
        assert!((io_read - 2_043_904.0).abs() < 1e-6, "{io_read}");
        assert_eq!(outcome.churn, ChurnSummary::default());
    }

    #[test]
    fn counter_decrease_resets_to_warming_up() {
        let mut baselines = RateBaselines::new();
        let identity = ProcessIdentity::new(9, 5);
        let _ = baselines.advance(&scan(vec![raw(identity, 20_000_000, 100_000)], 4), 1000);
        // CPU and IO counters went backwards.
        let outcome = baselines.advance(&scan(vec![raw(identity, 10_000_000, 50_000)], 4), 1000);
        let entry = &outcome.entries[0];
        assert_eq!(entry.cpu_percent.status, MetricStatus::WarmingUp);
        assert_eq!(entry.io_read.status, MetricStatus::WarmingUp);
        assert!(entry.cpu_percent.rate.is_none());
    }

    #[test]
    fn reused_pid_is_a_new_identity_and_warms_up() {
        let mut baselines = RateBaselines::new();
        let original = ProcessIdentity::new(42, 111);
        let replacement = ProcessIdentity::new(42, 222); // same PID, new creation
        let _ = baselines.advance(&scan(vec![raw(original, 1_000, 100)], 4), 1000);
        let outcome = baselines.advance(&scan(vec![raw(replacement, 9_000, 900)], 4), 1000);
        let entry = &outcome.entries[0];
        assert_eq!(entry.identity, replacement);
        assert_eq!(entry.cpu_percent.status, MetricStatus::WarmingUp);
        assert_eq!(outcome.churn.appeared, 1);
        assert_eq!(outcome.churn.vanished, 1);
    }

    #[test]
    fn persistently_denied_processes_do_not_count_as_churn() {
        // Regression: a process whose queries are denied every scan (e.g. a
        // protected process) exists in both scans and must not be reported
        // as appearing each time.
        let mut baselines = RateBaselines::new();
        let denied_identity = ProcessIdentity::new(4, 0);
        let healthy_identity = ProcessIdentity::new(600, 555);
        let first = scan(
            vec![
                platform_denied(denied_identity),
                raw(healthy_identity, 1_000, 100),
            ],
            8,
        );
        let second = scan(
            vec![
                platform_denied(denied_identity),
                raw(healthy_identity, 2_000, 900),
            ],
            8,
        );
        let _ = baselines.advance(&first, 0);
        let outcome = baselines.advance(&second, 1000);
        assert_eq!(
            outcome.churn,
            ChurnSummary::default(),
            "no churn on a stable table"
        );
    }

    fn platform_denied(identity: ProcessIdentity) -> ProcessCountersRaw {
        ProcessCountersRaw {
            identity,
            name: "System".to_string(),
            parent_pid: 0,
            working_set_bytes: Err(QueryFailure::AccessDenied),
            private_commit_bytes: Err(QueryFailure::AccessDenied),
            cpu_kernel_100ns: Err(QueryFailure::AccessDenied),
            cpu_user_100ns: Err(QueryFailure::AccessDenied),
            io_read_bytes: Err(QueryFailure::AccessDenied),
            io_write_bytes: Err(QueryFailure::AccessDenied),
        }
    }

    #[test]
    fn topology_change_resets_rates() {
        let mut baselines = RateBaselines::new();
        let identity = ProcessIdentity::new(3, 77);
        let _ = baselines.advance(&scan(vec![raw(identity, 1_000, 100)], 8), 1000);
        let outcome = baselines.advance(&scan(vec![raw(identity, 5_000, 900)], 4), 1000);
        assert_eq!(
            outcome.entries[0].cpu_percent.status,
            MetricStatus::WarmingUp
        );
        // After the reset, the next interval computes normally again.
        let outcome = baselines.advance(&scan(vec![raw(identity, 5_000 + 800, 1_800)], 4), 1000);
        assert_eq!(outcome.entries[0].cpu_percent.status, MetricStatus::Ok);
    }

    #[test]
    fn nonpositive_interval_is_warming_up() {
        let mut baselines = RateBaselines::new();
        let identity = ProcessIdentity::new(5, 9);
        let _ = baselines.advance(&scan(vec![raw(identity, 1_000, 100)], 4), 1000);
        let outcome = baselines.advance(&scan(vec![raw(identity, 2_000, 900)], 4), 0);
        assert_eq!(
            outcome.entries[0].cpu_percent.status,
            MetricStatus::WarmingUp
        );
    }

    #[test]
    fn per_process_denial_is_reported_not_zeroed() {
        let mut entry = ProcessCountersRaw {
            working_set_bytes: Err(QueryFailure::AccessDenied),
            private_commit_bytes: Err(QueryFailure::AccessDenied),
            cpu_kernel_100ns: Err(QueryFailure::AccessDenied),
            cpu_user_100ns: Err(QueryFailure::AccessDenied),
            io_read_bytes: Err(QueryFailure::Exited { code: 87 }),
            io_write_bytes: Err(QueryFailure::Exited { code: 87 }),
            ..raw(ProcessIdentity::new(1, 1), 0, 0)
        };
        entry.identity = ProcessIdentity::new(1, 1);
        let outcome = RateBaselines::new().advance(&scan(vec![entry], 4), 1000);
        let row = &outcome.entries[0];
        assert_eq!(row.working_set.status, MetricStatus::AccessDenied);
        assert!(row.working_set.value.is_none());
        assert_eq!(row.io_read.status, MetricStatus::Exited);
    }

    #[test]
    fn cpu_percent_scales_with_machine_capacity() {
        // One fully busy core: 1 s of CPU over 1 s on 8 logical CPUs ≈ 12.5 %.
        assert!((cpu_percent(10_000_000, 1000, 8) - 12.5).abs() < 1e-9);
        // All 8 cores fully busy = 100 %.
        assert!((cpu_percent(80_000_000, 1000, 8) - 100.0).abs() < 1e-9);
        // Degenerate inputs never produce NaN or infinity.
        assert_eq!(cpu_percent(1, 0, 8), 0.0);
        assert_eq!(cpu_percent(1, 1000, 0), 0.0);
    }

    #[test]
    fn full_machine_saturation_clamps_at_100() {
        // A delta larger than the whole machine over the interval (counter
        // anomaly) clamps instead of reporting an impossible percentage.
        let percent = cpu_percent(1_000_000_000, 1000, 4);
        assert!((percent - 100.0).abs() < 1e-9);
        assert!(percent.is_finite());
    }
}
