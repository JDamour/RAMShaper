# UI overhead budget — proposal for selection (doc 08 §5)

**Date:** 2026-10-24 • **Prepared for:** the doc 08 UI-budget selection (project decision) • **Status:** proposal, not a selected budget

Doc 08 §5 defers the UI budget to "after the Slint prototype". The prototype
(RS-010 spike) was measured 2026-10-24; this memo turns those numbers into a
concrete proposed budget with rationale and guardrails. Nothing here is a
current result against a target — the numbers are one machine's prototype
measurements (64 GiB non-cohort baseline, i9-12900HK, single 100 % display,
software renderer; methods and raw captures in doc 16, RS-010 spike records).

## Measured prototype baseline (2026-10-24)

| Item | Probe-poll 1 Hz | Subscribe (default) |
| --- | --- | --- |
| Working set, steady | 28.66 MiB (peak 28.76 MiB) | 27.77 MiB |
| Private bytes | 11.22 MiB | 11.29 MiB |
| Idle CPU, 60 s | 0.60 % of one core | 0.52 % of one core |
| Binary size | 10.6 MiB unstripped MSVC release | same |

Transport mode is irrelevant to cost (the window is the cost), so one budget
covers both. The hidden window does **not** trim (29.0 → 30.6 MiB after 8 s
minimized, updates continue) — doc 06 requires visibility-aware pausing, which
is required G2 work before any hidden-window claim.

## Proposed budget (visible, steady state)

| Metric | Proposed budget | Rationale |
| --- | --- | --- |
| Working set, mean | < 40 MiB | Measured 28.7 MiB + ~40 % headroom for the Applications page (RS-011), accessibility and DPI variants |
| Working set, peak | < 50 MiB | Measured peak 28.8 MiB; same headroom, guards against retained allocations (doc 08 §5: peaks, not just means) |
| Private commit | < 20 MiB | Measured 11.2 MiB + headroom |
| Idle CPU | ≤ 1 % of one core | Measured 0.60 % worst-case mode; matches the hidden-agent target's per-core style, machine-normalized value would be ≪ 1 % |
| Binary size | ≤ 16 MiB unstripped | Measured 10.6 MiB; packaging (strip) available later if a distribution constraint appears |

All budgets apply to the **visible** UI. A hidden/tray variant (RS-014) is in
scope only if it is separately measured against the same budget while paused;
the un-paused hidden window would currently fail a fairness check because
updates continue while hidden.

## Selection steps (after the owner picks)

1. Write the selected numbers into doc 08 §5 (replacing "selected after the
   Slint prototype") with the date and the decision record in doc 13.
2. RS-025's "repeat UI portion after RS-010" then has a target to measure
   against, using the doc 08 §5 methodology (mean/p95/max, 30-minute idle
   captures, peaks reported).
3. Any regression beyond the budget follows doc 08 §4 (disable implicated
   feature pending review) — for the UI that means pausing features (e.g.
   the Applications page) before shipping.

## Sources

Doc 16 RS-010 spike + post-GO addendum records (measurements, raw captures in
`docs/evidence/2026-10-24-rs010-slint-spike/`); doc 08 §4/§5 methodology and
hidden-agent targets; doc 06 visibility requirement.
