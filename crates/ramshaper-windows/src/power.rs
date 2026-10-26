//! Power/interrupt-time observations (suspend detection for the policy
//! ticker).
//!
//! The pair (biased, unbiased) interrupt time is the documented
//! sleep-detection primitive for a windowless app:
//! `GetTickCount64` **includes** the time the system spends asleep,
//! `QueryUnbiasedInterruptTime` **excludes** it — the difference is the
//! accumulated sleep since boot, and a *change* of that difference between
//! two calls means the system slept in between (modern standby and
//! hibernation both count). This is immune to wall-clock changes (no wall
//! clock involved) and to the agent process itself being suspended (the
//! counters are system-wide and keep running).
//!
//! SAFETY surface: two plain queries, no handles, no writes.

/// `(biased_ms, unbiased_ms)` — both interrupt-time based (100 ns units
/// converted to whole ms), read back-to-back. `None` when the unbiased
/// query fails (the BOOL return is 0); the caller skips the check that
/// tick, never fabricates sleep evidence.
#[must_use]
pub fn interrupt_time_pair() -> Option<(u64, u64)> {
    use windows::Win32::System::SystemInformation::GetTickCount64;
    use windows::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;

    let biased_ms = unsafe { GetTickCount64() };
    let mut unbiased_100ns = 0u64;
    // SAFETY: plain out-pointer query; BOOL 0 = failure.
    let ok = unsafe { QueryUnbiasedInterruptTime(&mut unbiased_100ns) };
    if ok.as_bool() {
        Some((biased_ms, unbiased_100ns / 10_000))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_is_available_and_stable_while_awake() {
        let (biased_a, unbiased_a) =
            interrupt_time_pair().expect("the unbiased interrupt-time query works on this rig");
        std::thread::sleep(std::time::Duration::from_millis(60));
        let (biased_b, unbiased_b) = interrupt_time_pair().expect("second query works");
        // Both counters advanced while the system was awake.
        assert!(biased_b > biased_a, "biased time advances while awake");
        assert!(
            unbiased_b > unbiased_a,
            "unbiased time advances while awake"
        );
        // The sleep-accumulation difference must be stable while awake
        // (no sleep happened between the calls; allow a full second of
        // measurement jitter — a real sleep is far larger).
        let diff_a = biased_a - unbiased_a;
        let diff_b = biased_b - unbiased_b;
        assert!(
            diff_b.saturating_sub(diff_a) < 1_000,
            "no sleep is expected between two awake calls (gap grew by {} ms)",
            diff_b.saturating_sub(diff_a)
        );
    }
}
