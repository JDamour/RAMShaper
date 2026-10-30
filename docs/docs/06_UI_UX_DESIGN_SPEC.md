# RAMShaper — UI/UX Design Specification

**Revision:** 0.2 • **Updated:** 2026-10-22. Source concepts: four generated screens in `../design/`. **These are visual references, not functioning software or proven performance results.** Written interaction requirements below are normative; runtime details follow [14_RUNTIME_CONTRACTS.md](14_RUNTIME_CONTRACTS.md).

## Release visibility

0.1 exposes Overview, Applications, inert Profiles and Safety/Settings. Action Log may show observation errors but has no apply control. The first CLI has no UI. Policy controls arrive only after G3, managed groups after G4, and the complete benchmark wizard in 0.4. Hide unreleased navigation instead of shipping dead buttons. Show capability reasons for unavailable actions within a released feature. Mode **Observe Only** is separate from preset **Balanced**; selecting a profile never enables actions.

## Design intent

Calm, trustworthy system utility with navy navigation, white/light cards, small blue/cyan accents and restrained severity colors. Works at 1366×768 and 1920×1080 on 100–200% scaling. Must remain fast on integrated GPUs, low RAM and HDD systems. No graph animation loops when window hidden. Localizable strings and accessible alternatives to color-only status.

## Navigation and information hierarchy

1. **Overview** — Memory, Contention, Responsiveness, Current Profile, “No action needed” state.
2. **Applications** — processes with RAM/CPU/I/O, category, eligibility, exclusions and explanation.
3. **Managed App Groups** — launch/supervise owned application groups, group utilization and policy.
4. **Profiles** — Balanced, Low-RAM, Focus, Gaming/Observe, Custom. Show exact rules per profile.
5. **Insights & Benchmarks** — pressure timeline, before/after comparisons, confidence, no fabricated percentages.
6. **Actions Log** — every adjustment with reason, outcome, duration, and revert status.
7. **Safety & Settings** — observe-only toggle, denied targets, optional login startup, diagnostics and privacy. Automatic update checks are outside the MVP.

### Visual system (starting tokens, subject to accessibility review)

| Token | Proposal |
|---|---|
| Navigation background | `#15263F` |
| Primary action | `#2563EB` |
| Light surface | `#F6F9FD` |
| Card surface | `#FFFFFF` |
| Primary text | `#152238` |
| Secondary text | `#607089` |
| Healthy | `#0B9A77` (pair with text/icon) |
| Warning | `#B87513` (pair with text/icon) |
| Danger | `#CB4242` (reserved for real risk) |
| Radius | 8–14px desktop cards; avoid giant mobile-like controls |
| Font | OS UI font/native Slint alternatives, measurable legibility |

Use contrast audits and keyboard focus rings. Colors are proposals, not final design tokens.

## Screen specs

### S01 Overview

Header: `RAMShaper` | system name | safe mode state. Main cards show:
- Physical Memory: total, available, in-use (clearly defined)
- Commit: used/limit (not mislabeled swap)
- Pressure: **Normal / Elevated / Unknown**, with pressure_v1 evidence and pending transition state. Critical is reserved and never emitted by v1. Never show arbitrary “28% memory pressure” on Windows.
- Responsiveness: measured p95 input/app-switch latency **only if instrumented**; otherwise “Not measured.”
- Current Profile and Active Interventions with one-click restore.
- Process list; top app memory with correct working-set/private-byte label; timeline.

Primary CTA should say **“Analyze slowdowns”** or **“Review recommendation”**; not “Free 8 GB instantly.” For a healthy system say **“Nothing to optimize right now.”**

### S02 Applications

Table: Application, process IDs (expand), Working Set, Private Commit (when available), CPU (% total machine capacity), Process I/O (B/s), Foreground/Background/Unknown, Eligibility, Current policy. Do not label process I/O as disk throughput. Show unavailable reasons and list truncation. Expand a row for why excluded; never offer bulk changes across critical or unknown targets.

### S03 Managed App Groups

Controls: `Launch a Managed App`, `View Group`, `Remove monitoring`. Accept a local executable and explicit arguments; shortcut/script/broker launches are deferred. A group detail card shows the exact metric name, executable, observed child-count, compatibility warnings and advisory threshold. Explain **“Not a security sandbox”** and no hard limits. Confirm that removing monitoring cannot detach the running process. A lost launch reply is **Outcome unknown**; do not silently retry. After an agent crash, group monitoring may be unavailable even while the app continues.

### S04 Profiles

- Balanced: default display/preferences preset; Observe Only remains the mode.
- Low-RAM: emphasize available/commit trends; no stronger automatic intervention.
- Focus: emphasize current task and an optional explicitly confirmed experiment after G3.
- Gaming: monitor first; game/anti-cheat targets remain excluded.
- Custom: safe display/retention preferences and additional exclusions. Rule tuning and recurring permissions are deferred.

