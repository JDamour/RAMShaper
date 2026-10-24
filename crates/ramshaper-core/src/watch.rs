//! Cancellable, rate-limited watch engine (contracts 14 §3 and 04 §B).
//!
//! Design: the low-memory resource condition can stay signaled for a long
//! time, so an unconditional wait on the notification object would spin. This
//! engine never waits on the memory object at all — it queries the (non-
//! blocking) state on a fixed monotonic cadence and waits only on the
//! cancellation handle between ticks. A persistently signaled condition is
//! therefore rate-limited by construction; missed ticks are skipped, never
//! caught up in a burst.

use crate::clock::MonotonicClock;

/// Minimum poll interval. Anything lower would risk a busy loop.
pub const MIN_INTERVAL_MS: u64 = 100;
/// Maximum poll interval (one hour).
pub const MAX_INTERVAL_MS: u64 = 3_600_000;
/// Default poll interval, matching the visible-dashboard cadence (1 s).
pub const DEFAULT_INTERVAL_MS: u64 = 1_000;

/// Validated watch configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchConfig {
    interval_ms: u64,
}

impl WatchConfig {
    /// Create a config, rejecting out-of-range intervals.
    pub fn new(interval_ms: u64) -> Result<Self, String> {
        if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&interval_ms) {
            return Err(format!(
                "interval {interval_ms} ms out of range ({MIN_INTERVAL_MS}..={MAX_INTERVAL_MS})"
            ));
        }
        Ok(Self { interval_ms })
    }

    /// Poll interval in milliseconds.
    #[must_use]
    pub const fn interval_ms(&self) -> u64 {
        self.interval_ms
    }
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            interval_ms: DEFAULT_INTERVAL_MS,
        }
    }
}

/// Outcome of one cancellation wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The cancellation handle was signaled (e.g. Ctrl+C).
    Signaled,
    /// The wait timed out; the next tick is due.
    TimedOut,
}

/// A cancellable wait (implemented over a Windows event on the real adapter).
pub trait WaitHandle {
    /// Block for at most `timeout_ms`, or until signaled.
    fn wait(&self, timeout_ms: u32) -> WaitOutcome;
}

/// Why the watch loop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchTermination {
    /// The user (or programmatic trigger) cancelled the watch.
    Cancelled,
}

/// Loop statistics for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchSummary {
    /// Number of samples emitted.
    pub iterations: u64,
    /// Termination cause.
    pub termination: WatchTermination,
}

