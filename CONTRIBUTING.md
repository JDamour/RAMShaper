# Contributing to RAMShaper

Thanks for looking at the project. This document describes how work is
organized and what a good contribution looks like. **The contribution
process itself is gated on the release review (risk R-09 in
[doc 00](docs/docs/00_IMPLEMENTATION_READINESS.md)) — until packaging and
provenance (RS-027) land, treat this as the contract future contributors
will be held to, and the one the current maintainers follow.**

## Read this first

The numbered documents in [docs/docs/](docs/docs/) are canonical; the
historical research and the master reference are *not* implementation
specifications. In order of authority for code changes:

1. [09 — Security, privacy and safety](docs/docs/09_SECURITY_PRIVACY_AND_SAFETY.md)
   — the mandatory safeguards. If a change conflicts with doc 09, the
   change is wrong.
2. [14 — Runtime contracts](docs/docs/14_RUNTIME_CONTRACTS.md) — exact
   behavior for telemetry, consent, the journal, recovery and IPC.
3. [07 — Roadmap and backlog](docs/docs/07_MVP_ROADMAP_AND_BACKLOG.md)
   — ticket IDs, dependencies and gates. Work is organized by ticket.
4. [15 — Acceptance matrix](docs/docs/15_ACCEPTANCE_TEST_MATRIX.md) —
   the evidence each ticket must produce.
5. [16 — Implementation evidence](docs/docs/16_IMPLEMENTATION_EVIDENCE.md)
   — what has actually been executed, and the defects found on the way.

## Ground rules (non-negotiable)

These are distilled from doc 09 and the maintainer instructions; they
are enforced in review:

- **Never fake results.** No invented test outputs, build results, OS
  API support or benchmark numbers. Report the exact commands you ran
  and what they actually printed.
- **Documented OS APIs only.** No kernel drivers, no process injection,
  no undocumented structures. All `unsafe` FFI lives in the
  `ramshaper-windows` crate, one auditable module per API family.
- **Observe Only is the default.** No silent kills, suspensions,
  working-set trimming, hard memory caps or foreground Realtime
  priority. The single 0.2 policy action is a manually confirmed,
  journal-backed, automatically restored `Normal → Below Normal` change.
- **A profile or an import is not consent.** Policy enabling is a typed
  per-session console act; no recurring policies, no child-process
  grants in this stage.
- **Journal before apply.** Reversible changes are recorded durably
  *before* the OS call; identity is PID + exact creation FILETIME and is
  revalidated through the same handle that writes; exit, PID reuse and
  access denial are expected, typed outcomes — unknown always denies.
- **Job Objects are not sandboxes.** No auto-attaching existing
  processes, no kill-on-job-close.

## How work is organized

- Pick (or propose) a ticket from doc 07 whose dependencies are
  satisfied. Before starting, **check `git log` and the head of
  [docs/CHANGELOG.md](docs/CHANGELOG.md)** — the ticket may have landed
  in a parallel session or while you were reading.
- Every ticket produces **implementation plus evidence**: code, tests,
  a record in doc 16, raw artifacts under `docs/evidence/<date>-<topic>/`,
  an architecture decision (doc 13) when a design choice was made, and a
  [CHANGELOG](docs/CHANGELOG.md) entry.
- Keep slices small and honest about scope: the evidence records have a
  standing "Not in this slice (recorded, not hidden)" section. Gaps are
  data, not embarrassment.

## Gates your change must pass

From the workspace root, with the pinned toolchain (see
[BUILDING.md](BUILDING.md)):

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --target x86_64-pc-windows-msvc --locked
```

The lints deny `unwrap` in OS-facing code (`expect` with a message is
the idiom). Tests run on Windows as a **standard user**; a test that
needs privileges the environment lacks must skip with a printed reason,
never fake a pass.

## Filing issues

Use the templates under `.github/ISSUE_TEMPLATE/`. For bugs, the raw
output matters more than the diagnosis: the tools print typed error
codes (`access_denied`, `exited_87`, `win32_<code>`) precisely so a
report doesn't have to guess. Include your Windows build (`winver`) and
whether a policy-journal agent was running. Redact process listings from
sensitive machines before attaching.

## Style

Match the surrounding code: comments state constraints the code cannot
show, not narration; evidence links are relative; machine-readable codes
are stable strings (never Rust variant names in wire formats); doc
comments carry the contract section a behavior implements.