All presets use pressure_v1 defaults initially. In 0.2, a preview shows one exact Normal → Below Normal CPU-priority change, identity, evidence, downside, TTL (120 s default, 10–300 s allowed) and crash-recovery limitation. Confirmations expire after 30 s or eligibility/config changes. No repeat permission or implicit child consent.

**Remove dangerous mockup concepts:** “Memory Cleanup Aggressiveness,” “Cache Trimming Every 5 Minutes,” arbitrary “Minimum Free RAM Target,” “Auto-restart offending processes,” “Keep Container Isolated” if implying security sandbox, and unqualified percent improvement badges.

### S05 Insights & Benchmarks

- Charts separated by metric and unit, with time range, device, run counts, workload and observed baseline.
- Before/After panels show p50/p95 *absolute times* and percentage change where mathematically valid.
- Include uncertainty intervals and a plain-language caveat (measurement is experimental).
- Do not show +23%, -31%, or -42% from concept image as achieved results.
- Export CSV and JSON with local paths removed by default.

### S06 Actions Log

Event example:

```
11:42:03 | Proposed temporary background CPU priority change
Target: OptionalSync.exe (PID + start fingerprint)
Reason: fresh Elevated low-memory evidence; benefit not established
Consent: one confirmed action for this process identity and agent instance
Applied: below-normal priority for up to 120 seconds
Actual result: success
Reverted: 11:42:48 (target became foreground)
```

Shipped status (RS-013, reconciled with the G3 boundary 2026-10-30): the
Actions page renders the agent's `ListActions` records read-only (state,
target, deadline, pending/history badge) and offers **recovery-direction
controls only** — restore one record (`RestoreAction`) and disable
everything (`DisableActions`). There is no apply, preview or consent
control in the UI: the typed `ENABLE` consent stays on the agent console
until G3 sign-off adds a reviewed UI consent flow. Safe-recovery behavior
is preserved exactly as specified under "UI states and errors": a journal
read failure renders as an explicit "records unavailable" state (never an
empty list implying nothing is pending), pending/unresolved records stay
visible with a count, and "All restored" is never displayed while
unresolved records remain.

### S07 First-run onboarding

Step 1: Explain no RAM is created; RAMShaper works with Windows.
Step 2: Choose “Observe Only” (default).
Step 3: Optional non-elevated user agent at login, off by default; no network telemetry.
Step 4: Run a 60–120s optional diagnostic session under normal usage.
Step 5: Show observation evidence and measurement gaps. 0.1 offers no actionable policy recommendation.

## UI states and errors

- Normal: “No low-memory signal observed.” Do not infer absence of other contention.
- Elevated: “Windows is reporting low available memory.” Show evidence, age and missing instruments; do not invent disk/latency attribution.
- Access denied: “Windows does not permit RAMShaper to change this process.”
- Not supported: hide action with capability explanation.
- Reverting: show progress and errors, never falsely claim restoration.
- Restore conflict: “Another change was detected; RAMShaper left it unchanged.” Offer details and investigation, not a blind forced reset.
- Recovery pending: keep a visible count and retry control; disable new actions and never display “All restored.”
- Journal unavailable: observation remains available; explain why actions are disabled.
- Stale telemetry: “Last updated 22s ago”; do not freeze chart and imply live status.
- Benchmark insufficient: “Need more trials” instead of green achievement badges.

## Components and interaction rules

- SideNav with stable icons and compact option; responsive collapse under 1100px.
- MetricCard with name, unit, source, last update, accessible text equivalent.
- LineSeries and BarSeries downsampled to display width; no 60fps polling animation.
- StatusBadge only for meaningful states (e.g., Protected, Applied, Needs Attention).
- Confirmation dialog for persistent profile changes, precise effect and rollback.
- Settings changes are transactional, show unsaved state and reset defaults.
- Tray mini widget read-only by default; hiding UI pauses high-frequency graph updates.

## UX acceptance test

A non-expert participant must distinguish an observed signal from a proven slowdown, identify current mode/action, find disable/restore, and explain whether benefit was measured. Test five participants; record each task's completion and misunderstanding, with a planning goal of at least four unaided completions per task. This is usability feedback, not proof of universal comprehension.

A keyboard-only user must reach every control with visible focus and no trap; dialogs restore focus to their trigger. Narrator must announce labels, values, units and state changes without reading every sample. Charts need a textual/table alternative. Verify 1366×768 and 1920×1080 at 100/125/150/200% DPI, scrolling when necessary without clipping the safety controls. Reduced motion disables nonessential transitions; hidden windows stop graph animation. Audit the proposed color palette before adoption; no contrast compliance is claimed yet. AT-19 records actual evidence.

## Source visual concepts

- `../design/01_dashboard.png`
- `../design/02_app_containers.png`
- `../design/03_analytics.png`
- `../design/04_settings.png`

Treat each screen as reference art; correct labels/actions to this specification before implementation.