/// Run the watch loop.
///
/// `emit` is called once immediately and then on the configured cadence; it
/// performs the (single, in-flight) collection and prints/records the sample.
/// The first argument of `emit` is the 1-based iteration number. Between
/// ticks the loop blocks on `cancel` only — never on the memory condition.
pub fn run_watch<F>(
    config: &WatchConfig,
    clock: &dyn MonotonicClock,
    cancel: &dyn WaitHandle,
    mut emit: F,
) -> WatchSummary
where
    F: FnMut(u64),
{
    let interval = config.interval_ms();
    let mut iterations: u64 = 0;
    // Deadline grid anchor. The grid is advanced by whole intervals only, so
    // a late OS wakeup can never shorten the next interval into a catch-up
    // burst: ticks land on `start + n*interval` or are skipped entirely.
    let mut next_deadline: Option<u64> = None;

    loop {
        iterations += 1;
        emit(iterations);

        let now = clock.monotonic_ms();
        let next = match next_deadline {
            None => now.saturating_add(interval),
            Some(deadline) => {
                let mut candidate = deadline;
                while candidate <= now {
                    candidate = candidate.saturating_add(interval);
                }
                candidate
            }
        };
        next_deadline = Some(next);

        let wait_ms = u32::try_from(next - now).unwrap_or(u32::MAX);
        if cancel.wait(wait_ms) == WaitOutcome::Signaled {
            return WatchSummary {
                iterations,
                termination: WatchTermination::Cancelled,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use std::cell::{Cell, RefCell};

    /// Fake wait handle that advances virtual time on every wait and can be
    /// programmed to report cancellation at a given call number.
    struct FakeWaiter<'c> {
        clock: &'c FakeClock,
        calls: Cell<u32>,
        cancel_at: Option<u32>,
        timeouts: RefCell<Vec<u32>>,
    }

    impl FakeWaiter<'_> {
        fn new(clock: &FakeClock, cancel_at: Option<u32>) -> FakeWaiter<'_> {
            FakeWaiter {
                clock,
                calls: Cell::new(0),
                cancel_at,
                timeouts: RefCell::new(Vec::new()),
            }
        }
    }

    impl WaitHandle for FakeWaiter<'_> {
        fn wait(&self, timeout_ms: u32) -> WaitOutcome {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            self.timeouts.borrow_mut().push(timeout_ms);
            self.clock.advance_ms(u64::from(timeout_ms));
            if self.cancel_at == Some(call) {
                WaitOutcome::Signaled
            } else {
                WaitOutcome::TimedOut
            }
        }
    }

    #[test]
    fn config_rejects_out_of_range_intervals() {
        assert!(WatchConfig::new(MIN_INTERVAL_MS).is_ok());
        assert!(WatchConfig::new(MAX_INTERVAL_MS).is_ok());
        assert!(WatchConfig::new(MIN_INTERVAL_MS - 1).is_err());
        assert!(WatchConfig::new(MAX_INTERVAL_MS + 1).is_err());
        assert_eq!(WatchConfig::default().interval_ms(), DEFAULT_INTERVAL_MS);
    }

    #[test]
    fn persistent_signaling_does_not_spin() {
        // The condition stays "signaled" for the entire (virtual) run; the
        // engine must still emit exactly one sample per interval.
        let clock = FakeClock::new(0, 0);
        let waiter = FakeWaiter::new(&clock, None);
        let config = WatchConfig::new(1_000).expect("valid");
        let emitted = Cell::new(0u64);

        // Virtual run: cancel once virtual time passes 10 seconds.
        let waiter = FixedRunWaiter {
            inner: waiter,
            clock: &clock,
            stop_after_ms: 10_000,
        };
        let summary = run_watch(&config, &clock, &waiter, |_| emitted.set(emitted.get() + 1));

        assert_eq!(summary.termination, WatchTermination::Cancelled);
        // 1 immediate + interval ticks through the 10 s virtual window. A
        // spin loop waiting on a permanently-signaled condition would emit
        // vastly more than this.
        assert_eq!(summary.iterations, 10);
        assert_eq!(emitted.get(), 10);
        assert_eq!(clock.monotonic_ms(), 10_000);
    }

    /// Waiter that reports cancellation once virtual time passes a deadline.
    struct FixedRunWaiter<'a, 'c> {
        inner: FakeWaiter<'c>,
        clock: &'a FakeClock,
        stop_after_ms: u64,
    }

    impl WaitHandle for FixedRunWaiter<'_, '_> {
        fn wait(&self, timeout_ms: u32) -> WaitOutcome {
            let outcome = self.inner.wait(timeout_ms);
            if outcome == WaitOutcome::TimedOut && self.clock.monotonic_ms() >= self.stop_after_ms {
                return WaitOutcome::Signaled;
            }
            outcome
        }
    }

    #[test]
    fn cancellation_while_signaled_is_prompt() {
        let clock = FakeClock::new(0, 0);
        let waiter = FakeWaiter::new(&clock, Some(3)); // cancel on 3rd wait
        let config = WatchConfig::new(1_000).expect("valid");
        let emitted = Cell::new(0u64);
        let summary = run_watch(&config, &clock, &waiter, |_| emitted.set(emitted.get() + 1));
        assert_eq!(summary.termination, WatchTermination::Cancelled);
        assert_eq!(summary.iterations, 3);
        assert_eq!(emitted.get(), 3);
        // Only three waits happened; no extra polling after cancellation.
        assert_eq!(waiter.calls.get(), 3);
    }

    #[test]
    fn slow_collection_skips_missed_ticks_without_burst() {
        // The emit callback itself consumes 3.5 intervals of virtual time per
        // call. Missed ticks must be skipped: the next deadline jumps forward.
        let clock = FakeClock::new(0, 0);
        let waiter = FakeWaiter::new(&clock, Some(4));
        let config = WatchConfig::new(1_000).expect("valid");
        let summary = run_watch(&config, &clock, &waiter, |_| clock.advance_ms(3_500));
        assert_eq!(summary.termination, WatchTermination::Cancelled);
        assert_eq!(summary.iterations, 4);
        // Waits after the slow emits are deadline waits, not immediate ones.
        let timeouts = waiter.timeouts.borrow();
        assert!(
            timeouts.iter().all(|t| *t > 0),
            "no zero-timeout catch-up waits: {timeouts:?}"
        );
    }

    #[test]
    fn waits_use_the_configured_interval() {
        let clock = FakeClock::new(0, 0);
        let waiter = FakeWaiter::new(&clock, Some(2));
        let config = WatchConfig::new(250).expect("valid");
        let _ = run_watch(&config, &clock, &waiter, |_| {});
        let timeouts = waiter.timeouts.borrow();
        assert_eq!(timeouts.first(), Some(&250));
    }

    #[test]
    fn late_wakeups_never_cluster_ticks() {
        // Regression: the OS can wake a wait slightly late. The deadline grid
        // must absorb that by skipping, never by shortening the next interval
        // below the configured cadence (no catch-up bursts).
        struct JitteryWaiter<'c> {
            clock: &'c FakeClock,
            calls: Cell<u32>,
        }
        impl WaitHandle for JitteryWaiter<'_> {
            fn wait(&self, timeout_ms: u32) -> WaitOutcome {
                self.calls.set(self.calls.get() + 1);
                // Wake 17 ms late every time (timer slop).
                self.clock.advance_ms(u64::from(timeout_ms) + 17);
                if self.calls.get() >= 20 {
                    WaitOutcome::Signaled
                } else {
                    WaitOutcome::TimedOut
                }
            }
        }

        let clock = FakeClock::new(0, 0);
        let waiter = JitteryWaiter {
            clock: &clock,
            calls: Cell::new(0),
        };
        let config = WatchConfig::new(500).expect("valid");
        let mut emit_times: Vec<u64> = Vec::new();
        let summary = run_watch(&config, &clock, &waiter, |_| {
            emit_times.push(clock.monotonic_ms());
        });
        assert!(matches!(summary.termination, WatchTermination::Cancelled));
        for pair in emit_times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap >= 500,
                "tick gap {gap} ms is a catch-up burst: {emit_times:?}"
            );
        }
    }
}
