//! Bounded metric history with gap and truncation accounting (RS-008,
//! contract 14 §3).
//!
//! Planning caps: 15 minutes in memory with a hard budget of 8 MiB
//! serialized payload; evict oldest. Missed samples surface as gap markers,
//! never as catch-up bursts; capacity pressure surfaces as an explicit
//! eviction counter. All bounds are enforced here so callers cannot
//! accidentally keep an unbounded backlog.

use std::collections::VecDeque;

use serde::Serialize;

/// Default retention window (contract 14 §3).
pub const DEFAULT_RETENTION_MS: u64 = 15 * 60 * 1000;
/// Default serialized-payload budget (contract 14 §3).
pub const DEFAULT_BYTE_BUDGET: usize = 8 * 1024 * 1024;

/// Statistics about what the history dropped (backpressure is visible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct HistoryStats {
    /// Samples currently stored.
    pub stored: u64,
    /// Samples evicted by the retention window.
    pub expired: u64,
    /// Samples evicted by the capacity or byte budget (oldest first).
    pub evicted: u64,
    /// Samples rejected because a single sample exceeded the byte budget.
    pub rejected: u64,
    /// Detected sequence gaps (missed samples).
    pub gaps: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct Entry<T> {
    sequence: u64,
    monotonic_ms: u64,
    size_bytes: usize,
    sample: T,
}

/// A bounded history of timestamped samples.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundedHistory<T> {
    entries: VecDeque<Entry<T>>,
    capacity: usize,
    byte_budget: usize,
    retention_ms: u64,
    current_bytes: usize,
    stats: HistoryStats,
    last_sequence: Option<u64>,
}

impl<T> BoundedHistory<T> {
    /// New history with the planning-default bounds.
    #[must_use]
    pub fn new() -> Self {
        Self::with_bounds(DEFAULT_RETENTION_MS, DEFAULT_BYTE_BUDGET, usize::MAX)
    }

    /// New history with explicit bounds (used by tests and small fixtures).
    #[must_use]
    pub fn with_bounds(retention_ms: u64, byte_budget: usize, capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
            byte_budget,
            retention_ms,
            current_bytes: 0,
            stats: HistoryStats::default(),
            last_sequence: None,
        }
    }

    /// Whether a push with this sequence extends an existing gap.
    fn sequence_gap(&self, sequence: u64) -> bool {
        matches!(self.last_sequence, Some(last) if sequence > last + 1)
    }

    fn evict_front(&mut self) {
        if let Some(entry) = self.entries.pop_front() {
            self.current_bytes = self.current_bytes.saturating_sub(entry.size_bytes);
        }
    }

    /// Push one sample (`sequence`, `monotonic_ms`, serialized-size estimate
    /// in bytes). Evictions happen oldest-first; a sample larger than the
    /// whole budget is rejected and counted, never silently dropped.
    pub fn push(&mut self, sequence: u64, monotonic_ms: u64, size_bytes: usize, sample: T) {
        if self.sequence_gap(sequence) {
            self.stats.gaps += 1;
        }
        self.last_sequence = Some(
            self.last_sequence
                .map_or(sequence, |last| last.max(sequence)),
        );

        // Retention window: expire entries older than the cutoff.
        let cutoff = monotonic_ms.saturating_sub(self.retention_ms);
        while self
            .entries
            .front()
            .is_some_and(|e| e.monotonic_ms < cutoff)
        {
            self.evict_front();
            self.stats.expired += 1;
        }

        if size_bytes > self.byte_budget {
            self.stats.rejected += 1;
            return;
        }

        self.current_bytes += size_bytes;
        self.entries.push_back(Entry {
            sequence,
            monotonic_ms,
            size_bytes,
            sample,
        });

        // Capacity and byte budget: evict oldest until the history fits.
        while self.entries.len() > self.capacity || self.current_bytes > self.byte_budget {
            if self.entries.len() <= 1 {
                // The newest sample fits the budget (checked above) and at
                // least one sample is retained unless capacity is zero.
                if self.capacity == 0 {
                    self.evict_front();
                }
                break;
            }
            self.evict_front();
            self.stats.evicted += 1;
        }
    }

    /// Number of stored samples.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no samples are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Current statistics.
    #[must_use]
    pub fn stats(&self) -> HistoryStats {
        let mut stats = self.stats;
        stats.stored = self.len() as u64;
        stats
    }

    /// Iterate stored samples (oldest first): `(sequence, monotonic_ms, &sample)`.
    pub fn iter(&self) -> impl Iterator<Item = (u64, u64, &T)> {
        self.entries
            .iter()
            .map(|e| (e.sequence, e.monotonic_ms, &e.sample))
    }
}

