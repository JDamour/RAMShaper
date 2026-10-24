//! Clock contracts: wall-clock UTC for record timestamps, monotonic for
//! intervals and TTLs (contract 14 §1). Wall-clock time describes records;
//! never use it to measure intervals. A monotonic deadline is not serialized.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Wall-clock source producing Unix epoch milliseconds (UTC).
pub trait WallClock {
    /// Milliseconds since 1970-01-01T00:00:00Z. Returns 0 if the system clock
    /// is before the epoch (does not panic; the value is display-only).
    fn unix_epoch_ms(&self) -> u64;
}

/// Monotonic source for intervals, deadlines and age computation.
pub trait MonotonicClock {
    /// Milliseconds on a monotonic scale within this agent instance. Not
    /// comparable across restarts and never serialized.
    fn monotonic_ms(&self) -> u64;
}

/// Real clocks backed by `std`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

static MONOTONIC_ORIGIN: OnceLock<Instant> = OnceLock::new();

impl WallClock for SystemClock {
    fn unix_epoch_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }
}

impl MonotonicClock for SystemClock {
    fn monotonic_ms(&self) -> u64 {
        let origin = MONOTONIC_ORIGIN.get_or_init(Instant::now);
        u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// Deterministic clock for tests; advance it manually.
#[derive(Debug)]
pub struct FakeClock {
    wall_ms: Cell<u64>,
    mono_ms: Cell<u64>,
}

impl FakeClock {
    /// New fake clock at the given wall/monotonic positions.
    #[must_use]
    pub fn new(wall_ms: u64, mono_ms: u64) -> Self {
        Self {
            wall_ms: Cell::new(wall_ms),
            mono_ms: Cell::new(mono_ms),
        }
    }

    /// Advance both scales by `ms`.
    pub fn advance_ms(&self, ms: u64) {
        self.wall_ms.set(self.wall_ms.get().saturating_add(ms));
        self.mono_ms.set(self.mono_ms.get().saturating_add(ms));
    }

    /// Force the wall-clock position.
    pub fn set_wall_ms(&self, ms: u64) {
        self.wall_ms.set(ms);
    }
}

impl WallClock for FakeClock {
    fn unix_epoch_ms(&self) -> u64 {
        self.wall_ms.get()
    }
}

impl MonotonicClock for FakeClock {
    fn monotonic_ms(&self) -> u64 {
        self.mono_ms.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_advances_both_scales() {
        let clock = FakeClock::new(1_000, 5);
        assert_eq!(clock.unix_epoch_ms(), 1_000);
        assert_eq!(clock.monotonic_ms(), 5);
        clock.advance_ms(250);
        assert_eq!(clock.unix_epoch_ms(), 1_250);
        assert_eq!(clock.monotonic_ms(), 255);
    }

    #[test]
    fn system_clock_is_plausibly_current() {
        let clock = SystemClock;
        // Written 2026; any sane test run is after 2020-01-01.
        assert!(clock.unix_epoch_ms() > 1_577_836_800_000);
        let a = clock.monotonic_ms();
        std::thread::sleep(std::time::Duration::from_millis(15));
        assert!(clock.monotonic_ms() >= a + 15);
    }
}
