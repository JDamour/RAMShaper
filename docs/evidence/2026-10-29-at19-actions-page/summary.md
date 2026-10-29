# AT-19 automated portion — the Actions page inventoried, 2026-10-29

The 2026-10-24 UIA inventory (42 elements) predates the Actions page
(RS-013, 0.3.32, 2026-10-26); this slice extends AT-19's automated
evidence to the current page. The interactive Narrator/DPI pass remains
human.

## Method

`at19-actions-uia.ps1` — the uia-reprobe methodology (async AccessKit
tree-push wait, bounded ControlView/RawView walks, provider diagnostics,
explicit-PID cleanup) against the RELEASE binaries with the agent
**gateless** (production default), so the page renders its
explained-refusal state — which is itself AT-19 scope: stale/unavailable
states must be carried as text, never color-only, never silent.

**Result** (`uia-actions-inventory.txt`, 24 elements, framework=Slint
server-side provider present):

- Navigation: four named buttons, `Actions (current page)` marked, every
  one `focusable=True`; `Quit` focusable.
- `Actions status: 0 action record(s), 0 pending. no policy actions
  exist in this build (observe only)` — the state carried as text.
- `Disable all + restore` button — **focusable=True** (the page's one
  interactive control is keyboard-reachable).
- `Action records: pending, conflict and history` table present.
- The refusal rendered verbatim as element text: "Actions exist only
  when an explicit policy session is enabled; agents start Observe Only
  and a restart restores every outstanding record before serving. An
  agent without the policy gate answers 'unsupported' for the controls —
  the page says so instead of pretending."

No color-only information anywhere in the tree; every state is text.

## The gotcha that cost the first two walks (recorded)

The first probe walked a window that was NOT on the Actions page, and a
second walk landed on Applications — both because **leftover UI
processes from earlier launches** were still running and the walk
resolved its target via `Get-Process ramshaper-ui` (first match), plus a
stale DEBUG ui binary (RS-010-era, pre-Actions-page) whose
`--page actions` error message briefly masqueraded as a live bug. The
clean run asserts exactly **one** UI process before walking. Lesson,
added to the recorded gotchas: **when walking a UIA tree, first assert
the process count; stale windows from prior evidence runs will answer
the probe for you.**

(Also incidentally re-verified: the debug `ramshaper-ui` binary on disk
was stale and has been rebuilt from current source — which walks as
`Actions (current page)` too.)
