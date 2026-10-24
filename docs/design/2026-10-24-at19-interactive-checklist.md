# AT-19 interactive pass — tester checklist (RS-010 Overview spike)

**Date:** 2026-10-24 • **Scope:** the human half of AT-19 (doc 15) for the
RS-010 Overview build — keyboard, Narrator and the DPI matrix. The automated
portion (UIA tree present, keyboard activation, stale-state text legibility)
is already recorded in doc 16; this checklist exists so a human can execute
the rest in one sitting and record the captures AT-19 asks for.

**Build under test:** `ramshaper-ui` (release), `ramshaper-agent serve`
(default pipe), both as the standard user, no elevation. Read-only;
Observe Only; no policy controls exist.

**Build note (added after RS-011 landed):** this checklist was generated
against the Overview-only spike build (42-element UIA inventory). The
RS-011 increment added the **Applications** page (nav item + sortable
process table, polled on demand while visible), so the element inventory is
larger and the nav now contains a live control — re-run the UIA tree probe
on the current build and treat the element table below as the **Overview
page** expectations; the Applications page follows the same legibility
principles (per-cell unavailable notes, whole-row accessible labels per the
RS-011 record).

## Setup

1. `ramshaper-agent serve` in one console (keep visible for the stale test).
2. `ramshaper-ui` in another console (stderr visible — state transitions are
   logged there and are part of the evidence).
3. Set the display to **1366×768** at **100 %** scale (the AT-19 reference
   size), Narrator (`Win+Ctrl+Enter`) on, and open the run log
   (`docs/evidence/2026-10-24-rs010-slint-spike/`) for captures.

## Element expectations (from the verified UIA inventory, 42 elements)

Narrator/Inspect must expose, at minimum — all names carry the information;
nothing relies on color alone:

| Element | Expected announcement contains |
| --- | --- |
| Window | `RAMShaper`, ControlType.Window, keyboard-focusable |
| Pressure group (live region) | `Memory pressure` + level + low-memory condition + `Rule pressure_v1 · pending evidence: N signal / M clear observations` |
| Physical memory group | `Total`/`Available`/`In use` with **GiB values** (e.g. `Physical memory available: 36.56 GiB`) |
| System commit group | `Used`/`Limit`/`Headroom` with GiB values; `System commit charge, not pagefile occupancy.` |
| Mode chip | `Mode: Observe Only. RAMShaper will not change process settings.` |
| Header | OS label, `agent: cli-…`, `Data freshness: …` |
| CTA | `Status: Nothing to optimize right now.` (healthy) |
| Footer | `RAMShaper observes and explains; it does not create RAM…` |
| Quit | ControlType.Button, keyboard-focusable |

## Steps

### 1. Keyboard (100 %, 1366×768)

- [ ] Tab from the window reaches the Quit button; focus is visible.
- [ ] Shift+Tab returns; no stop is a dead button (pre-RS-011 the nav items
      were labels; on the current build the Applications nav item is a live
      control — verify it is presented as a real, working navigation target
      and that nothing else presents as clickable without acting).
- [ ] Enter (or Space) on Quit closes the app; exit code 0 in the console
      (`watch` contract: success exit).
- [ ] Relaunch; no state carries over (no persistence in this build).

### 2. Narrator reading (100 %)

- [ ] Reading order follows the visual order: pressure card → physical →
      commit → CTA → footer.
- [ ] Every value announcement carries its unit (GiB) — no bare numbers.
- [ ] The pressure group is announced with its rule evidence line, not a
      percentage and never "Critical".
- [ ] `Inspect` on the pressure group shows LiveSetting = Polite (the .NET
      wrapper probe could not read this — that is why it is here, not in the
      automated portion).

### 3. Stale/disconnected legibility

- [ ] Kill the agent console. Within seconds the UI shows `agent: -`,
      `Data freshness: last updated Ns ago` (growing), and
      `Status: Session agent not reachable — start 'ramshaper-agent serve'.
      Retrying…`. Narrator reads all three; the amber staleness strip is
      additional, never the only signal.
- [ ] Restart the agent. The window returns to `agent: cli-…` and fresh
      values **without user action**; Narrator announces the restored status.

### 4. DPI matrix (repeat at 125 %, 150 %, 200 %; keep 1366×768 physical)

- [ ] All text remains legible and unclipped at each scale; capture a
      screenshot per scale into the evidence directory.
- [ ] The window stays within the screen at each scale. **Known risk to
      evaluate, not a pass assumption:** at 200 % on a 1366×768 physical
      panel the logical desktop is ~683×384, below the window's
      1024×640 minimum — record the actual behavior (clipping/scrolling/
      refusal) rather than passing it.
- [ ] Focus and Narrator still reach every element after each scale change.

### 5. States that cannot be forced on this machine

- Unavailable-metric rendering (`unavailable (status)` per metric) is covered
  by unit tests (`formats_elevated_and_unavailable_states`) but cannot be
  produced live here — GetPerformanceInfo/QueryMemoryResourceNotification do
  not fail on this hardware. Record as covered-by-fixture.
- Pressure `Elevated` transitions require a real low-memory condition; the
  live-region announcement on transition is verified via the LiveSetting
  property (step 2) plus the classifier's fixture tests, not by forcing RAM
  exhaustion.

## Evidence to record

Per-step screenshots + Narrator output snippets into
`docs/evidence/2026-10-24-rs010-slint-spike/at19-interactive/`, then a
summary line in doc 16 and the doc 15 execution-status header (AT-19 Passed
with limitations / Not run — whichever the findings support).