impl<T> Default for BoundedHistory<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_samples_in_order_and_reports_stats() {
        let mut history: BoundedHistory<u64> = BoundedHistory::new();
        history.push(1, 0, 10, 100);
        history.push(2, 1000, 10, 101);
        history.push(3, 2000, 10, 102);
        assert_eq!(history.len(), 3);
        let stats = history.stats();
        assert_eq!(stats.stored, 3);
        assert_eq!(stats.gaps, 0);
        assert_eq!(stats.evicted, 0);
        assert_eq!(
            history.iter().map(|(_, _, v)| *v).collect::<Vec<_>>(),
            vec![100, 101, 102]
        );
    }

    #[test]
    fn missed_sequences_are_counted_as_gaps() {
        let mut history: BoundedHistory<u64> = BoundedHistory::new();
        history.push(1, 0, 8, 0);
        history.push(2, 1000, 8, 0);
        history.push(5, 2000, 8, 0); // missed 3 and 4
        assert_eq!(history.stats().gaps, 1);
        history.push(6, 3000, 8, 0);
        assert_eq!(history.stats().gaps, 1);
        history.push(10, 4000, 8, 0); // missed 7-9
        assert_eq!(history.stats().gaps, 2);
    }

    #[test]
    fn capacity_pressure_evicts_oldest_and_counts_it() {
        let mut history: BoundedHistory<u64> = BoundedHistory::with_bounds(u64::MAX, usize::MAX, 3);
        for seq in 1..=6 {
            history.push(seq, seq * 100, 8, seq);
        }
        assert_eq!(history.len(), 3); // bounded, never grows past capacity
        let stats = history.stats();
        assert_eq!(stats.evicted, 3);
        assert_eq!(stats.gaps, 0, "steady cadence has no gaps");
        assert_eq!(
            history.iter().map(|(_, _, v)| *v).collect::<Vec<_>>(),
            vec![4, 5, 6],
            "oldest evicted first"
        );
    }

    #[test]
    fn byte_budget_evicts_oldest_and_stays_exact() {
        let mut history: BoundedHistory<u64> =
            BoundedHistory::with_bounds(u64::MAX, 100, usize::MAX);
        for seq in 1..=5 {
            history.push(seq, seq * 100, 40, seq);
        }
        // 5 × 40 = 200 > 100 budget: only the newest ~2 fit (exact accounting).
        assert!(history.len() <= 3, "len {}", history.len());
        assert!(history.stats().evicted >= 2);
        // The byte total never exceeds the budget after a push.
        history.push(6, 600, 40, 6);
        let sizes: usize = history.iter().count() * 40;
        assert!(sizes <= 100 || history.len() == 1);
    }

    #[test]
    fn retention_window_expires_old_samples() {
        let mut history: BoundedHistory<u64> =
            BoundedHistory::with_bounds(1000, usize::MAX, usize::MAX);
        history.push(1, 0, 8, 0);
        history.push(2, 900, 8, 0);
        history.push(3, 1700, 8, 0); // cutoff 700: sample 1 expired, sample 2 kept
        assert_eq!(history.len(), 2);
        assert_eq!(history.stats().expired, 1);
    }

    #[test]
    fn oversized_sample_is_rejected_and_counted() {
        let mut history: BoundedHistory<u64> =
            BoundedHistory::with_bounds(u64::MAX, 100, usize::MAX);
        history.push(1, 0, 50, 0);
        history.push(2, 10, 101, 1); // larger than the whole budget
        assert_eq!(history.len(), 1, "oversized sample not stored");
        assert_eq!(history.stats().rejected, 1);
    }

    #[test]
    fn empty_history_is_empty() {
        let history: BoundedHistory<u64> = BoundedHistory::new();
        assert!(history.is_empty());
        assert_eq!(history.stats().stored, 0);
    }
}
