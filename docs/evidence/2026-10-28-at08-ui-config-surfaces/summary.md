# AT-08 UI portion — config surfaces end to end, 2026-10-28

The AT-08 row: "Given fresh install, import, profile selection or
restart, when settings load, then mode remains Observe Only, unsupported
schema/fields are rejected and no OS setting changes." Evidence class:
config fixtures + OS call audit. The core/config portion Passed
2026-10-24; this slice completes the UI-side portion (what the UI loads
and renders, and that it cannot mutate anything).

## OS call audit (the UI cannot change an OS setting)

`ramshaper-ui`'s complete OS-touching surface:

- `ramshaper_windows::window::{find_window_by_title, is_iconic}` —
  query-only visibility checks for the hidden-window pausing rule. No
  setters.
- `ramshaper_windows::pipe::{client_connect, write_frame, read_frame}` —
  named-pipe IPC client.
- Zero `unsafe` of its own beyond that usage, no file writes, no
  process/job/priority APIs, no configuration writes (config
  export/import is a CLI-only surface).

Everything the UI knows about settings comes from one read-only IPC
command, `GetConfig` (RS-012 UI portion): a serve-time summary
{mode, profile, exclusions} plus the inertness note — never the config
file. The UI has NO profile-selection control (the Profiles page marks
the active preset; selection lives in config/import by design, and a
profile is not consent). The UI's two policy commands (RestoreAction /
DisableActions, RS-013) reach the agent's consent gate — production
gateless agents answer `Unsupported`; even with a gate, the OS write (if
any) happens in the agent behind typed consent + journal, never in the
UI process.

## Live fixtures (`live-fixtures.txt`, release build at this commit)

1. **Fresh install** — agent with no `--config`: GetConfig serves
   `mode: observe_only`, `profile: balanced`, `exclusions: []` with the
   inertness note. Observe Only survives a fresh install.
2. **Import** — default export → profile switched to `low_ram` +
   exclusion added inside the existing `[exclusions]` table → `config
   import` (validated, atomically replaced) → agent launched with the
   installed file: GetConfig serves `low_ram` + the exclusion, **mode
   still `observe_only`** (a profile/exclusion can never change the
   mode — the closed schema has no such field).
3. **Restart** — agent killed and relaunched on the same file:
   identical summary, mode still `observe_only`. Settings load from the
   file, not from session state.
4. **Rejection** — unknown field (`watch.bogus_field`) and unknown
   profile (`turbo`): both `config import` (exit 2, field-level error)
   and the agent at startup (exit 2) refuse; nothing is half-applied.

The rendered visual (Profiles page marking the active preset, Overview
profile indicator) was live-verified with PrintWindow captures in
`docs/evidence/2026-10-24-rs012-profiles-ui/` — referenced, not redone.

## Verdict

AT-08 UI portion **Passed** (2026-10-28): on fresh install, import and
restart the loaded settings render with mode Observe Only, unsupported
schemas/fields are rejected before anything loads, and the UI process
has no OS-setting mutation path by construction. The G2 gate row's
remaining blockers stay the owner's (Slint license) and the human's
(AT-19 interactive).
