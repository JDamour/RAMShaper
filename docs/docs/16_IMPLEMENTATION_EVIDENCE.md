# RAMShaper — Implementation Evidence (G0, first CLI increment)

**Created:** 2026-10-23 • **Scope:** RS-001 through RS-006 (read-only CLI) • **Gate:** G0 (AT-01–AT-05)

This file records actual commands, outputs and environments for the first
implemented increment. Every entry below was executed; nothing is projected.
JSON fixtures in tests are marked synthetic. Later cases (AT-06+) remain
**Not run** until their slice exists.

## Environment (all entries)

| Item | Value |
|---|---|
| OS | Microsoft Windows 11 Pro, version 10.0.26200, build 26200.9711 (25H2), 64-bit |
| Hardware | 12th Gen Intel Core i9-12900HK, 64 GiB RAM (66,843,784 KB visible) |
| Privileges | Standard user, not elevated (PowerShell `IsInRole(Administrator)` = False) |
| Rust | 1.93.1 (x86_64-pc-windows-msvc), pinned in `rust-toolchain.toml`; rustfmt + clippy installed |
| MSVC/SDK | Visual Studio 2022 (17.14.37012.4, MSVC v143) + VS 18.11.12306.390; Windows SDK 10.0.26100.0 |
| Dependencies | windows =0.62.2 (10 features), serde 1.0.229, serde_json 1.0.154, thiserror 2.0.21; `Cargo.lock` SHA-256 `15778ebba6d3d3d6d648b533d369c22c16020592a47bf18467005d174c22f319` |
| Release binary | `target/x86_64-pc-windows-msvc/release/ramshaper-cli.exe` SHA-256 `cb211cbfd2ffbf511a803a5cc7effc6fe9a77e8ad403a79e1bc01648b94598f6` (241,664 bytes) |

Note on OS metadata: this machine's registry `ProductName` (HKLM …\CurrentVersion)
reads "Windows 10 Pro" while WMI `Caption` reports "Windows 11 Pro" (build 26200,
25H2). The CLI reports the registry value verbatim with the exact build number
("Windows 10 Pro 10.0.26200.9711, 25H2"); it does not rewrite the stale
marketing name. Exactness is preferred over invention.

## Workspace validation (RS-001, all commands exit 0)

```text
cargo fmt --all -- --check                                    PASS
cargo check --workspace --all-targets --locked                PASS (no warnings)
cargo clippy --workspace --all-targets --locked -- -D warnings PASS (no warnings)
cargo test --workspace --locked                               PASS 72 tests, 0 failed
cargo build --release --target x86_64-pc-windows-msvc --locked PASS
```

Test inventory: ramshaper-core 38 (units), ramshaper-platform 8 (units),
ramshaper-windows 3 unit + 11 integration (real OS calls), ramshaper-cli 4
unit + 8 binary-level. CI: `.github/workflows/ci.yml` runs the same five
commands on `windows-latest` plus a standard-user smoke test of the built exe
(version/info/JSON shape/exit codes).

## Case records

### AT-01 — build + launch as standard user (RS-001, RS-004)

Status: **Passed**

- Procedure: clean checkout state (fresh workspace), run the five commands
  above, then launch `ramshaper-cli.exe --version`, `info`, `low-memory-watch`
  from a non-elevated shell.
- Expected/actual: all commands exit 0; binary launches with no service
  installation, no UAC prompt, no admin requirement. `--version` printed
  `ramshaper-cli 0.1.0`, exit 0.
- Limitations: validated on one machine/build (above); no other Windows
  versions tested (R-01 remains open for a support matrix).

### AT-02 — exact page-to-byte conversion + independent raw capture (RS-002, RS-005)

Status: **Passed**

- Boundary fixtures (unit, synthetic): 4 KiB pages × 1,048,576 pages =
  4 GiB exactly; 2 MiB pages × 8192 = 16 GiB; 64 KiB × 16 = 1 MiB; zero page
  size -> `zero_page_size` error; page counts overflowing u64 ->
  `conversion_overflow` error (never wrapped); `available > total` ->
  `inconsistent_counters` on the derived metric while raw readings stay OK.
  All in `ramshaper-core` snapshot/units tests.
- Independent raw API capture (real machine, PowerShell P/Invoke of
  `GetPerformanceInfo` in a separate process, immediately followed by
  `ramshaper-cli info --json`):
  - PageSize: 4096 = 4096 (exact)
  - PhysicalTotal: 16,710,946 pages = **68,448,034,816 bytes = CLI value (exact)**
  - CommitLimit: 17,759,522 pages = **72,743,002,112 bytes = CLI value (exact)**
  - PhysicalAvailable: 38,396,985,344 vs CLI 38,411,304,960 (Δ 13.7 MiB,
    0.037% — live counter, two samples a moment apart)
  - CommitTotal: 33,496,059,904 vs CLI 33,470,926,848 (Δ 24 MiB — same reason)
- Limitations: available/commit drift is expected between any two samples on
  a live machine; only stable counters can match exactly, and they do.

### AT-03 — exit / restart / PID-reuse identity safety (RS-003)

Status: **Passed**

- Windows exit probe (real): test spawns `cmd /c ping -n 20 …` (own fixture),
  resolves identity (PID + creation FILETIME), verifies `Matches` while
  alive, kills and reaps the child, then requires `Unreachable(Exited)` —
  never a false `Matches`. Passed.
- Implementation note discovered during testing: a terminated process can
  remain *queryable* with its original creation FILETIME until the OS
  retires the object. Identity comparison alone therefore reports `Matches`
  for a dead process. `verify_identity` consequently also checks liveness
  (`GetExitCodeProcess` = `STILL_ACTIVE`) **through the same handle**, per the
  same-handle rule in contract 14 §1.
- Fake reuse trace (real process, synthetic marker): verifying a live PID
  against a deliberately wrong creation FILETIME yields
  `Mismatch { observed }` — the exact signature of PID reuse. Passed.
- "Replacement receives zero writes": trivially guaranteed — this increment
  contains no process-setting write path at all (read-only adapter traits;
  no actuator exists).
- Limitations: no genuine OS PID-reuse event was awaited (nondeterministic);
  reuse is covered by the marker-comparison trace above.

### AT-04 — denied/failed queries visible, never zero, partial exit codes (RS-002, RS-005, RS-006)

Status: **Passed** (fixture evidence; no live denial induced on the host)

- Unit/renderer fixtures (synthetic): memory query `AccessDenied` ->
  every byte metric `status=access_denied`, no `value` key in JSON, reason
  preserved, snapshot classifies `NoUsableGlobalSample` -> CLI exit 1;
  low-memory capability `Unsupported` with healthy counters -> exit 3
  (partial); Win32 error (e.g. 1455) -> `error_code=win32_1455`.
- Binary-level: usage errors (unknown command, bad interval) print to
  stderr only and exit 2; `info --json` validated field-by-field
  (integer byte values, UTC timestamps, source, sequence, age) by the CLI
  integration tests; CI repeats these checks.
- Limitations: no real access-denied or API-failure condition was induced on
  the validation host (no fault injection or protected-process targeting was
  performed; host resource policy untouched by design). The unavailability
  path is exercised by deterministic fakes, which is the evidence class the
  case names ("CLI output and error fixtures").

### AT-05 — watch cancellation, handle baseline, no spin (RS-006)

Status: **Passed**

- 1,000 start/stop cycles (real): create low-memory notification object,
  query state, drop (RAII close) × 1000. `GetProcessHandleCount`
  before=99, after=99 — **delta 0**.
- Cancellation while waiting (real): cancel event triggered 400 ms into a
  200 ms-interval watch -> loop returned `Cancelled` after 400.5 ms with 2
  samples; monitor remained healthy afterwards. A genuine console control
  event (`GenerateConsoleCtrlEvent` CTRL_BREAK, same handler as Ctrl+C) sent
  to the release binary's process group: **exit code 0**, 6 snapshots at
  200 ms cadence, "watch cancelled … handles released" on stderr.
- No spin / no catch-up (real + deterministic): 1 s-interval watch cancelled
  after 2.30 s emitted exactly 3 samples. A regression found and fixed
  during validation: anchoring deadlines to the previous emit's end let a
  late OS wakeup shorten the next interval (a 16 ms tick cluster was
  observed in a smoke run); the engine now advances a fixed monotonic grid,
  covered by `late_wakeups_never_cluster_ticks`. Persistently-signaled
  conditions cannot spin by construction (the loop never waits on the memory
  object) and by test (virtual-clock run with a permanently signaled source
  emits exactly one sample per interval).
- Limitations: the host's low-memory condition was not signaled during
  validation; forcing it would require inducing memory pressure, which is
  excluded from this increment. Signaled-path rate limiting is proven
  deterministically at the engine level; the real-OS cadence test bounds
  emission regardless of condition state.

## G0 gate assessment

Required cases AT-01–AT-05: **Passed** with the per-case limitations recorded
above. "No policy/launch endpoints": confirmed — the CLI exposes only `info`,
`low-memory-watch`, `--version`, `--help`; the platform traits are read-only;
no actuator, journal, IPC, service or UI code exists.

G0 evidence is complete **on one identified machine/build**. A release
sign-off should still record a second machine/cohort per R-01 before any
broader compatibility claim; that remains open, as do all later gates.

## Not-run registry (unchanged)

AT-06 through AT-23: **Not run** — they require process telemetry, pressure
classification, UI, policy, storage or launch capabilities that do not exist
in this increment, or hardware cohorts not available here.

---

# G1 (observation increment) — partial evidence, 2026-10-24

**Scope implemented:** RS-007 (process telemetry), RS-008 (bounded history),
RS-009 (`pressure_v1`), the config-schema portion of RS-030 required by
AT-08's core/config case, and RS-024 Stage A / RS-025 harnesses
(`ramshaper-bench observer-overhead`). New crate: `ramshaper-bench`; new core
modules: `process`, `pressure`, `history`, `config`; new CLI command:
`processes` (two-scan rates); `info`/watch output now carries a
`pressure_v1` section; watch exit prints bounded-history stats.

**Environment:** same machine/privileges/toolchain as the G0 entries above
(Windows 11 Pro 10.0.26200.9711, standard user, Rust 1.93.1). New
dependencies: `toml =1.1.8`; `windows` feature add: `Win32_System_Diagnostics_ToolHelp`.

**Workspace validation (all exit 0):** fmt --check clean; check --locked
clean; clippy -D warnings clean; `cargo test --workspace --locked`
**119 passed, 0 failed**; release build OK.
`ramshaper-cli.exe` SHA-256 `71e691bc0f7bd75a664b24f5f223144271646c6d138cd401fdf6bec300f15179`,
`ramshaper-bench.exe` SHA-256 `c41c7908adcb2ffb9b6e68c6e92a297073701f3e19c8f65e85b635e7d0d7c83e`,
`Cargo.lock` SHA-256 `da1ea03fc8b893a2156d00631a23fb70f8a910074a3257847cdaf58da55e2d3a`.

## Case records (G1 set)

### AT-06 — churn / topology / counter reset / bounded queues

Status: **Passed (deterministic fixtures + real-machine spot checks); 24-hour soak Not run**

- Deterministic (unit): first-sample `warming_up`; counter decrease →
  warming-up + baseline reset; reused PID (new creation marker) → new
  identity, cold baseline; topology change → rate reset then recovery;
  nonpositive interval → warming-up; capacity/retention/byte-budget
  eviction oldest-first with explicit counters; sequence gaps counted;
  oversized sample rejected (never stored silently); denied processes
  stable across scans contribute zero churn (regression found on the real
  machine and fixed — churn previously compared against fully-successful
  baselines only, inflating `appeared` to ~137 per scan; now compares
  consecutive identity sets).
- Real machine: 350-row scan, 136 rows `access_denied`/`exited` (protected
  system processes) all reported with per-row statuses, never zeroed;
  churn 0/0 on a stable table; two-scan rates all finite within [0, 100] %.
- **Not run:** the 24-hour soak (G1 exit condition) — out of session scope;
  the longest continuous run is the 191-sample soak below.

### AT-07 — pressure_v1 transitions

Status: **Passed (deterministic, replay-verified)**

- Unit tests implement the contract's rule table exactly: Elevated needs 3
  consecutive signals spanning ≥ 2 s; Normal needs 5 consecutive clears
  spanning ≥ 4 s; pending transitions retain the previous valid level; a
  missing observation resets counts and yields Unknown; `Critical` cannot be
  emitted (does not exist in the type); a 30-step mixed sequence replayed
  twice produces identical outputs.
- Real machine: 191-sample watch (1 s cadence, ~3.2 min) — classifier
  reported `unknown` for the first 4 observations, then `normal` for 187
  (5 valid clears spanning ≥ 4 s, exactly the contract's qualification);
  low-memory state `not_signaled` throughout. A second 60-sample run with
  graceful console-event cancel: sequences 1–60, zero gaps, exit 0, history
  summary `60 stored, 0 gaps, 0 evicted, 0 expired, 0 rejected`.

### AT-08 — core/config portion

Status: **Passed (fixtures; no OS settings touched)**

- Config fixtures (unit): valid document parses with every field; round-trips
  through serialize/parse; unknown fields rejected **by name** (including
  nested `watch.bogus`); missing/unsupported/negative `schema_version`
  rejected; policy modes rejected (only `observe_only` exists);
  out-of-range `interval_ms`/`retention_ms` produce field-level errors;
  malformed TOML reported without partial state; minimal document defaults
  inside `[watch]`.
- Binary-level: `--config bad.toml` (unknown field) and `--config future.toml`
  (schema 2) both exit 2 with stderr diagnostics and empty stdout; a valid
  config's `watch.interval_ms` demonstrably drives the watch cadence
  (stderr banner "every 300 ms", ≥ 2 samples in 900 ms).
- OS-call audit: the config path contains no actuation code of any kind
  (read-only traits; no actuator exists in this codebase), so loading any
  configuration cannot change an OS setting.

### AT-21 — observer overhead on declared cohorts

Status: **Not run (declared 4/8/16 GiB cohorts unavailable); machine-local baseline measured; cohort classification implemented the same day — see [AT-21 cohort classification, 2026-10-24](#at-21-cohort-classification-observe-only-config--bench-2026-10-24) below**

- Harness: `ramshaper-bench observer-overhead` (500 iterations, release
  build, this machine): system snapshot p50 **0.060 ms** / p95 0.083 ms /
  max 0.372 ms; full process scan (~347 processes) p50 **17.0 ms** / p95
  21.8 ms / max 26.7 ms; harness self working set 4.6 MiB, private commit
  ~0.97 MiB. Report schema `ramshaper.observer_overhead.v1` with OS
  metadata and an explicit "not a cohort claim" note.
- These are single-machine observations on a 64 GiB / 20-logical-CPU
  machine (not a declared cohort); NFR-01 targets remain unmeasured.

## G1 gate assessment (as of 2026-10-24)

Implemented and verified here: AT-06 deterministic portion, AT-07 in full,
AT-08 core/config portion, plus a machine-local observer baseline. **Open
items blocking G1 sign-off:** the 24-hour bounded soak (AT-06 evidence), the
declared 4/8/16 GiB cohort matrix (AT-21), and the UI portions of AT-08
(which arrive with the G2 UI slice). Gate: **partially verified — remains
open.**

## Not-run registry (updated)

AT-09 through AT-20 and AT-22/AT-23: **Not run** — policy, journal, IPC, UI,
launch and export capabilities do not exist in this increment. AT-19/AT-20
subchecks that are purely fixture-based for the *future* bench wizard are
also Not run (no wizard exists yet; the Stage A harness is collector-only).

---

# RS-030 — session agent + IPC v1 (portions of G2 prerequisites), 2026-10-24

**Scope implemented:** the session-agent slice of RS-030: agent lifecycle,
bounded/versioned IPC, capability reporting and config schemas. The policy
portions of RS-030 (idempotent action confirmation, journal interplay) remain
gated on G3 and are not implemented. New crate `ramshaper-agent`
(`serve`/`probe` binaries); new core module `ipc` (protocol v1 codec,
envelopes, error codes); new windows modules `event` (manual-reset events),
`token` (SID/session lookup + SDDL DACL), `pipe` (named-pipe server/client +
cancellable accept via `CancelSynchronousIo`), `fs` (atomic
temp+`MoveFileExW` persistence); config gained an `[agent]` section.
`windows` features added: `Win32_System_Pipes`, `Win32_Storage_FileSystem`,
`Win32_Security_Authorization`, `Win32_System_IO`.

**Design (contract 14 §7):** pipe name `\.\pipe\ramshaper-agent-s<session>-v1`
(configurable); explicit DACL from SDDL granting only the serving user; every
accepted client re-validated (logon session via `GetNamedPipeClientSessionId`,
user SID via `OpenProcess`+`OpenProcessToken`); frames are 4-byte LE length +
UTF-8 JSON capped at 1 MiB **before** allocation; version checked before
dispatch; commands `GetCapabilities`, `GetSnapshot` (age attached; stale beyond
2× cadence → `StaleSnapshot`), `SubscribeSnapshots` (bounded queue of 2
pending frames per subscriber with `gap_dropped` markers),
`ListActions` (empty, honest note); connections capped at 8 with an explicit
`Busy` frame; malformed frames get one `Unsupported` error then close, after
`FlushFileBuffers` so the frame is not discarded by the disconnect.
Same-user malware is explicitly **not** isolated (R-07).

**Workspace validation (all exit 0):** fmt --check clean; check --locked
clean; clippy -D warnings clean; `cargo test --workspace --locked`
**151 passed, 0 failed**; release build OK. `ramshaper-agent.exe` SHA-256
`3009506e7b6207d86e65d8cedbae060cb29835b15c1542dc65a8697c6423ff77`.

## Case records

### AT-17 (portions applicable to read-only IPC) — wrong version, malformed/oversized frames, bounded memory

Status: **Passed (same-session real-pipe evidence + deterministic unit tests); cross-session/foreign-user peers Not run locally**

- Real pipe (integration tests, standard user): `GetCapabilities` returns the
  capability set with protocol 1; `GetSnapshot` serves the newest snapshot
  with `age_ms` and the embedded `pressure_v1` state; `SubscribeSnapshots`
  acks then streams deliveries carrying `gap_dropped`; `ListActions` returns
  an empty list; protocol version 2 → `VersionMismatch` (retryable false)
  rejected before dispatch; unknown command → `Unsupported`; malformed
  payload → one `Unsupported` frame, then the connection closes (verified by
  the following read failing); a raw 256 MiB length prefix → the reader
  rejects the prefix **without allocating** and terminates the connection.
- Deterministic unit tests: frame codec round-trip, oversized/zero-length
  prefixes rejected, error-code retryability table, response shape,
  subscriber queue bounded at 2 with gap accounting, `validate_peer`
  verdicts (session mismatch / foreign SID / unresolvable SID → Rejected).
- **Not run locally:** a live cross-session or foreign-user client (needs a
  second logon session or a service client, outside this machine/session);
  remote clients (named pipes are local-machine objects; no remote endpoint
  is created). Those paths are exercised only through `validate_peer` unit
  tests and the DACL construction (SDDL verified: the descriptor is 64 bytes,
  built from the serving user's SID).
- Real binaries: `ramshaper-agent serve --pipe-name ramshaper-live-evidence3
  --interval-ms 400` + `ramshaper-agent probe --subscribe 2`: capabilities ok,
  snapshot ok (age 301 ms, pressure honestly `unknown` on a fresh instance),
  6 streamed snapshots in 2 s; Ctrl+Break (same handler as Ctrl+C) →
  **exit code 0**, "handles released".

**AT-17 update (2026-10-24, later — adversarial completion of the same-session
portion):** three cases added, all green (`docs/evidence/2026-10-24-at17-ipc-adversarial/`,
13 integration + 110 core tests, fmt/clippy `-D warnings` clean):

- **Malformed framing end-to-end:** a zero-length length-prefix (framing
  violation, not a parseable request) ends the connection silently — the
  server cannot correlate it to a request id, so it answers no frame and
  closes; verified by the client's subsequent read failing
  (`malformed_framing_prefix_ends_the_connection_silently`).
- **Replay characterized (accepted v1 limitation):** protocol v1 has no
  replay cache — `request_id` is a correlation id, not an anti-replay token.
  This is accepted because every v1 command is read-only and idempotent, so a
  replay is a benign re-issue, never a state mutation. Pinned by
  `replayed_request_is_served_identically_on_the_same_connection`: the same
  frame served twice returns two identical successful responses with the same
  correlation and identical data. A future protocol v2 must not rely on v1
  replay behavior if state-mutating commands (policy stage) are added.
- **Response-side frame cap:** `response_frame` refuses a payload over the
  1 MiB cap with `Oversized` (previously only the request side and the
  success path were directly asserted; `response_frame_rejects_payload_over_cap`).

The cross-session/foreign-user and remote-client portions remain **Not run
locally** for the same reason as above (single interactive session, standard
user; named pipes are local-machine objects) — covered by the `validate_peer`
unit tests and the explicit user-only DACL, recorded as the documented
same-user limitation the AT-17 contract expects for IPC.

### Defects found and fixed during this slice (recorded, not hidden)

1. **Double framing:** `response_frame` originally returned length-prefixed
   bytes that `write_frame` framed again; clients received `[len][len][json]`
   and JSON parsing failed at column 1. Fixed by making `response_frame`
   produce the payload only (transport owns framing).
2. **Uncancellable accept:** `ConnectNamedPipe` on a synchronous-mode
   instance ignores the OVERLAPPED event, so a stop event could never wake
   the accept loop and shutdown hung on join. Fixed with the documented
   `CancelSynchronousIo` on the accept thread's duplicated handle; abort is
   mapped to a clean stop.
3. **Lost final frames:** `DisconnectNamedPipe` discards unread buffered
   data, racing the client's read of the final error frame. Fixed with
   `FlushFileBuffers` before disconnect on close-after-write paths.
4. **Bare pipe names:** a user-supplied name without `\.\pipe\` made
   `CreateNamedPipeW` fail with error 123; names are now normalized.

## Gate assessment

This closes the IPC/config portion of RS-030 required before RS-010 (UI).
G2 itself remains open: the Slint UI, its feasibility evidence (R-02) and
AT-17's cross-session/live-adversary portions are not done. G1's open items
(24-hour soak, declared cohorts) are unchanged.

---

# AT-21 cohort classification (observe-only config + bench), 2026-10-24

**Scope implemented:** the *classification* half of AT-21 (RS-025): declared
4/8/16 GiB hardware cohorts in the closed TOML config schema, a pure
classifier that stamps every observer-overhead observation with its cohort,
and the refusal path that prevents a machine outside a cohort from being
substituted into one. This does **not** run the AT-21 cohort matrix — no
4/8/16 GiB machine exists in this session — and it does not change any
collection behavior; the config stays observe-only and the classifier is
labeling and refusal only.

**Implementation:**

- New pure module `ramshaper-core::cohort`: `HardwareCohort` (`4gib`,
  `8gib`, `16gib`, `none`), cohort membership bands `[4/5, 6/5) × nominal`
  (±20%, exact rational bounds; bands of adjacent cohorts are 2× apart and
  can never overlap), `from_total_bytes` classification of a *measured*
  total physical memory reading, and `verify_declared` cross-checking a
  *declared* cohort against that reading. `none` always verifies (a
  baseline claims nothing); a named cohort verifies only on a machine whose
  measured memory classifies into exactly that cohort.
- Config schema (RS-030, closed): new `[cohort]` section with one field,
  `declared = "4gib" | "8gib" | "16gib" | "none"`; absent section defaults
  to `none`; unknown declaration strings and unknown fields are rejected
  with field-level errors (`unsupported cohort declaration '…'`,
  `unknown field 'cohort.…'`). Parsing accepts a declaration without
  hardware knowledge — verification is measurement-time, since core makes
  no OS calls.
- `ramshaper-bench observer-overhead` gains `--config <PATH>` (default: no
  config = `none` = baseline) and a `cohort` block in every report:
  `declared`, `measured` (what the machine's memory classifies as, or
  `unknown`), `total_physical_bytes` (auditable) and `status`
  (`baseline` | `verified`). A declared/measured mismatch or an
  unverifiable named cohort prints the reason to stderr and exits 2
  (configuration error) — no report is emitted. The report `note` now
  distinguishes a non-cohort baseline from a verified-cohort run.

**Tests added (19):** 12 in `cohort.rs` (nominal classification, band edges
inclusive-low/exclusive-high, non-overlap proof, in-between sizes 2/6/12/32
GiB and `u64::MAX` → non-cohort, the validation machine's recorded
68,448,034,816 bytes → non-cohort, reservation-shortfall membership,
declaration round-trips, serde strings, `none` always verifies, matching
cohort verifies, **a better machine cannot substitute for a declared
cohort** — the 64 GiB machine declaring 4/8/16 is refused, and the reverse
direction is refused too — and no-reading refusal), 5 in `config.rs`
(all four declarations parse and round-trip, absent/empty section → `none`,
unknown declaration → field error, unknown field → `cohort.…` error,
declaration accepted without machine verification), 2 in the bench
(baseline report block with and without a reading; declared cohort on the
64 GiB machine refused).

**Workspace validation (2026-10-24, same environment as above):**

```text
cargo fmt --all -- --check                                    PASS
cargo check --workspace --all-targets --locked                PASS
cargo clippy --workspace --exclude ramshaper-ui --all-targets
  --locked -- -D warnings                                     PASS
cargo test --workspace --locked                               PASS 173 tests, 0 failed
cargo build --release --target x86_64-pc-windows-msvc --locked PASS
```

`cargo test` went from 151 to **173 passed, 0 failed**: +19 AT-21 tests and
+3 from the G2 UI feasibility spike crate (`ramshaper-ui`), which was added
to the workspace after the RS-030 record above and is excluded from the
clippy command because **pre-existing** Slint-*generated* code (not this
work) trips the workspace-wide `clippy::unwrap_used` deny (173 hits in
`target/…/out/app-window.rs`); resolving that lint boundary is G2 scope.
(Lifted later the same day, still 2026-10-24: the spike scopes the allow to
the generated module — see the RS-010 spike record below; clippy now passes
`--workspace` including `ramshaper-ui`.)
To make the ui spike compile at all (it previously failed
`check/test --workspace`), this increment applied three mechanical,
compiler-suggested fixes to its test fixture and calls — `u64` suffixes on
six oversized JSON-fixture literals, `UiState<'_>` turbofish lifetimes, and
unused-binding cleanup — changing no logic. No dependencies were added by
this increment; the `Cargo.lock` hash below
changed only because the ui spike brought `slint =1.18.1` into the graph.
Binaries: `ramshaper-bench.exe` SHA-256
`226d04f554ba51b31daa2c7d69a4e4af1309884d9d0b4aff95a7d48db257fb46`;
`ramshaper-cli.exe` and `ramshaper-agent.exe` rebuilt (unchanged sources
plus the new core module) `1a74f32d8223bec5bd438584e952ddb094a9c0ac35bb11bd41b908923d8e71a8`
/ `ebd824e9e6388569096d62f35dcd438be7f41a66122d980d641b219a2dd232c2`;
`Cargo.lock` SHA-256
`692e9fd8f846e4a24d4995914cecbff0c4b9acf7547c9ba3ccf15d05bcb32751`.

## Live classification evidence (this 64 GiB machine)

1. **Explicit baseline declaration** (`[cohort] declared = "none"`,
   `--iterations 500`, release build) — report classified, exit 0. The
   machine's measured total (68,448,034,816 bytes = 63.75 GiB nominal 64)
   classifies as `none`:

   ```json
   "cohort": {
     "declared": "none",
     "measured": "none",
     "total_physical_bytes": 68448034816,
     "status": "baseline"
   },
   "note": "machine-local observer-overhead measurement; non-cohort baseline, not a cohort claim"
   ```

   Latencies from the same run: system snapshot p50 0.056 ms / p95
   0.077 ms; process scan (439 processes) p50 19.4 ms / p95 32.0 ms;
   bench self working set 5,570,560 bytes.
2. **No-substitution refusal** (`[cohort] declared = "8gib"` on the same
   machine): exit **2**, no report, stderr:
   `error: declared cohort '8gib' does not match this machine (measured
   total physical memory classifies as 'none'); a machine outside a cohort
   cannot substitute for it`. Same for any other declared cohort.
3. **No config** (default): classified `none (baseline; measured none,
   total physical 68448034816 bytes)` — a cohort claim requires an explicit
   declaration, never a default.

## What a 64 GiB machine can and cannot claim about cohort behavior

**Can (and does, in the records above):**

- Produce **machine-local, non-cohort baseline** measurements of its own
  observer overhead, permanently labeled `cohort.status = "baseline"` with
  the measured total bytes attached for audit.
- Verify the AT-21 *machinery* end to end: classifier band edges, closed
  config schema, report stamping, and the refusal path — behavior
  correctness evidence, not cohort performance evidence.
- Prove the substitution guard live: this machine cannot stamp any
  declared cohort onto its observations, by construction.
- Bound its own regressions across builds (same machine, same baseline
  label), which is what the G0/G1 baseline comparisons above do.

**Cannot (and is refused, or must not be attempted):**

- Claim **any** 4/8/16 GiB cohort behavior: no statement that overhead
  distributions, working sets or collection costs on low-memory machines
  resemble these numbers. Physical page counts, process counts, commit
  pressure and eviction behavior differ by memory size; nothing here
  measures a cohort machine.
- Stand in for a declared cohort in the AT-21 matrix ("without
  substituting a better machine"): AT-21 passes only when each declared
  cohort is measured on hardware that actually classifies into it.
- Claim NFR-01 target status for any cohort; the targets remain unmeasured
  here, on this machine and everywhere else.
- Publish its numbers *as* cohort-adjacent evidence: any use must carry the
  "non-cohort baseline on a 64 GiB machine" label the tool attaches
  automatically.

**Status:** AT-21 declared-cohort measurements remain **Not run** (4/8/16
GiB cohorts unavailable); the classification portion is implemented,
tested and live-verified on the baseline path. G1's cohort-matrix exit
condition is unchanged.

---

# RS-010 (spike portion) — Slint UI feasibility for G2 (R-02), 2026-10-24

**Scope implemented:** the feasibility half of RS-010 — a working Slint
Overview window (`crates/ramshaper-ui`, its own crate; observer, agent and
core untouched) driven by the session agent over protocol v1, plus the
overhead/robustness evidence R-02 asks for before G2. New UI crate
`ramshaper-ui` (slint 1.18.1 pinned exactly: `std`, `backend-default`,
`renderer-software`, `accessibility`, `compat-1-2`; `slint-build` in
build.rs). The client connects, handshakes `GetCapabilities`, and by default
subscribes to pushed snapshots; `--poll-ms <MS>` switches to one-shot
`GetSnapshot` probes on the same connection (the `ramshaper-agent probe`
request pattern) for the polling-cost measurement. State transitions are
logged to stderr; the window shows the serving `agent_instance_id`, so an
agent restart is visible in the UI. Renders live system resident-memory
counters (physical total/available/in-use, commit used/limit/headroom) and
the `pressure_v1` classification with pending-evidence counts. Read-only,
Observe Only, no policy controls. Keyboard/Narrator/DPI validation (AT-19)
is **not** part of this spike.

**Method:** agent `serve` (1 Hz cadence) and `ramshaper-ui.exe` (release,
MSVC) on the 64 GiB dev machine (i9-12900HK, 20 logical processors),
standard user, both under the interactive session. CPU = Δ
`TotalProcessorTime` over a 60 s wall sample; memory = `Get-Process`
working set / private bytes at steady state. Raw captures in
[evidence](../evidence/2026-10-24-rs010-slint-spike/) (client logs, agent
logs, desktop capture).

**Results:**

| Evidence item | Probe-poll 1 Hz (`--poll-ms 1000`) | Subscribe (default) |
| --- | --- | --- |
| Binary size | 11,116,032 B (10.6 MiB) unstripped MSVC release; debug info external to a 3,985,408 B PDB. Agent 0.77 MiB, CLI 0.61 MiB for comparison | same binary |
| Idle CPU, 60 s | 0.359 CPU-s = **0.60% of one core** (0.030% of the 20-LP machine) | 0.312 CPU-s = 0.52% of one core |
| Working set (steady) | 28.66 MiB (peak 28.76 MiB) | 27.77 MiB |
| Private bytes | 11.22 MiB | 11.29 MiB |
| Live rendering | Desktop capture shows pressure card (OK / Normal / not signaled / "0 signal / 51 clear observations"), physical memory (63.75 GiB total / 37.67 GiB available / 26.08 GiB in use), commit, `agent: cli-…` and "just updated" | same |

The two transports cost the same: the spike's idle cost is the window
itself (1 Hz UI updates + freshness ticker), not the IPC mode. Polling via
probe is therefore viable as a simpler alternative to subscription if a
future slice wants it.

**Agent restart handling (probe mode, stderr trail):** kill -9 of the agent
was noticed within one poll interval — the next probe write failed
(`OS error 232 from WriteFile`), the UI flipped to its disconnected state
and retried every 1.5 s (`OS error 2 from CreateFileW` while the name was
gone). After the agent returned, the UI reconnected on the next retry and
resumed polling against the **new** agent instance id
(`cli-37472-…` → `cli-7672-…`), all without user action:

```
[ui +  262.1s] disconnected: write failed: OS error 232 from WriteFile
[ui +  283.2s] pipe connected; handshaking GetCapabilities
[ui +  283.2s] connected: agent instance cli-7672-134373157792892953
[ui +  283.2s] probe-polling GetSnapshot every 1000 ms
```

**Gate checks at spike time:** `cargo fmt --check`, `cargo clippy
--workspace --all-targets --locked -- -D warnings`, `cargo test
--workspace --locked` (174 tests) and `cargo build --release` all pass with
the UI crate in the workspace. Snapshot v1 carries system-level resident
memory only; per-process RSS is not in protocol v1 and was not added (that
would touch the observer, out of the spike's scope).

**Decision (R-02 feasibility half): GO** — continue Slint for G2. All four
feasibility concerns pass with margin: 10.6 MiB binary (no distribution
constraint exists for an internal tool; strip/packaging can shrink it
further), 0.60% of one core idle, <29 MiB working set against a UI budget
still to be selected in doc 08, and automatic reconnection with a visible
instance change. **Open, owner-gated (R-02 halves not covered here):** the
Slint license selection (GPL-3.0-only / Royalty-free-2.0 / commercial —
ADR-002/ADR-012 remain open decisions, nothing is distributed), and the
AT-19 accessibility pass (keyboard/Narrator/DPI at 1366×768 and
100/125/150/200%).

**Limits of this evidence:** one machine (64 GiB non-cohort baseline), one
run per transport, software renderer only (no femtovg/skia comparison), and
no long-run soak of the UI itself. The UI budget in doc 08 remains
"selected after the Slint prototype" — these numbers are that prototype's
input, not a claim against any target.

## RS-010 post-GO addendum — DPI, UIA and hidden-window probes, 2026-10-24

Three additional single-run probes (scripts and raw outputs in
[evidence](../evidence/2026-10-24-rs010-slint-spike/)):

- **DPI awareness** (`capture-footprint-dpi-uia.ps1` → `ui-dpi-awareness.txt`):
  the UI process reports `PROCESS_PER_MONITOR_DPI_AWARE` (value 2) via
  `GetProcessDpiAwareness` — the prerequisite state for the AT-19
  100/125/150/200 % matrix. The legacy enum does not distinguish
  per-monitor V1 from V2, and per-monitor *behavior* was not exercised
  (single 100 % display on this machine).
- **UIA control-view tree** (`capture-uia-scope-fix.ps1` →
  `ui-uia-controlview-tree.txt`): the window is exposed to UI Automation
  with the correct name (`RAMShaper`, `ControlType.Window`,
  keyboard-focusable), but a depth-4 `ControlViewWalker` enumeration
  returned **zero child elements** — the Slint 1.18 accessibility bridge
  does not surface the spike's child controls in this configuration. This
  is the concrete blocker for the AT-19 Narrator pass and must be resolved
  (Slint version/feature investigation or an alternative exposure path)
  before AT-19. Method note: the first, unbounded walker run hung and was
  abandoned; the recorded run is bounded (depth 4, 30 s job budget) and
  completed well inside it, so the empty tree is a real observation, not a
  truncated capture.
- **Hidden-window footprint**
  (`ui-footprint-visible-vs-minimized.txt`): after 8 s minimized
  (`SW_MINIMIZE`), working set 29.0 → 30.6 MiB and private bytes
  11.4 → 11.9 MiB — no trimming, and the 1 Hz updates plus freshness
  ticker keep running while hidden. Doc 06 requires hidden windows to stop
  high-frequency updates; the spike does not implement visibility-aware
  pausing, so this is recorded as required G2 work, not a pass.

Fresh full gates after the addendum captures: `cargo fmt --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings` (now
**including** the ui crate — the earlier `unwrap_used` exclusion is
obsolete because the generated Slint module is wrapped in a scoped
`#[allow(clippy::unwrap_used)]`), and `cargo test --workspace --locked` —
**175 passed, 0 failed, 16 suites**.

---

### Correction (same day, later): the UIA zero-children finding is superseded — the accessibility tree is present

Re-probe with a corrected client script (`uia-reprobe.ps1` →
`uia-reprobe-result.txt` in the same evidence directory): the original
capture's walk called `TreeWalker.GetFirstChildElement` /
`GetNextSiblingElement`, which **do not exist** on the .NET
`UIAutomationClient.TreeWalker` (the real method names are `GetFirstChild` /
`GetNextSibling`). With `ErrorActionPreference='Continue'` the per-call
`MethodNotFound` errors went to the job's error stream and never into the
result file, so the capture recorded only the window line: **no child
enumeration ever ran** in the original probe.

With corrected method names the same window exposes **42 elements** in the
control view, reproduced in three independent passes (background query +
3 s settle, foreground + 3 s, and raw view): the Memory-pressure,
Physical-memory and System-commit groups with every annotated accessible
label, the Quit button (`focusable=True`), the Observe-Only mode chip,
freshness and serving-agent-instance labels; live metric values update
between passes. Window diagnostics: `FrameworkId='Slint'`,
`IsControlElement=True`, `HasServerSideProvider=True` — a live Slint
1.18.1/AccessKit server-side UIA provider, not the HWND fallback.

Conclusion: the Slint accessibility bridge works in this spike's build
configuration, and the "zero children" blocker is **withdrawn**. AT-19
remains open for its real substance — the interactive keyboard-navigation,
Narrator-announcement and DPI-matrix pass — not for a missing UIA tree.

---

### AT-19 automated portion — keyboard, live-region and stale-state probes, 2026-10-24

Script `at19-keyboard-states.ps1` → `at19-results.txt` + `at19-stale-state.png`
(same evidence directory), against the release build and a dedicated agent
instance:

- **Keyboard (A1–A3):** initial focus lands on the window; one Tab moves
  focus to the Quit button (the only focusable control in the read-only
  Overview, matching the spec's no-dead-navigation rule); Enter on the
  focused button exits the application **gracefully with exit code 0**,
  consistent with the CLI exit-code contract.
- **Stale/disconnected legibility without color (C):** with the agent
  killed, the UIA text inventory (38 elements) shows every state carried in
  text: `Serving agent instance: agent: -`, `Data freshness: last updated
  11s ago`, `Status: Session agent not reachable — start
  'ramshaper-agent serve'. Retrying…`, plus all metric values with units.
  The 3-px staleness strip is additional and non-exclusive. Screenshot
  `at19-stale-state.png` (window partially overlapped by the launcher
  terminal; the UIA text inventory is the authoritative record).
- **Live-region (B): inconclusive by tooling** — the pressure card declares
  `accessible-live-region: polite` in the .slint source, but the .NET
  Framework `UIAutomationClient` wrapper available to PowerShell 5.1
  (4.0.0.0 GAC assembly) exposes neither `LiveSettingProperty` nor a
  `LookupById(33155)` mapping, so the machine check cannot read the value.
  Verify during the interactive Narrator pass.
- **Remaining for AT-19 (interactive, human):** Narrator announcement
  quality and live-region behavior, the full keyboard walk at 1366×768,
  and the 100/125/150/200 % DPI matrix (single 100 % display here — the
  per-monitor-aware process state from the addendum above is the
  prerequisite, not the evidence).

---

# RS-012 — inert profiles/exclusions + schema-validated atomic import/export, 2026-10-24

**Scope implemented:** the M2 configuration slice of RS-012. New closed-schema
sections in `ramshaper-core::config`: `[profiles] active` (presets
`balanced`, `low_ram`, `focus`, `gaming_observe`, `custom`; default
`balanced`) and `[exclusions] processes` (user exclusion list; names only —
no paths, no control characters; ≤ 512 entries of ≤ 260 chars). Both are
**inert by construction**: the profile is a stored label (contract 14 §5 —
presets share the same defaults in this slice and selecting one never grants
write permission), and no actuation path exists; the mandatory exclusions of
contract 14 §5 are eligibility rules, not config values, so no import can
weaken them.

**Import/export (CLI):** `ramshaper-cli config export [--config <PATH>]
<DEST>` writes validated TOML to DEST; `config import <SOURCE> --into
<DEST>` validates SOURCE against the closed schema (field-level errors) and
atomically replaces DEST via the existing `fs::atomic_write` (temp +
`MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`), then writes the canonical
re-serialization. Contract 14 §7 points hold: unknown fields/unsupported
versions/malformed entries are field-level errors (exit 2) and a rejected
import leaves the destination byte-identical (retain-last-valid); the parsed
mode must be `observe_only` (policy modes rejected before any write); no
journal exists, and nothing but the named file is touched. Startup stays
Observe Only by construction.

**Tests:** 12 new unit tests (presets closed set + roundtrip + inertness;
exclusion shape/count/roundtrip; unknown-field rejection per section) and 5
new binary-level tests (export→import byte-identical roundtrip; custom
source roundtrip incl. profile and exclusions; rejected import keeps DEST;
`--into` and policy-mode rejections with no DEST created; usage errors +
help). Workspace at record time: **202 tests, 0 failed**, fmt and clippy
(`-D warnings`) clean.

**Live demo** (`docs/evidence/2026-10-24-rs012-inert-profiles/
demo-transcript.txt` + exported/imported files): default export shows
`[profiles] active = "balanced"`; a custom source with `active = "low_ram"`
and two exclusions round-trips (`imported … (mode: observe_only, profile:
low_ram)`, exit 0); an `active = "turbo"` source is refused with exit 2
(`unsupported profile 'turbo'; valid profiles: …`) and the destination file
is not created.

**Limits:** release-binary rebuild is deferred while the AT-06 soak holds
`ramshaper-cli.exe` (os error 5) — the demo and binary tests used the debug
build, which compiles from the same sources; per-process exclusion *matching*
semantics are a G3 eligibility concern and deliberately not defined here.

---

# RS-010 continuation — visibility-aware pausing (doc 06 hidden-window rule), 2026-10-24

**Gap closed:** the post-GO addendum recorded that a minimized window does
not trim and keeps 1 Hz updates — required G2 work. Implemented in
`ramshaper-ui`: a plain std::thread polls `IsIconic` on the window every
500 ms (new `ramshaper-windows::window` module — `FindWindowW` by title +
`IsIconic`, the workspace's `Win32_UI_WindowsAndMessaging` feature added;
unsafe FFI stays in `ramshaper-windows` per the audited-boundary rule) and
sets a shared flag. While minimized: subscribe pushes are skipped (the
stream is still drained — the agent's bounded subscriber queue would
otherwise mark gaps), probe-poll mode sends nothing at all, the freshness
ticker skips its label update (the stale flag recomputes truthfully from
the same timestamp on restore), and the Applications fetch is gated.
State transitions (connect/disconnect) still update the UI — they are rare,
not high-frequency. Transitions are logged to stderr.

**Measured** (`measure-vispause.ps1` → `vispause-measure.txt`,
`vispause-ui.log`, agent on a dedicated pipe): minimize → CPU **exactly
zero** for the paused window (0.484 → 0.484 CPU-s across 9 s) and flat
working set (28.0 MiB); restore → `window restored: UI updates resumed`
logged within one poll tick, updates live again (post-restore 8 s delta
0.125 CPU-s includes the restore repaint catch-up).

**Defect found and fixed during this slice:** the first implementation
polled `Window::is_minimized()` from a **Slint timer on the event loop** —
the loop stops waking while the window is occluded, so the restore was
never seen and the UI stayed paused permanently (post-restore 8 s delta
0.000, reproduced twice). Slint timers cannot detect their own
un-occlusion. The plain-thread + FFI design is event-loop independent.

**Limits:** the window is located by exact title (`RAMShaper`), so two
simultaneously running UIs cannot be distinguished (single instance per
session is the design); the restore repaint causes a one-off CPU catch-up
(0.125 CPU-s over the first 8 s) before the ~0.6 % steady rate; release
`ramshaper-cli.exe` rebuild still deferred (soak lock).

---

# RS-012 UI portion — Profiles page + additive GetConfig, 2026-10-24

**Scope implemented:** the UI half of the M2 "inert profiles" milestone item.
Additive protocol-v1 command `GetConfig` (same pattern as RS-011's
`GetProcesses`): serves the loaded configuration's inert summary — `mode`,
`profile` (the active preset), `exclusions` and an explicit inertness note —
never the whole config file; unknown-command agents answer `Unsupported`.
The agent builds a `ConfigSummary` from the loaded configuration at serve
time (per-agent static material, delivered to connection workers like the
capability payload). The UI gained a **Profiles** page (nav item, `--page
profiles`): "Current profile: <preset>" plus the five presets from doc 06
§S04 with the active one marked and an inertness note, and the Overview
header now shows `profile: <preset>` (doc 06 §1 "Current Profile"). Nothing
on the page switches anything: selection lives in the configuration file or
`config import`; the page is display-only (Observe Only).

**Tests:** +3 dispatch unit tests (GetConfig serves the summary; summary
builds from a parsed config; existing contexts extended), +1 real-pipe
integration test (GetConfig over the wire incl. version-mismatch
rejection). Workspace at record time: **211 tests, 0 failed**, fmt and
clippy (`-D warnings`) clean.

**Live demo** (`docs/evidence/2026-10-24-rs012-profiles-ui/`): agent serving
`demo-config.toml` (`profiles.active = "low_ram"`, exclusions
`obs64.exe`/`Audacity.exe`); `ramshaper-agent probe --request GetConfig`
returns `{mode: observe_only, profile: low_ram, exclusions: […], note: …}`;
the UI Profiles page screenshot shows `Current profile: low_ram` with
**Low-RAM (active)** marked among the five presets.

**Limits:** the config is fetched on the Profiles-page-visible poll and once
at startup for the Overview indicator — a config *edit while the agent runs*
is not reflected until the agent restarts (the agent reads its configuration
at serve time; live reload does not exist in this slice); per-process
exclusion matching semantics remain a G3 concern.

---

# CLI defect: `--interval-ms` ignored for `low-memory-watch` (found + fixed), 2026-10-24

**Found while setting up the AT-06 24-hour soak:** launching
`low-memory-watch --interval-ms 60000` produced the stderr banner
"watching low-memory condition every **1000** ms" and emitted one sample per
second. Reproduced in the foreground with the release binary (hash
`1a74f32d…`): any `--interval-ms` value is accepted, validated, and then
disregarded.

**Root cause:** the M1 increment's `run()` passed
`config.watch.interval_ms` to the watch command and dropped
`args.interval_ms` — the parsed flag never reached the watch. The G0
cadence tests (200 ms intervals) predate the regression; the M1 cadence
demonstration drove the interval through `--config`, which works; and the
flag default equals the config default (1000 ms), so nothing else exposed
it. The agent's `serve` always resolved the equivalent precedence correctly
(`interval_ms.unwrap_or(config.watch.interval_ms)`).

**Fix:** `CliArgs.interval_ms` keeps the raw flag as `Option<u64>` and a
pure `watch_interval(args, config)` resolves the documented precedence
**flag > config file > built-in default** for `low-memory-watch`;
`processes` resolves its own default at run time. +2 tests: a unit
precedence test (flag over config over default, both directions) and a
binary-level test asserting the stderr banner honors `--interval-ms 300`
(the binary-level twin of the existing config-file cadence test).
Workspace after the fix (including the concurrent `--stop-after-ms`
increment): fmt clean, clippy `-D warnings` clean,
**177 passed, 0 failed**. Verified live with the rebuilt binary
(SHA-256 `ca6faff86a04b18c5fe53b3b959e1d4f03aa8295b8118819092f092395fca03b`):
`low-memory-watch --interval-ms 60000` → "every 60000 ms".

**Soak note:** the running 24-hour soak was launched through the **config
path** (`--config soak.toml`, `watch.interval_ms = 60000`), which was never
affected; its cadence was verified correct at launch ("every 60000 ms").

---

# RS-011 — Applications page (process inventory over IPC), 2026-10-24

**Scope implemented:** the G2 read-only application table: a new additive
protocol-v1 command `GetProcesses` (contract 14 §7 updated), the agent's
on-demand inventory provider, and the `ramshaper-ui` **Applications** page
(nav item, sortable table, poll-while-visible). Observe Only throughout; no
policy controls.

**Design (ADR-026):** the agent scans the process table **on demand, once
per `GetProcesses` request** — never on the collector cadence, which keeps
the idle agent's NFR-01 CPU budget for the snapshot collector alone (a full
scan costs ~17-21 ms here; caching it at 1 Hz would spend ~2 % of a core).
Rates come from baselines shared across requests and connections: the first
request honestly reports `warming_up`, later ones compute identity-keyed
rates over the exact gap since the previous scan. Entries are served sorted
by working set (descending) and capped at 1,024 rows with an explicit
`row_cap_note`, so a response can never exceed the 1 MiB frame cap even at
the 4,096-identity collector limit. A scan-level failure is a retryable
`OsError`; an agent without the command answers `Unsupported`, which the UI
renders as an explained empty table — never as data.

**Tests (+9; workspace 206 across all increments):** dispatch (fresh
inventory served verbatim, failed collection → retryable `OsError`, no
provider → `Unsupported` distinct from empty), `ProcessScanState` (warm-up
then exact 3.125 % CPU over a synthetic 2 s gap), real-pipe integration
(`GetProcesses` on a live agent: schema, row cap, identity contract,
second-request rate window ≥ the real gap; wrong version rejected before
any scan), UI pure functions (parse with byte/rate metrics and per-cell
notes, all four sort keys with unavailable values sinking, model rows with
units and whole-row accessible labels, note/count/cap line, missing-array
tolerance).

**Workspace gates:** fmt clean; clippy `-D warnings` clean for core, agent
and ui; `cargo test` **202 → 206 passed, 0 failed** (the concurrent RS-012
session's count included; its in-flight `tests/cli.rs` work is not mine to
lint). Release rebuild of `ramshaper-cli` remains deferred by the AT-06
soak; agent/ui exes rebuilt.

**Live evidence** (this machine, standard user, release build):
`target/rs011/` — agent `serve --interval-ms 500` + `ramshaper-ui --page
applications`; the table populated with **459 processes · sorted by working
set · rate window 2004 ms**, real byte values with units (vlc.exe 1006.83
MiB working set / 997.88 MiB private at the top), cpu% and io rates
rendered per column, and protected/unavailable rows carrying bracketed
notes instead of zeros. Killing the agent disconnects the Overview stream
(OS error 109) and the Applications fetches fail with an explanatory note;
both recover when the agent returns. Captures: `rs011-applications.png`,
`rs011-disconnected.png`, `ui.stderr.log`, `ui-footprint.csv` (WS 32.6 MiB
/ private 13.9 MiB with the table active).

**Limitations / next:** the Fluent sort buttons render low-contrast on the
light page background — folded into the already-pending AT-19 contrast
audit. Sorting is client-side over the served (working-set-capped) rows;
server-side sort parameters are unnecessary at this scale. Eligibility
explanations become meaningful with the RS-015 eligibility matrix (G3);
the page is observation-only until then.

---

# AT-23 privacy/offline review — G2-applicable portions, 2026-10-24

**Scope reviewed:** AT-23's portions applicable to the current offline,
observe-only increment: "no hidden upload/update request occurs" and "default
exports redact identities". The journal and uninstall-report portions bind to
RS-031/G3 and packaging (RS-027) capabilities that do not exist yet — recorded
as not applicable here, not passed.

**No hidden upload / update (RS-026 boundary review):** workspace code contains
zero network API usage (`std::net`, `TcpStream`, `UdpSocket`, `WinHTTP`,
`WinINet`, `WSAStartup`, reqwest/ureq/http — no hits in `crates/**`). The
Windows host dependency graph contains no network-capable crates; the only
async-stack entry, `tokio 1.54.1`, resolves solely on the Slint UI's wasm32
path (`wasm-bindgen-futures → winit`; host-target `cargo tree -i tokio`
prints nothing), so no socket code is compiled into any Windows binary.
Behavioral corroboration: the long-running observers hold no sockets
(`Get-NetTCPConnection`/`Get-NetUDPEndpoint` by owning process — soak watch
PID 41836 after ~4.5 h: tcp=0 udp=0; session agent PID 8540: tcp=0 udp=0).
No update/version endpoint exists anywhere in the codebase (contract 14:
"No network upload or automatic update check in the MVP").

**Exports carry no identities:** real `ramshaper-cli config export` captures
(`docs/evidence/2026-10-24-at23-privacy-defaults/`, debug build `0.1.0` — the
`config` command postdates the frozen release exe; the release rebuild stays
deferred while the AT-06 soak holds it): the default export contains only
schema/mode/cadence/cohort/profile markers and no machine name, user name,
SID, path, instance id or timestamp; a populated export carries exactly the
user-supplied fields (`agent.pipe_name`, `exclusions.processes`) and nothing
else. The closed schema's only free-text fields are user-typed; the tool
injects no identity material.

**Uninstall story (current status):** no installer exists (dev increment,
binaries under `target/`); verified no ramshaper scheduled task, service or
HKCU Run-key entry. The contract's retained-recovery disclosure binds when
RS-031/packaging exist.

**Limitations:** the UI process's live socket state belongs to the UI
session's evidence (the Slint/winit stack is covered by the static audit);
live socket observation is point-in-time — the static audit is the structural
argument. AT-23 remains open as a whole (journal, installer, uninstall
report, closed-alpha portions belong to G3+); what is recorded here is the
G2-applicable offline/privacy boundary review, **Passed** with the stated
boundaries.

---

# AT-19 — programmatic contrast audit + button restyle, 2026-10-24

**Scope:** the machine-checkable half of the AT-19 contrast obligation
(WCAG 1.4.3 text, 1.4.11 non-text boundaries). The interactive Narrator/DPI
pass remains open as recorded.

**Implementation:** a test in `ramshaper-ui` computes WCAG 2.x relative
luminance and contrast ratios (validated against published check values:
black/white = 21.0, #767676 on white ≈ 4.54) over **15 audited
foreground/background pairs** covering body text, secondary/footnote text,
the nav sidebar (three text colors on navy), the mode chip, active nav, the
CTA line, the new palette buttons (active and inactive faces plus the
inactive border as a 3:1 non-text boundary) and the two large-text status
glyphs (OK/ATTN at 26 px weight 800 → 3.0 large-text minimum). The palette
values are **parsed live from `ui/app-window.slint`** at test time, so a
token edit that breaks contrast fails the build rather than drifting.

**What the audit found and what changed:**

1. `warning` **#B87513 failed 4.5:1** on the page background (3.56:1) —
   used at 12 px for the stale-freshness label. Darkened to **#9C6200**
   (4.78:1); the 3 px stale stripe and the 26 px ATTN glyph keep the token
   (still ≥ 3.0 everywhere it is used).
2. The stock Fluent `Button` rendered nearly invisible on the light page
   background (observed in the RS-011 screenshots — the sort row and the
   Quit button were pale ghosts). Replaced with a `PaletteButton` built
   from audited tokens (card background + primary text inactive; accent
   background + white text active), now used for the four sort controls and
   Quit. Confirmed visually: the re-capture shows all five buttons clearly
   and the CPU sort button in its active accent state with the table
   re-sorted (`target/rs011/rs011-applications.png`).

**Status:** `cargo test -p ramshaper-ui` 11 passed / 0 failed (2 new audit
tests). Remaining AT-19 work is the human pass: Narrator reading quality,
1366×768, and the 100/125/150/200 % DPI matrix.

## Workspace repair note — 2026-10-24 (concurrent session's in-flight file)

`ramshaper-windows/src/window.rs` (the concurrent session's new
hidden-window probe, added after their last recorded gate run) did not
compile — `HWND` was imported from `Win32::UI::WindowsAndMessaging` and
`FindWindowW` was handled as a bare handle. Mechanical windows-rs 0.62
corrections applied on their behalf: `HWND` imported from
`Win32::Foundation`, `FindWindowW` unwrapped as a `Result` (a null OS
return is `Err`). Their probe test (`missing_window_is_none_and_reads_not_minimized`)
passes unchanged; `cargo test -p ramshaper-windows` 4 + 15 + … all green.

## AT-06 soak — attempt 2 outcome and attempt 3 start (2026-10-25)

**Attempt 2 (config-driven 60 s cadence, `target/soak/`) ran 2026-10-24
12:39:17Z → 20:24:17Z (7 h 45 min, 467 samples, zero cadence drift) and was
terminated by an external console-control event (exit 0x40010004) that also
ended the concurrent 1 Hz rig (its watch.jsonl froze at the same minute)
during a ~20:07–20:25Z kill window — consistent with an end-of-day machine
shutdown, which no launched process survives.** Resources over the partial
run: handle count flat at 72, working set 5.07 MB, private commit 0.79 MB
(half-hourly samples in `attempt2-process-samples.csv`; logs archived as
`attempt2-*`). No gaps, no truncation notes, low-memory never signaled.

**Attempt 3 started 2026-10-25T07:42:12Z** (fresh boot), same config-driven
60 s cadence, watcher PID 25280, binary refreshed to the current release
(SHA-256 `bf783b8da009cea3e06532e5d8e5a3fbaa035f165906e90ce53e2b793ba32f96`
— which also completes the release-binary rebuild that was deferred while
the earlier watchers held the exe). Planned end 2026-10-26T07:42Z; the
wrapper still stops the watcher with a real CTRL_BREAK and records the
history summary. The concurrent session's criteria-declared rig in
`docs/evidence/soak/at06-20261024/` (1 Hz + bench windows) was interrupted
at the same time and has not been relaunched by this session — its owner
should relaunch it or fold its criteria onto a shared run.

---

# RS-031 — action journal, first slice (pure state machine + SQLite store), 2026-10-25

**Scope implemented:** the journal foundation required before any policy
write: the pure action state machine, startup reconciliation and
pinning/quota rules in `ramshaper-core::journal`, and the SQLite-backed
store in a new `ramshaper-journal` crate (rusqlite 0.32 bundled — a new
workspace dependency). **No actuator exists and nothing in this slice can
write a process setting**; the fake actuator in tests is an in-memory
value + identity flag. The G3 policy coordinator, real actuator wiring and
agent startup-recovery loop build on this store later.

**What is pinned by tests (18 new: 11 pure, 7 store):**

- The legal transition table exactly (incl. refusals: no skipping the
  read-back, no backward jumps, no resolving `Conflict` by force).
- **Startup reconciliation reads reality** (AT-12): crash-after-Prepared
  with `current == desired` → only the ORIGINAL is written back (never a
  re-apply); `current == original` → the record resolves with **zero OS
  calls**; a third value → `Conflict` preserved untouched; identity
  mismatch (PID reuse) → `TargetGone` with zero writes to the replacement;
  unreadable → `Uncertain`, pinned, nothing inferred from defaults.
- Durability across restarts: a Prepared record survives close/reopen and
  appears in the recovery set with its exact identity (PID + creation
  FILETIME) intact.
- Replay idempotency: re-recording an action ID returns the existing
  record; TTL is never extended.
- Optimistic guard: a transition whose expected state has moved fails with
  a typed `StateConflict`; missing records are explicit `Missing`.
- Pinning/quota: a `Conflict` record as old as the retention window is
  never pruned; only terminal unpinned records go; at cap with only pinned
  records the pure decision blocks new actions.
- Schema refusal: a `user_version` from a future build is refused with
  `SchemaMismatch { found }` and the file is byte-for-byte untouched
  (verified: version and records unchanged after the refused open); a
  non-database file is `Corrupt`, also untouched.
- `SQLITE_FULL` classifies to `DiskFull` (continue observation, disable
  new actions) by primary result code, not string matching.

**Workspace gates (2026-10-25):** fmt clean; clippy workspace-wide
`-D warnings` clean; `cargo test --workspace --locked` **226 passed,
0 failed** (both sessions' increments: core 119 incl. journal, journal 7,
plus the earlier suites).

**Not in this slice (recorded, not hidden):** the real actuator (G3, stays
disabled until recovery tests pass per the roadmap rule), agent integration
(startup recovery loop, `ListActions` serving real records), disk-full
live-fault injection (the classification is unit-pinned; an induced
`SQLITE_FULL` run on a full temp volume is AT-18 material), and
clear-history/erase-all interplay (needs the retention UI decisions).

**Environment:** same machine/toolchain as the prior entries (Windows 11
build 26200.9711, standard user, Rust 1.93.1); `Cargo.lock` updated for
`ramshaper-journal` + `rusqlite`/`libsqlite3-sys` (bundled) + `tempfile`
(dev). The AT-06 soak attempt 3 was running throughout (seq 203 at gate
time) and is unaffected — the journal crate is not in any binary the soak
runs.

---

# RS-015 — eligibility matrix, 2026-10-25

**Scope implemented:** the apply-time eligibility gate for the first policy
action as pure logic in `ramshaper-core::eligibility` (contract 14 §5).
Eleven named checks — identity (PID reuse), owner/session, foreground
(fresh, target not foreground), compatibility allowlist, safety
classification, user exclusion, original-Normal priority, leaf workload
(no live children), no outstanding action, journal available, priority API
capability. No OS calls: adapters hand observations to `evaluate`;
nothing here can write a process setting or query the OS.

**What is pinned by tests (23 new):**

- **Every `Unknown` denies** (R-03): unknown owner/session, focus,
  classification, priority class, child state and API capability each
  produce a typed deny.
- **The initial allowlist is empty by design**: with no workload having
  passed RS-024 Stage B compatibility testing, a fully-observed "good"
  target still denies with exactly `not_on_tested_allowlist`.
- PID reuse denies and the audit text carries the replacement identity
  (authority never transfers); unreachable identity denies with the
  machine failure code (`exited_87` etc.).
- Foreground semantics: a fresh observation with the target foreground
  denies (`target_foreground`); background observations are fresh at
  age ≤ 2,000 ms (contract 14 §3, inclusive) and stale at 2,001 ms.
- All seven protected categories deny (audio, security, accessibility,
  install/update, game/anti-cheat, RAMShaper self, known incompatible
  priority manager) with `protected_<category>` codes.
- User exclusions match case-insensitively (`LeafApp.EXE` vs
  `leafapp.exe`) and the deny reports the matched name; exclusion can
  only reduce eligibility, never grant it.
- Observed live children deny the leaf-workload gate (R-10); non-Normal
  and unknown original priority deny (skipped, not corrected).
- Multiple failures are all reported in the documented order (tested with
  three simultaneous failures); `first_deny` follows the same order.
- Reports serialize for IPC/UI with stable machine codes
  (`{"code":"excluded_by_user","message":...}`) — never Rust variant
  names.

**Workspace gates (2026-10-25):** fmt clean; clippy `--workspace
--all-targets -D warnings` clean and `cargo test --workspace --locked`
green (242 passed) **excluding `ramshaper-journal`**, whose in-flight
store tests were being edited by the parallel session at gate time (one
failing test there is theirs, untouched by this slice — this module adds
no dependency and does not touch the journal crate).

**Not in this slice (recorded, not hidden):** OS signal adapters —
foreground detection, owner/session queries and the protected-category
signal matrix that R-03 requires documented and tested before G3; allowlist
population (blocked on RS-024 Stage B evidence); IPC/UI wiring
(`PreviewAction` consumes this in RS-016; RS-013 renders reports); the
per-identity cooldown and stale-input trip (RS-018).

**Environment:** same machine/toolchain as prior entries (Windows 11
build 26200.9711, standard user, Rust 1.93.1); no new dependencies. The
AT-06 soak attempt 3 was unaffected.

---

# RS-031 — storage-failure semantics (clear-history + disk-full), 2026-10-25

**Scope:** the remaining two storage-failure inputs of AT-18 that the store
can exercise without a policy coordinator: **clear-history during an active
action**, and a deterministic **SQLITE_FULL fault**.

**Clear-history (contract 14 §9):** `JournalStore::clear_history()` removes
terminal unpinned records of any age and **retains every active/unresolved
recovery record**, returning `(removed, retained)` so callers can explain
the retained set ("never say all restored while unresolved records
remain"). Pinned by test: a `Restored`, an `Applied` and a `Conflict`
record together → `(1, 2)` — the active action and the pinned conflict
survive and remain in the served recovery set.

**Disk-full (AT-18):** `SQLITE_FULL` is induced deterministically via the
documented testing hook `set_fault_page_cap` — a per-connection
`PRAGMA max_page_count` (probed first: the pragma enforces at-or-above the
current page count; values below the current size clamp away, so the test
caps at 50 pages and grows past it with ~8 KB rows; real exhaustion takes
the identical error path, and creating an actually-full volume needs
privileges the standard-user tests run without). Pinned by test:
record inserts fail with the typed `DiskFull` error; **prior records stay
intact and readable** (the journal is consistent, not half-written);
**recovery proceeds while "full"** — a transition of the recorded intent
commits (in-page update, no growth); lifting the cap accepts new records
again. Plus a quota gate helper, `may_record_new_action(cap)`, folding the
current counts into the pure block-don't-evict decision.

**Workspace gates (2026-10-25):** fmt clean; clippy workspace-wide
`-D warnings` clean; `cargo test --workspace --locked` **251 passed,
0 failed** (includes the concurrent session's RS-015 eligibility suite,
landed the same evening; journal crate now 9 store tests + 11 pure-core
journal tests). The AT-06 soak attempt 3 was running throughout (seq 245
at gate time).

**RS-031 remains open for:** the real (still disabled) actuator, agent
startup-recovery wiring and `ListActions` serving records, crash-boundary
fault injection at each commit point in a real process (needs the agent
integration), and clear-history/erase-all UX decisions (RS-013).

---

# RS-019 — deterministic policy dry-run, 2026-10-25

**Scope implemented:** `ramshaper-core::policy` — `dry_run()` simulates
the 0.2 policy decision layer over synthetic traces with zero OS writes,
by composing the real building blocks: the `pressure_v1` classifier
(RS-009), the eligibility matrix (RS-015) and the journal transition
table (RS-031). Rules (`PolicyRules`) validate the contract TTL window
(10–300 s, default 120 s); traces carry per-step pressure inputs, target
observations, and synthetic events (confirm/disable/external
change/config invalidation/journal availability). This is the engine the
future `policies --dry-run` CLI command (contract 14 §8) and the
RS-016 preview/confirm flow build on.

**What is pinned by tests (20 new):**

- **Determinism**: the same rules + trace produce an equal (and
  byte-identical serialized) log — twice, asserted.
- **Offer path**: previews only under fresh `Elevated` pressure
  (3 consecutive signals, 2 s span) with a fully eligible target; the
  offer carries the deterministic id (`dry-N`), TTL and rules version.
- **Deny/no-action path, all explicit**: `pressure_unknown`,
  `pressure_not_elevated` (incl. Normal), `ineligible` carrying every
  eligibility deny reason (empty allowlist → exactly
  `not_on_tested_allowlist`; foreground target → `target_foreground`),
  `no_valid_preview`, `preview_expired`, `journal_unavailable`,
  `disabled`, `action_active`.
- **Confirm path**: TTL deadline = confirm + TTL (asserted numerically);
  the 30 s preview window is inclusive at the boundary and expires one
  ms beyond; **eligibility is re-checked at confirmation** — a
  foreground return between offer and confirm denies the apply
  (`applies_simulated == 0`); a confirm replay against an active action
  is `ReplayIgnored` with exactly one apply.
- **Resolve path — every contract trigger**: foreground return, target
  vanished, external priority change, child appeared, stale focus,
  config invalidation, manual disable, TTL expiry — each maps through the
  journal table to its contract final state (`Restored`,
  `TargetGone` for vanish, `Conflict` for external change), with a fixed
  precedence (disable wins) when triggers coincide.
- **Journal semantics in decisions**: journal unavailability blocks
  offers and confirmations but **never blocks recovery** (the TTL resolve
  proceeds with `journal_available = false`); one-action cap blocks new
  previews while active; a re-offer keeps the action id until the
  preview expires.
- **Known boundary, pinned honestly**: the 60 s per-identity cooldown and
  stale-input trip are RS-018's — a resolved experiment CAN be re-offered
  in this simulation, asserted by a boundary test.
- **Serialization**: the log serializes with schema
  `ramshaper.policy-dryrun.v1`, summary counts and stable cause codes.

**Workspace gates (2026-10-25):** fmt clean; clippy `--workspace
--all-targets -D warnings` clean; `cargo test --workspace --locked`
**271 passed, 0 failed** (core lib 161: 20 policy + 23 eligibility + 118
prior; journal store 9 from the parallel session's AT-18 slice).

**Not in this slice (recorded, not hidden):** the `policies --dry-run`
CLI surface (appears with the gated policy capability per contract 14
§8), the RS-016 `PreviewAction`/`ConfirmAction` IPC wiring, the RS-018
cooldown/stale-input trip, real trace capture from live observations,
and RS-024 Stage B comparison over dry-run logs.

**Environment:** same machine/toolchain as prior entries (Windows 11
build 26200.9711, standard user, Rust 1.93.1); no new dependencies. The
AT-06 soak attempt 3 was running throughout and is unaffected.

**RS-019 collision addendum (2026-10-25 21:05):** after this section
landed (commit 9e0bdaf, CHANGELOG 0.3.21, ADR-029), an **unwired
in-flight duplicate** was found in the tree:
`crates/ramshaper-core/src/dryrun.rs` (parallel session, file created
20:45 local — before this section's changelog entry landed; not declared
in `lib.rs`, not compiled, not tested, and swept into 9e0bdaf by a
`git add -A`). `ramshaper-core::policy` (this section) is the canonical,
tested, documented implementation — **do not build on dryrun.rs**. To its
author: fold anything worth keeping into `policy.rs` (its
`Step::Observe`/`Step::Probe` trace model and the AT-22 independent-rerun
reference are the notable differences worth reconciling), then delete the
file. The `271 passed` gate claim in 9e0bdaf covers everything compiled
at gate time; dryrun.rs was not (and is not) part of any build.

---

# R-03 — eligibility OS signal-source adapters, 2026-10-26

**Scope implemented:** the tested OS signal side of the eligibility contract
(ADR-028's deliberately-out-of-scope item; readiness risk R-03's "tested OS
signal sources"): new `ramshaper-windows::eligibility` module, one adapter
per observation the RS-015 matrix consumes. All read-only — nothing here
writes a process setting; the capability probe opens a
`PROCESS_SET_INFORMATION` handle and closes it without writing.

- `observe_owner_session(pid)` — target user SID (process token,
  `TOKEN_QUERY`) vs current user SID, plus both logon session ids; any
  failure (exit `exited_87`, `access denied`, OS error) is `Unknown`.
- `observe_focus(pid)` — `GetForegroundWindow` +
  `GetWindowThreadProcessId`, compared to the target PID; fresh by
  construction (`observed_age_ms = 0`; staleness is decided in core).
  No interactive desktop (null foreground) is `Unknown`, never "not
  foreground".
- `observe_priority_class(pid)` — `GetPriorityClass` through a
  `PROCESS_QUERY_LIMITED_INFORMATION` handle; all six documented class
  constants mapped by name, unknown values reported verbatim, zero read
  is `Unknown` with the preserved error.
- `observe_children(pid)` — direct live children counted from one
  Toolhelp snapshot (`enumerate_pid_parents` added to `processes`); a
  snapshot failure is `Unknown`, never a silent zero (R-10).
- `probe_priority_capability(pid)` — `OpenProcess(PROCESS_SET_INFORMATION)`:
  success = `Supported` (handle closed, no write), `ACCESS_DENIED` =
  `Unsupported` (the tested cannot-change signal), exit/OS error =
  `Unknown`.
- `classify_target(...)` — deliberately narrow (doc 04: no universal
  detection): self-identification only — target PID equal to RAMShaper's
  own PID or image name equal to one of RAMShaper's own well-known binary
  names (case-insensitive) is `protected_ramshaper`; **every third-party
  target is `Unknown` (deny)** until RS-024 Stage B adds tested coverage.
  No denylist of other software exists or is claimed.

**Fixture note (defect found during testing, fixed):** the first version of
the unknown-state tests spawned a child, waited for exit and polled until
the PID stopped resolving — it never did within 5 s because **Windows
reuses exited PIDs within seconds**; the poll was measuring a new owner of
the recycled PID. Replaced with an honest deterministic fixture: an unused
high PID that `resolve_identity` reports as unresolvable (`ERROR_INVALID_PARAMETER`),
which is exactly the documented post-exit state the adapters must map to
`Unknown` (QueryFailure::Exited covers "target process exited or the PID
no longer resolves").

**Live results (13 tests, evidence in
[../evidence/2026-10-26-r03-signal-adapters/live-adapter-tests.txt](../evidence/2026-10-26-r03-signal-adapters/live-adapter-tests.txt)):**
own process owner/session = `CurrentUserSession`; unresolvable-PID
observations all `Unknown` with `exited_87`; `services.exe` token query =
`access denied` → owner `Unknown` and capability `Unsupported` on this
standard-user environment; focus of the windowless test process observed
background (never foreground); fresh `ping` child = `Normal` priority and
counted as a live child; self-classification covers PID/name
self-identification only.

**Workspace gates (2026-10-26):** fmt clean; clippy `--workspace
--all-targets --locked -D warnings` clean; `cargo check --workspace
--all-targets --locked` clean; `cargo test --workspace --locked` **284
passed, 0 failed** (271 + 13); release build
`cargo build --release --target x86_64-pc-windows-msvc --locked` clean
(the AT-06 soak watcher no longer holds `ramshaper-cli.exe` — see the
AT-06 note below).

**Not in this slice (recorded, not hidden):** the RS-024 Stage B tested
workload matrix (what populates the compatibility allowlist — requires
cohort-rig workload testing); wiring these adapters into the agent's
`PreviewAction` flow (RS-016); per-identity cooldown/stale trip (RS-018).
The classification adapter makes no claim beyond self-identification:
protected-category detection (audio/security/accessibility/installer/game)
has no tested signal source yet and stays `Unknown`-denies by design.

**Environment:** Windows 11 build 26200.9711, standard user, Rust 1.93.1,
windows-rs 0.62.2 (no new dependencies; no new feature flags —
`Win32_UI_WindowsAndMessaging`/`Win32_System_Threading` were already
pinned).

---

# AT-06 soak — attempt 3 outcome: early stop, 2026-10-26

Observed at 2026-10-26T07:59Z (17 min after the planned
2026-10-26T07:42Z end): no `ramshaper-cli` process remains, and
`target/soak/watch.jsonl`'s last sample is **seq 245 at
2026-10-25T11:46:13Z** — attempt 3 stopped after ~4 h 04 min of its
planned 24 h, consistent with the overnight shutdown/sleep pattern that
ended attempts 1 and 2 (nothing was written after 11:46Z; the wrapper was
verified alive before that point). 245 samples at 60 s cadence, no
low-memory signal. This record is factual only: the 24-hour completion
criterion (C3) was not met on this rig, and the relaunch-vs-fold decision
stays with the owner as previously recorded. No verdict was recorded and
no process was killed.

---

# RS-016 — pure preview/confirm coordinator, 2026-10-26

**Scope implemented:** the 0.2 action lifecycle as a pure coordinator
(`ramshaper-core::coordinator`) over two traits it defines — `ActionJournal`
(implemented by the SQLite store in `ramshaper-journal`) and
`PriorityActuator` (fake in tests; the real Windows implementation remains
**disabled and unwired** per the roadmap's actuation gate). Nothing here
can write a real process setting.

**Contract behavior pinned by tests (+10: 9 coordinator units + 1
end-to-end over the real SQLite store):**

- Previews are offered only when the journal is available, the slot is
  free, the TTL is inside 10–300 s (default 120 s), pressure is fresh
  `Elevated`, and the RS-015 gate fully passes — deny codes surface
  verbatim (`excluded_by_user` verified); otherwise `PressureNotElevated` /
  `Denied(report)` / `TtlOutOfRange` / `JournalUnavailable` /
  `ActionOutstanding`.
- The 30 s preview validity is inclusive: confirm at exactly +30 000 ms
  applies; +30 001 ms expires (matches ADR-029's boundary convention).
- Confirm orders durability before effect: the fake actuator asserts the
  journal already holds the `Prepared` record at write time.
- Idempotent replay: a second confirm of an applied action returns
  `Replayed { deadline }` with the actuator call count unchanged — and the
  replay check precedes the pending-preview gate, so it works after the
  slot has freed.
- Failure classification: clean OS failure → `Failed` (unpinned);
  unknown outcome and read-back mismatch → `Uncertain` (pinned, recovery
  retained for startup reconciliation); identity mismatch → `TargetGone`
  with zero writes; storage faults → `StorageBlocked` and no applies.
- End-to-end over `JournalStore` on a real file: preview → confirm →
  `Applied { deadline 122_000 }` → replay → durable `Applied` record with
  the exact deadline, one actuator call total.

**Workspace gates (2026-10-26):** fmt clean; clippy workspace-wide
`-D warnings` clean; `cargo test --workspace --locked` **297 passed,
0 failed** (core 170 incl. 9 coordinator; journal 10 incl. the trait
end-to-end).

**Not in this slice (recorded, not hidden):** the real actuator (stays
disabled), restore triggers (RS-017), the one-action cooldown and
stale-input trip (RS-018), agent/IPC wiring (the coordinator stamps no
agent-instance/consent fields — the agent fills them), and the eligibility
"no outstanding action" count is fed as 0 while the single slot models the
cap (multi-action accounting arrives with RS-018).

---

# RS-016 — real priority actuator (Windows half of the coordinator), 2026-10-26

**Scope implemented:** the real `PriorityActuator` behind the
disabled-actuation gate — new `ramshaper-windows::priority`
(`WindowsPriorityActuator`), implementing `ramshaper-core::coordinator`'s
trait (the pure preview/confirm coordinator and the SQLite store's
`ActionJournal` impl are the parallel session's in-flight slice; this is
the OS write half it composes with).

**Safety properties (contract 14 §1/§6), each pinned by a live test:**

- **One validated handle per write**: `OpenProcess(PROCESS_SET_INFORMATION |
  PROCESS_QUERY_LIMITED_INFORMATION)` — the write right plus exactly the
  query right the verification reads need; no inheritance.
- **Identity validated through the write handle** before any effect: the
  creation FILETIME is read through the same handle and compared to the
  consented identity. Live test: a wrong creation marker on a spawned child
  is refused with `IdentityMismatch` and an independent adapter read
  confirms the class is untouched; a PID reuse can therefore never inherit
  the consented write.
- **Every write is read back through the same handle**; the returned value
  is what the coordinator compares to the desired value.
- **A failed `SetPriorityClass` is never assumed effect-free**: the value
  is read back, and a read-back equal to the desired value is the honest
  `UnknownOutcome` (write may have landed). A disagreeing read-back is the
  clean `OsFailure{code, function}`.
- **Unresolvable PIDs refuse without effect** (the documented post-exit
  state; same top-of-address-space fixture as the R-03 adapters).

**Live result** (evidence in
[../evidence/2026-10-26-rs016-priority-actuator/live-actuator-tests.txt](../evidence/2026-10-26-rs016-priority-actuator/live-actuator-tests.txt)):
the only live write target is a test-spawned `ping` child — Normal →
Below Normal (read back `BELOW_NORMAL_PRIORITY_CLASS`, confirmed by an
independent `observe_priority_class` adapter read) → restored to Normal
through the same verified path, all inside the test. Workspace gates at
record time (both sessions' work integrated): fmt clean, clippy `-D
warnings` clean, **297 passed / 0 failed**.

**Not in this slice:** agent IPC wiring (`PreviewAction`/`ConfirmAction`
commands) and the startup recovery loop — the actuation stays disabled at
the agent boundary until the RS-017 recovery tests pass (contract 14 §7);
restore-on-trigger flow (RS-017); the coordinator records
`original: None` at prepare with read-side pinning still to be wired.

**Collision note (2026-10-26):** while building this slice I wrote a
duplicate pure coordinator (`action.rs`) and a duplicate
`ActionJournal for JournalStore` impl; the parallel session's
`core::coordinator.rs` and its store impl (integrated test in
`ramshaper-journal/tests/store.rs`) are canonical — both duplicates were
deleted unmerged before any commit. Pre-flight `git log`/CHANGELOG checks
had shown RS-016 unclaimed ~30 min earlier; the claim window is that short
with three live sessions.

---

# RS-017 collision note — restore engine is coordinator.rs (parallel session), 2026-10-26

Following this session's earlier RS-016 pattern: while the pure RS-017
restore/reconcile engine was being built here (claim posted to shared
memory, `recovery.rs` + a Windows `RestoreActuator` written and
partially tested), the parallel session landed the restore engine inside
`ramshaper-core::coordinator.rs` — `RestoreTrigger` (11 named triggers
including `Suspend`/`Error`/`CleanExit`), `RestoreOutcome` (including
`NeedsAttention` for must-not-retry failures and `RetryScheduled` with
absolute due times), the per-action bounded-retry bookkeeping
(`RESTORE_RETRY_DELAYS_MS` 1 s / 5 s / 30 s, then `ManualRetry`) and the
compare-before-restore `RestoreEngine`. **`coordinator.rs` is canonical
for RS-017; the duplicate `core::recovery.rs` and
`windows::restore.rs` were deleted unmerged** — nothing in them is
missing from the canonical engine (its trigger set, retry schedule and
outcome classes are a superset of the duplicates').

The complementary piece delivered here instead: **`PriorityActuator::read`
implemented for `WindowsPriorityActuator`** — the identity-validated read
their engine composes with: query-only handle, exact creation FILETIME
validated through the handle (`IdentityMismatch` otherwise — a reused PID
never lends its value to a consented action), class read with the
preserved-error `OsFailure` mapping. Two live tests: a spawned child reads
`NORMAL` through the validated handle (wrong creation marker refused),
and an unresolvable PID refuses as `IdentityMismatch`.

For the coordinator session, from the deleted duplicates' design (all
covered by yours — confirming, not requesting): reconcile routing
(Applied → restore, `RestorePending` → retry, `Prepared`/`Uncertain` →
startup reconcile), and the never-infer-a-missing-original guard.

---

# RS-017 — restore engine (triggers, bounded retry, conflict), 2026-10-26

**Scope implemented:** `RestoreEngine` in `ramshaper-core::coordinator`
(+10 tests): all 11 contract restore triggers as a typed vocabulary moving
`Applied → RestorePending` first; compare-before-restore through the
actuator's identity-validated `read` (0.3.25's `WindowsPriorityActuator`
impl); bounded retries 1 s / 5 s / 30 s then manual; `ERROR_ACCESS_DENIED`
→ immediate attention; `Coordinator::disable/enable` stopping new applies.
Reuses `journal::reconcile` so startup recovery and live restoration are
literally the same decision function. **No real process is written**: the
actuator is the test fake (the real impl stays behind the disabled-IP
boundary; the agent does not expose policy commands yet).

**Pinned by test:** TTL expiry restores the original through one verified
write; an external third value is preserved untouched as `Conflict` (zero
writes); a gone target is `TargetGone` with zero writes; a record already
back at its original resolves `Restored { wrote_back: false }` with zero
writes; transient read failures schedule retries at exactly +1 s / +5 s /
+30 s (an early call restates the schedule instead of hammering), then
`ManualRetry` with the record still pinned `RestorePending`;
`ERROR_ACCESS_DENIED` reads go to `NeedsAttention` with no retries; a
success on a scheduled retry clears the schedule; disable drops the
preview and refuses previews/confirms until re-enabled; the snapshot's
`ttl_expired` treats the deadline instant itself as not-yet-expired.

**Workspace gates (2026-10-26):** fmt clean; clippy workspace-wide
`-D warnings` clean; `cargo test --workspace --locked` **308 passed,
0 failed** (core 179 incl. 10 restore-engine tests; the end-to-end SQLite
store test updated for the richer `JournalSnapshot` trait).

**Collision note:** the concurrent session independently started RS-017
(`recovery.rs`/`restore.rs`), saw this slice and deleted their duplicates
unmerged (their 0.3.25 note) — the canonical engine is
`core::coordinator.rs`. Their `WindowsPriorityActuator::read` (0.3.25) is
the complementary half this engine composes; both landed together.

**Not in this slice (recorded, not hidden):** the 2-second
verification-target measurement (the agent ticker owns scheduling; the
engine exposes due times), session-lock/suspend signal wiring (R-03
adapters exist; the agent loop integrating them is the remaining G3 glue),
and multi-action accounting (RS-018).

---

# RS-018 — guardrails (cap, cooldown, disable, safety trips), 2026-10-26

**Scope implemented:** `ramshaper-core::guardrails` (+8 tests), the pure
decision layer AT-14 names: `OneActionCap` (single slot; same-id reserve
idempotent; refusals name the holder), `TargetBook` (60 s per-identity
cooldown after terminal records only, keyed by exact identity; target
disable on failure lifted only by explicit `re_enable`, never by time) and
`evaluate` (child-spawned → RS-017 `child_spawned` trip + recorded
`CompatibilityFinding`; foreground-returned / session-locked / stale-focus
→ their triggers). **No OS writes**: unapproved descendants appear only as
recorded findings — the decision vocabulary has no write-to-child path.

**Pinned by test:** cooldown blocks for exactly 60 s after a terminal
record then admits; per-identity keying (one target's cooldown never
affects another); pinned records start no cooldown; a disabled target
stays inadmissible even at `u64::MAX` until re-enable (reason carried);
repeat confirmations hold the cap while a different action is refused and
terminal release frees it; child observations trip with the finding
attached (child identity + image + timestamp recorded); the three other
observations map to their triggers; trigger codes are stable.

**Workspace gates (2026-10-26):** fmt clean; clippy workspace-wide
`-D warnings` clean; `cargo test --workspace --locked` **316 passed,
0 failed**.

**Not in this slice (recorded, not hidden):** the agent loop that feeds
safety observations and performs the mapped restorations (the RS-017
engine accepts the trips as-is); the child-inheritance *live probe*
(AT-14's second evidence half — a real child inheriting a priority class
needs the enabled actuator at the agent boundary); allowlist population
(RS-024 Stage B consumes the findings).

---

# Agent policy loop — G3 glue (restore direction), 2026-10-26

**Scope implemented:** `ramshaper-agent::policy_loop` (+6 tests): startup
recovery that reconciles `Prepared`/`Uncertain` records from the current
value and restores `Applied`/`RestorePending` ones through the RS-017
engine; safety observations through the RS-018 guardrails with the mapped
restoration on the outstanding record; TTL-expiry and due-retry ticks; and
the 2-second verification target as **accounting** — one counted miss per
trigger, never a fabricated success, preserved even after a later honest
restore.

**Pinned by test:** a disabled loop is a no-op on every entry point;
startup recovery with the value already back at the original resolves with
zero writes and counts one reconciled record; startup recovery with a live
write restores the original; a child-spawned observation trips exactly one
restore; a TTL-expired record restores on the tick past its deadline; and
the missed-deadline test walks the full path — two transient read failures
across the 2-second boundary count exactly one miss (ticks inside the
target count none, repeats never double-count), the 1/5/30 s schedule
lands its third attempt at first+6 s, and the eventual restore is real.

**Composition:** the concurrent session's `policy::PolicyGate` (the four
policy IPC commands, coordinator + journal + actuator behind one lock,
startup reconciliation before the pipe accepts) wired this loop into
`serve` as the restore-direction ticker; both halves landed as one
increment (their repair pass even absorbed this module's helper mid-flight
— the crate compiles only as a unit).

**Workspace gates:** agent crate 21 passed / 0 failed at slice close
(6 loop tests + the gate suite); the full workspace gate run is blocked
only by the concurrent session's in-flight `tests/policy_ipc.rs` lint
(two unused imports, being finished as this was written).

**Not in this slice (recorded, not hidden):** the action direction stays
closed (no IPC path can create actions; enabling is the owner-gated G3
consent flow); the live child-inheritance probe and allowlist population
(RS-024 Stage B) remain.

---

# RS-016 completion — agent policy IPC + startup recovery wiring, 2026-10-26

**Scope implemented:** the four policy-stage commands of contract 14 §7 are
now served end to end: additive protocol-v1 payloads (`PreviewAction`,
`ConfirmAction`, `RestoreAction`, `DisableActions` in `ramshaper-core::ipc`,
with two additive error codes `InvalidArgument`/`Uncertain` and an optional
`ErrorBody.detail` carrying the explain-all report of an `Ineligible`
refusal); a new `ramshaper-agent::policy` **PolicyGate** (coordinator +
restore-engine bookkeeping + SQLite journal + the real actuator behind one
lock; preview bindings remembered per durable action ID); dispatch arms
(gate absent ⇒ `Unsupported`, exactly like a pre-policy agent); and
`serve()` gaining `Option<Arc<PolicyGate>>` — **agents started without it
are unchanged, and the gate starts disabled: startup is Observe Only, and
enabling is the G3 consent flow, never a protocol command.**

**Safety properties, each pinned by a live test** (evidence in
`docs/evidence/2026-10-26-rs016-agent-policy-ipc/`):
- **Full loop over the live OS**: preview offered (fresh Elevated pressure,
  tested allowlist, verified classification) → confirm durably prepares,
  **reads the original through the identity-validated handle and pins it
  before the write**, applies Below Normal to the test's spawned `ping`
  child (independent adapter read confirms) → a lost-reply replay returns
  the recorded deadline without a second write → restore returns the child
  to Normal (adapter-confirmed).
- **Untested third-party targets are denied with the full report**: the
  production shape (empty allowlist, self-only classification) yields
  `Ineligible` with the explain-all `detail` (`not_on_tested_allowlist`,
  `classification_unknown`).
- **Disabled gate** refuses previews (`Busy`) until explicitly enabled;
  **DisableActions** stops new applies and drains everything outstanding
  immediately (child verified back at Normal).
- **Restore of an unknown ID** is an explained `InvalidArgument`.

**Division of labor with the parallel session (collision #4, resolved by
scoping):** the restore *direction* is theirs — `ramshaper-agent::policy_loop`
(restores only; "nothing in this build can create actions"), wired into
`serve()` to run startup reconciliation before the pipe accepts. The action-
*creating* direction is this slice. The gate exposes its journal to the loop
(`journal_clone` — one SQLite database, two handles).

**Contract gap found and closed (this slice):** `Coordinator::confirm`
persisted `original: None` ("read-side pinning later") — but nothing ever
pinned it, so every restore would have hit the engine's honest
"original never pinned → NeedsAttention" refusal. Closed with
`JournalStore::record_original` (guarded to `Prepared` records) + the
`ActionJournal::record_original` trait method + a pre-write
read-through-the-validated-handle in confirm — the original is now durably
committed **before** the OS call, as contract 14 §6 requires. Their in-flight
test fakes were extended mechanically to match (recorded here per the
inter-session convention).

**Workspace gates (2026-10-26, integrated tree):** clippy
`--workspace --all-targets -D warnings` clean; this slice's suites green
(agent lib, `tests/policy_ipc.rs` 5/5 live, core 187 incl. the pinning,
journal, windows 22). **One test was failing at gate time, not this slice's:** the parallel
session's in-flight `policy_loop` test
`missed_deadline_counts_once_and_never_fakes_success` (their file was
mid-write; recorded, not touched). **Resolved ~30 min later by their fix**
— and an independent probe through the real SQLite store (real
`unresolved_ids` → `outstanding_applied_action` → flaky-read
`RetryScheduled` schedule) confirmed the accounting counts one miss past
the 2 s target and never fabricates a restore; agent lib suite green
(22/0) at close.

# RS-020 — managed-launch spike (creation-time job attribute), 2026-10-26

**Scope implemented:** `ramshaper-windows::job` (+6 live tests), the doc 04
§D evaluation R-06 asks for before any managed launcher: a fresh advisory
job (`CreateJobObjectW`; the module never calls `SetInformationJobObject`,
so no limits, no kill-on-close, no UI restrictions — **queried back**,
never assumed) and `ManagedChild::spawn` — `CreateProcessW` with
`EXTENDED_STARTUPINFO_PRESENT` and one `PROC_THREAD_ATTRIBUTE_JOB_LIST`
attribute, **no `CREATE_SUSPENDED` anywhere**, membership verified through
the child's own handle before the spawn reports success. Probes:
`is_in_any_job` (environment preflight), `limit_flags`, `process_ids`;
`AdvisoryJob::assign` kept only as the legacy-fixture seam.

**Live findings (Win11 26200.9711, standard user):** creation-time
assignment works — membership is provable at birth, the child completes
unaided (no suspended orphan), job-close is survivable (queried flags stay
0), a failed launch leaves the job empty, and a child created while the
launcher is itself in a job lands nested in **both** (never escaped).
**The superseded mechanism is dead on this build:** post-create
`AssignProcessToJobObject` returns `ACCESS_DENIED` for a same-user running
child (refusal is clean — no half-assignment), which independently
supports doc 04 §D's supersession by the creation-time attribute.

**Boundary:** spike only — no production caller exists (the agent, CLI and
UI do not reference the module), and the full AT-15 launcher-interruption
lifecycle trace stays G4 rig work; the spike pins the crash-window *shape*
by construction (no suspend state; membership is the same syscall as
birth). Evidence in `docs/evidence/2026-10-26-rs020-job-spike/`.

**Workspace gates (2026-10-26):** `cargo fmt -p ramshaper-windows` clean;
`cargo clippy -p ramshaper-windows --all-targets -- -D warnings` clean;
windows crate **44 passed, 0 failed** (28 lib incl. the 6 new job tests +
15 integration + 1 pipe probe). ADR-035.

# 0.3.29 repair note — policy loop due-retry section, 2026-10-26

The 0.3.28 commit shipped `policy_loop::tick` without the due-retry pass
its own doc comment ("TTL expiry, due retries and missed-deadline
accounting") and its test
(`missed_deadline_counts_once_and_never_fakes_success`) pin: nothing
re-fired a `RetryScheduled` record at its due time, so the third scheduled
attempt never landed and the test failed 0 vs 1 — deterministically, on
the committed tree (recorded 3/3 runs against 0.3.28 HEAD; the green
"22/0 at close" in the 0.3.28 record above was measured on a working tree
that already carried this repair — a working-tree/commit skew, recorded
here per the no-faked-results rule). The 0.3.29 repair adds the missing
pass: a `retry_dues` map fed by `account_terminal` (`RetryScheduled`
records the next allowed attempt; `ManualRetry`/`NeedsAttention` clear the
schedule — the record then waits for an external trigger, never a busy
tick loop) and a tick pass that re-attempts **at exactly the due time,
inclusive** (a due value is when the attempt becomes allowed; earlier
calls restate the schedule instead of writing — RS-017 semantics, and the
house's inclusive-boundary precedent). Clippy `-D warnings` also failed on
0.3.28's committed `tests/policy_ipc.rs` (four `unwrap()`s); fixed in
657a6f9 by the loop's session and verified clean here. Agent suite green
(22 lib + 4 policy_ipc + integration) after the repair; full workspace run
in `docs/evidence/2026-10-26-rs020-job-spike/workspace-run.txt`. Collision
note: the repair lands in the RS-020 session's commit because the loop
session had moved on to `ramshaper-bench`; the loop session's 657a6f9
record above already claimed the resolution this commit makes true.

---

# AT-20 — A/B analysis fixtures (RS-024 Stage B analysis half), 2026-10-26

**Scope implemented:** `ramshaper-bench::analysis` (+8 tests) and the
`analyze-ab` CLI subcommand: per-run nearest-rank **ceil** p50/p95, sample
counts with preserved missing reasons, paired-by-block deltas
(`100×(A−B)/A` when `A > 0`), a 10,000-resample whole-block bootstrap CI
with a recorded seed (local SplitMix64 — no new dependency), and the four
doc-08 outcome labels with the 5% primary threshold, the 2%/10%
completion guardrails and the safety-failure override. **Fixture
calculations only — no claim of real benefit**; the A/B runner that
produces real trials stays owner-gated.

**Independently verified (hand-computed fixtures):** nearest-rank edges
(10 events → p95 = position 10, p50 = position 5; single event; empty and
out-of-range `q`); paired percentages [+10, −5, +10, 0]% computed by hand
and matched to 1e-9; missing values counted and their reasons carried
verbatim while ranks are computed over valid events only; a missing
baseline denominator yields an excluded pair (never infinity); identical
seeds replay identical intervals.

**Label decision table pinned:** strong benefit over 5 blocks →
`validated_improvement` (interval excludes zero, confirmatory count); a
safety failure → `regression` regardless of speed; uniform −5% over 5
blocks → established `regression`; flat 0% over 5 blocks →
`no_measured_improvement`; a 3-block mixed fixture whose bootstrap
slowdown upper bound (5%) violates the 2% foreground guardrail →
`regression` **even though exploratory** (guardrails trump small-sample
uncertainty); a 3-block uniform +2% → exploratory `inconclusive`; the
cohort-p95 rule (never the average of run percentiles) pinned by a pooled
fixture.

**Live evidence** (`docs/evidence/2026-10-26-at20-ab-analysis/`):
`ramshaper-bench analyze-ab --blocks fixture-5-blocks.json --seed 42` over
a marked-synthetic fixture (5 blocks × 20 events; baseline 100 ms with one
preserved missing event per run; treatment 90 ms) → paired +10% ×5, CI
[10.00, 10.00], label `validated_improvement` with the rule trail — every
number independently derivable by hand.

**Workspace gates (2026-10-26):** fmt clean; clippy workspace-wide
`-D warnings` clean; `cargo test --workspace --locked` **343 passed,
0 failed**.

**Not in this slice (recorded, not hidden):** the A/B *runner* (bounded
synthetic workload with stop control — needs the enabled actuator,
owner-gated); safety/completeness accounting from a real runner; RS-024
Stage C (wizard, export) at G5.

---

# RS-024 Stage B runner — synthetic workload pilot, 2026-10-26

**Scope implemented:** `ramshaper-bench::workload` (+6 tests) and the
`workload-ab` CLI: a bounded, allocation-free CPU task-latency workload
(≥ 100 measured events per run enforced, warmup executed then excluded by
design, stop control with `StoppedEarly { executed }` completion records),
a counterbalanced AB/BA block driver (seeded order, executed order
recorded), and conversion into the AT-20 analysis input (complete pairs
only; stopped series retained in the record). The treatment is an injected
closure — observe-only today; the owner-gated policy action lands at that
seam without harness changes. **The pilot claims nothing**: with a no-op
treatment the honest result is noise.

**Live evidence** (`docs/evidence/2026-10-26-rs024b-workload-pilot/`):
`workload-ab --pilot-blocks 10 --events 100 --seed 4242` — 10 AB/BA blocks,
all complete, 2.2 s wall; median −1.6%, CI [−15.7, +33.3]; label
`regression` via the 2% completion guardrail (worst-case noise "slowdown"
15.7% > 2%) with the observe-only note printed beside it.

**Tests:** schedule completeness (warmup excluded, 100 measured events,
all positive), pre-fired and mid-run stop control (nothing executes after
a pre-fired stop; a mid-run stop records the count), treatment closure
invoked exactly once per measured task (never for warmup), counterbalanced
orders both present with per-block ids intact, and incomplete pairs
excluded from analysis while retained in the record.

**Workspace gates:** bench fmt/clippy `-D warnings`/tests 18 passed /
0 failed; workspace `cargo test` **358 passed, 0 failed** (the concurrent
session's in-flight ui edit has a formatting-only diff at write time; the
agent crate's in-flight dispatch work is theirs).

**Not in this slice (recorded, not hidden):** the real treatment (owner
gate), per doc 08 the *pressure*-style workload variant (disposable
environment only), and the preregistration document that freezes the
thresholds before a real trial — the labels' thresholds are code constants
here, and a real run must declare them externally first.

---

# RS-013 first slice — Actions UI + ListActions completion, 2026-10-26

**Scope implemented:** the action journal is now visible end to end.
Agent side: `ListActions` serves **real journal records** when the agent
was started with the policy gate — unresolved records first (the
pending/conflict set), then the newest history window
(`JournalStore::recent`, capped at 50 rows so a response can never
approach the 1 MiB frame cap), deduplicated by action ID; every row
carries state, target label, pid, original/desired, timestamps and
pending/pinned flags. Gateless agents (the production default) keep the
existing observe-only stub — they have no journal at all.
UI side: a fourth **Actions page** (`--page actions`) rendering the
records with state/deadline/pending text (no color-only information, per
the AT-19 rule; each row is one accessible label), plus the
**Restore** (per pending row) and **Disable all + restore** controls wired
to `RestoreAction`/`DisableActions`. A control's outcome — including an
agent's `Unsupported` refusal — is rendered verbatim in the page note,
never a silent no-op.

**G3-gating boundary kept:** production agents still start gateless, so
the controls answer `unsupported: this agent was started without the
policy gate` and the page explains it. Starting actions from the UI (the
preview/confirm consent flow) is deliberately not built — that is the
owner-gated G3 consent flow. The wiring is exercised in tests against a
policy-enabled gate (dispatch-level ListActions over a seeded journal;
live write/restore covered by the RS-016 e2e).

**Tests:** agent 25 lib tests (incl. `list_actions_serves_journal_records_when_the_gate_exists`
— one seeded Applied record served with original/desired and the correct
unresolved=1 accounting); UI 13 (incl. the row-model tests: pending vs
history badges, Conflict not restorable, expired/future deadline text,
state announced as text on every row, empty stub renders nothing).
Workspace gates: fmt clean, clippy `-D warnings` clean, **357 passed /
0 failed**. Evidence in `docs/evidence/2026-10-26-rs013-actions-ui/`.

**Not in this slice:** the preview/confirm consent UI (G3 flow, owner
call); startup reconciliation surfacing in the UI beyond record states;
the interactive AT-19 pass for the new page (checklist-ready, human task).

# G3 consent path — session consent + clean-exit trigger, 2026-10-26

**Scope implemented:** `ramshaper-agent::consent` + the agent bin's
`--policy-journal <path>` + the `AgentHandle::stop` clean-exit trigger.
Before this slice the policy gate existed only behind the API — the
shipped agent was always started gateless (`serve` received `None`), so no
operator could ever reach it. Contract 14 §5 shape, now shipped: the
explicit journal path is the only opt-in (the operator states where
recovery records live; no default, no bare flag, no config route); the
gate starts **disabled** — Observe Only with the restore loop and startup
reconciliation active; enabling is the **typed console consent** (exact
word `ENABLE`, strict: case-sensitive, alone on the line; end of input
declines, never guesses), per-session, never persisted, never grantable
over IPC. User exclusions flow into the gate; the production allowlist
stays empty until RS-024 Stage B evidence exists. The clean-exit trigger
(§6's "clean agent exit") disables the gate and restores every outstanding
record through the engine before the workers shut down, while the pipe
still serves.

**Pinned by test (+5, all live on this rig):** the strict consent decision
(`only_the_exact_word_consents`, `everything_else_stays_observe_only` — a
lowercase word, extra words, an empty line and EOF all decline); the flag
(`policy_journal_flag_selects_the_policy_stage`, usage error without a
value); the real-pipe distinction
(`policy_commands_with_a_present_but_disabled_gate_answer_busy_not_unsupported`:
gateless agent → `Unsupported`, journal-backed but unconsented agent →
`Busy` naming the disabled session); and
`stopping_the_agent_runs_the_clean_exit_trigger` — an applied action on
the test's spawned `ping` child (injected Elevated + allowlist seam, the
RS-016 gate-evidence stand-in) is restored to Normal by `handle.stop()`,
the gate ends disabled, and the journal record is terminal `Restored`.

**Workspace gates (2026-10-26):** `cargo fmt -p ramshaper-agent` clean;
`cargo clippy -p ramshaper-agent --all-targets -- -D warnings` clean;
agent crate 25 lib + 15 ipc + 5 policy_ipc + 6 integration green; full
workspace **357 passed, 0 failed** (run in
`docs/evidence/2026-10-26-g3-consent-path/workspace-run.txt`). ADR-038.

**Not in this slice (recorded, not hidden):** the UI consent dialog
(RS-013's controls render refusals; the consent act lives on the agent
console until the UI session builds it); the eligibility allowlist (empty
by design); AT-14's live child-inheritance probe (needs the enabled
session with a populated allowlist — now startable, evidence pending).
**Collision note:** the RS-013 session's 59fd21e swept this slice's agent
sources into 0.3.32 while `consent.rs` was untracked — 0.3.32 as committed
did not compile; 0.3.33 completes it (the green gate numbers in both
entries were measured on the integrated working tree).

---

# RS-026 — boundary review of the policy stage, 2026-10-26

**Scope reviewed** (the surfaces that changed since the last boundary
review, AT-23 0.3.13): the policy IPC commands (`PreviewAction`,
`ConfirmAction`, `RestoreAction`, `DisableActions`), the G3 consent path
(`--policy-journal` + typed `ENABLE`, landed as 0.3.33), the SQLite
action journal, the `ListActions` record surface, and the dependency
graph of the new modules.

**Static checks** (evidence `docs/evidence/2026-10-26-rs026-policy-boundary/static-checks.txt`):
- `cargo tree -i tokio` on the host target: **absent** (as at AT-23; the
  wasm-only path is unchanged). No network API usage in any policy-stage
  module (`policy.rs`, `consent.rs`, `policy_loop.rs`, `priority.rs`,
  `coordinator.rs`, journal lib) — grep for
  `TcpStream|UdpSocket|TcpListener|std::net|tokio|reqwest|hyper`: empty.
- Dependency additions since 0.3.22: exactly one windows-rs **feature**
  (`Win32_System_JobObjects`, the RS-020 spike) on the already-pinned
  `=0.62.2` crate. No new crates.
- `rusqlite` (bundled) remains the only storage dependency, reaching only
  the agent via `ramshaper-journal`.

**Live probes** (gated agent on a real pipe, stdin closed ⇒ consent
declined; `live-probes.txt`):
- **Journal privacy FINDING, fixed in this review**: a newly created
  journal file inherited the directory ACL — on this machine,
  `Authenticated Users` had Modify. Recovery records (target labels,
  pids, original values) are user-private per contract 14 §6, like the
  pipe's explicit user-only DACL. Fix: new
  `ramshaper-windows::token::restrict_file_to_current_user` (single
  grant-all ACE for the current user, `PROTECTED_DACL` so no inherited
  ACEs flow), applied at gate open in `consent::open_session_gate`.
  Verified live: the journal now shows exactly one ACE
  (`KINAFORGE-STUDI\jmpat:(F)`). Two implementation iterations recorded
  honestly (relative-path error 87; missing PROTECTED flag let
  inheritance flow). A hardening failure blocks the policy stage (exit 1)
  — it never leaves the journal unprotected.
- Export stays identity-free; the agent holds no tcp/udp sockets
  (netstat for the agent PID: empty) — policy adds no network surface.
- Consent semantics verified live: closed stdin ⇒ "no consent given; the
  session stays Observe Only", restore loop active, agent serves
  read-only normally.

**Adversarial in-process probes** (`tests/policy_boundary.rs`, 3 tests,
real pipe + real journal): with the gate present-but-disabled, a same-user
client gets typed refusals for every action command (Preview `Busy`
explaining the disabled session; Confirm/Restore `InvalidArgument` —
binding/record checks run first; nothing succeeds without the consent
word); `DisableActions` succeeds as an **idempotent no-op that can only
reduce authority** (empty drain), pinned deliberately; malformed payloads
(non-numeric FILETIME, wrong shape, wrong JSON type) are
`InvalidArgument`, never a panic; probe requests for never-previewed IDs
leave `ListActions` empty (no fabricated journal state); `ListActions`
stays readable while disabled (observation, not action).

**Gates (2026-10-26, excluding the sibling session's in-flight
`ramshaper-bench` crate):** clippy `-D warnings` clean across the other
seven crates; **347 passed / 0 failed** (incl. the 3 new boundary tests).
Their in-flight `policy_ipc::child_spawned_after_apply…` AT-14 live test
is timing-flaky on a cold PowerShell start (failed once, passed on
re-run; their uncommitted file, recorded not touched — mechanical
unblock of two clippy items in their quiet >25 min `policy_loop.rs`
applied per convention).

**Not covered here:** the bench workload/analysis boundary (sibling's
in-flight RS-024 Stage B slice — reviewed when it lands); RS-027
packaging/provenance (release-gated).

# Policy-loop ticker — live safety feed + AT-14 inheritance probe, 2026-10-26

**Scope implemented:** the loop 0.3.28 shipped was constructed transiently
for startup reconciliation and dropped — no `tick` or `observe_safety`
ever ran in the shipped agent, so TTL expiry, the child-spawned /
foreground-return / stale-input trips, due retries and missed-deadline
accounting existed only in tests. This slice: `serve()` keeps the loop on
a **1 s stop-event-driven ticker** whose feed observes the outstanding
Applied target through the R-03 adapters — children of the applied target
(an applied target had zero at apply, so any child row is a new
descendant) become `ChildSpawned` with the child's exact identity for the
AT-14 finding; only a positive foreground observation trips; a snapshot
older than the 2 s freshness window is the `FocusStale` trip (session
lock has no adapter and is not fed — recorded gap). The loop now also
**captures the `CompatibilityFinding`** (child identity + image +
timestamp, parent label from the record's intent) into a findings sink —
0.3.28's ticker discarded it, breaking AT-14's "incompatibility is
recorded".

**Repair found live:** the loop's TTL expiry read an in-memory deadline
map populated only by `note_applied` — a bridge nothing in production
calls (the gate applies through its own coordinator), so **an applied
action stayed applied forever**; the new
`ttl_expiry_restores_through_the_live_ticker` test pinned it and the fix
makes the journal the single source of truth (expiry derives from
`unresolved_ids` + `applied_deadline_unix_ms`, exactly like startup
recovery). The dead map, `note_applied` and the loop's unused
`confirm`/`coordinator` wrappers are removed.

**AT-14 live probe (twice-stable):** a consented `powershell.exe` parent
is applied Below Normal; 3 s in it spawns a `ping` child; the test proves
**the child inherited Below Normal at creation** (the exact
compatibility risk RS-024 Stage B must test), then the ticker trips the
child-spawned guardrail and **the parent restores to Normal** while the
descendant receives no write. Fixture finding recorded: `Start-Process`
defaults to ShellExecute and the child can be broker-launched with a
different Toolhelp parent (observed: zero same-parent rows);
`UseShellExecute = $false` forces direct CreateProcess parenting.

**Gates (2026-10-26):** fmt clean; clippy `-D warnings` clean; agent
crate **59 passed, 0 failed** (28 lib incl. the findings/identity/stale
tests + 3 boundary + 15 ipc + 8 policy_ipc incl. both live ticker tests);
workspace run in `docs/evidence/2026-10-26-policy-loop-ticker/`. ADR-039.

# Session-lock restore trigger — WTS adapter + ticker feed, 2026-10-26

**Scope implemented:** the last contract 14 §6 restore trigger that could
not fire live. The `SessionLocked` observation and its guardrail trip
existed (0.3.27) and the loop mapped the trigger (0.3.28), but no OS
adapter produced the observation — 0.3.35's ticker feed recorded the gap.
Now: core `SessionLockObservation` (`Locked`/`Unlocked`/`Unknown`);
`ramshaper_windows::eligibility::observe_session_lock()` via
`WTSQuerySessionInformationW` + `WTSSessionInfoEx` (server handle `None`
= current server — `WTSCurrentServerHandle` does not exist in windows-rs
0.62; `SessionFlags` compared against the documented
`WTS_SESSIONSTATE_LOCK`(0)/`UNLOCK`(1); buffer freed on every path;
level/size and undocumented values are `Unknown`; the documented pre-Win8
flag reversal is noted, build 26200 uses correct semantics); the ticker
feed polls it each pass and only a positive `Locked` trips.

**Evidence:** live adapter probe resolves `Unlocked` on this attended
interactive rig (`LockWorkStation` is never called in tests — it would
lock the operator's real screen); the loop trip test pins the
observation → restore path. **Boundary:** the end-to-end
lock→trip→restore chain is parts-covered + inspection-verified, not
forced live, for the same operator-hostility reason. Gates: fmt/clippy
(`-D warnings`) clean on core/windows/agent; windows **45/0**, agent
**60/0**, workspace **373/0**
(`docs/evidence/2026-10-26-session-lock-trigger/`). ADR-040.

---

# Trial preregistration — executable enforcement (RS-024 Stage B), 2026-10-26

**Scope implemented:** `ramshaper-bench::prereg` (+6 tests) and
`--preregistration` on both `workload-ab` and `analyze-ab`: a frozen JSON
plan (workload schedule incl. the AB/BA order seed, bootstrap parameters,
decision thresholds, nonempty workload declaration; closed schema,
structural rules at parse) that both tools verify before acting — refusing
with **every deviation named** (order-seed re-rolling after peeking is
explicitly covered). Defaults without a plan remain for exploratory runs
(the analysis labels those honestly).

**Pinned by test:** the sample plan parses and verifies clean against a
matching spec; unknown fields, wrong schema versions, sub-minimum event
counts, empty declarations and nonpositive thresholds are refused at
parse; a mismatching run names **all five** workload deviations (not just
the first); seed/resample and threshold deviations are caught on the
analysis side; matching plans verify clean.

**Live evidence** (`docs/evidence/2026-10-26-rs024b-preregistration/`):
a deviation on the order seed → `workload.ab ... refusing` with the
deviation named; a matching plan executes; the illustrative plan file
itself (marked as a demo, not a real trial's preregistration).

**Workspace gates (2026-10-26):** bench fmt/clippy `-D warnings`/tests
24 passed / 0 failed; workspace `cargo test --locked` **373 passed,
0 failed** (bench 24: 3 Stage A + 8 AT-20 + 6 workload + 6 prereg + 1
percentile; the concurrent session's 0.3.34–0.3.37 increments included).

**Not in this slice (recorded, not hidden):** the real trial (owner-gated
actuation consent), RS-024 Stage C plan generation in the wizard, and the
preregistration's declared_at ordering versus collection (operator
discipline, documented).

---

# RS-026 continuation — bench-crate boundary review, 2026-10-26

**Scope reviewed** (surfaces landed since the 0.3.34 review): the RS-024
Stage B runner (`workload.rs`), the AT-20 analysis (`analysis.rs`) and
executable trial preregistration (`prereg.rs`, 0.3.38).

**Static checks** (evidence
`docs/evidence/2026-10-26-rs026-bench-boundary/probes.txt`): no
dependency changes since 0.3.34 (empty Cargo diff); no network APIs in
any bench module; the preregistration schema is genuinely closed
(`#[serde(deny_unknown_fields)]` on every struct).

**Boundary FINDING, fixed in this review:** the frozen plan was not the
authority for the executed trial. `workload-ab` verified only the
*workload* section and then ran the internal analysis with hardcoded
defaults — a plan freezing `resamples: 500` was silently analyzed at
10,000 with default seed and default guardrails. `analyze-ab` refused
deviations (loud, correct) but had no way to *honor* a non-default plan
(its config was defaults plus `--seed`). Fix (both tools, in
`main.rs`): with a plan present, `BootstrapConfig` and `Guardrails` are
built **from the plan** — the executed analysis matches the frozen one —
and `verify_analysis` gates `min_blocks_confirmatory` against this
build's fixed threshold, so a plan freezing a different value is
*refused* (`thresholds.min_blocks_confirmatory: planned 12, attempted
5`), never silently ignored or silently honored. Verified live: an
honored plan runs with `summary.resamples = 10000, seed = 42` (the
plan's values); the un-honorable plan exits 2; seed deviations still
name both parameters. Supporting restructure: the bench crate gained a
`lib.rs` exposing the three modules (standard pattern so integration
tests can exercise the same surface the binary uses; `main.rs` paths
unchanged via one `use`).

**Adversarial in-process tests** (`tests/boundary.rs`, 6 passed): closed
schema refuses unknown fields and wrong schema markers; structural rules
refuse at parse (empty declaration, sub-p95 event count, non-positive
benefit); **all five** simultaneous workload deviations are named (not
just the first); the plan-honoring construction verifies while the
defaults deviate in three named parameters; a non-default confirmatory
threshold refuses; the order-seed re-roll (0.3.38's contract) is a named
deviation with planned vs attempted values.

**Gates (2026-10-26):** bench 30 lib/worker tests + 6 boundary tests, 0
failed; clippy `-D warnings` clean; the other six crates 291/0. The
agent crate's in-flight compile error at gate time is the parallel
session's uncommitted work (touched minutes earlier; recorded, not
touched — their policy_ipc.rs/guardrails.rs/policy_loop.rs edits are in
flight).

**Not covered:** the sibling's in-flight guardrails/policy_loop changes
(reviewed when they land); owner-gated cohort claims remain out of scope
for this machine.

# Suspend/resume restore trigger — interrupt-time divergence, 2026-10-26

**Scope implemented:** the last contract 14 §6 trigger with no production
path — `RestoreTrigger::Suspend` existed (tested) but nothing produced
it, and AT-11 names suspend among the restore triggers. Design (ADR-042):
the windowless agent detects sleep from the interrupt-time domain —
`ramshaper_windows::power::interrupt_time_pair()` (GetTickCount64 advances
through sleep; QueryUnbiasedInterruptTime pauses; new
`Win32_System_WindowsProgramming` feature), core `ResumeDetector` (pure:
growth of the biased−unbiased difference beyond 1 s between two
observations = slept; immune to wall-clock changes and to agent self-
suspension by construction), `SafetyObservation::SystemResumed{sleep_ms}`
→ `TripTrigger::SystemResumed` (code `system_resumed`) → the loop's
`RestoreTrigger::Suspend` (the agent observes the resume; the journal
reason is the suspend). The 0.3.35 ticker samples the detector **every
pass before the outstanding-action early-return** (a stale baseline would
misread the first post-idle jump) and feeds a detected resume even with
nothing outstanding — refused honestly.

**Evidence:** live probe (both counters advance, accumulator stable while
awake); pure detector test (baseline, jitter, jump-with-size, reset
baseline); loop trip test (restores; refuses when nothing outstanding).
**Boundary:** a real sleep is never forced in tests (operator-hostile);
the chain is parts-covered + inspection-verified, the session-lock
standard.

**Parallel-flake repair found during the gate run:** the AT-14
inheritance test missed the child's whole life once under parallel test
threads — Toolhelp snapshots fail transiently under process churn
(documented), and the full `scan_processes` probe was too heavy to retry
enough. The test now probes with the light `enumerate_pid_parents` map at
150 ms over a 25 s window and reports failed-probe counts; the workspace
suite passes in parallel mode after the fix.

**Gates (2026-10-26):** fmt clean; clippy `-D warnings` clean on
core/windows/agent; guardrails 9/0, windows 46/0, agent 61/0 serial;
workspace **382 passed, 0 failed** parallel
(`docs/evidence/2026-10-26-suspend-trigger/`). ADR-042.

---

# RS-027 slice 1 — provenance manifest + packaging/uninstall decisions, 2026-10-27

**Scope implemented:** `scripts/provenance.ps1`, emitting a
`ramshaper.provenance.v1` JSON manifest (toolchain versions, per-release-
binary path/size/SHA-256, `Cargo.lock` hash, pinned toolchain file, git
HEAD + working-tree state; BOM-less UTF-8 — PS 5.1's `Set-Content -Encoding
UTF8` BOM was caught by JSON validation and fixed) — automating the
artifact identification previously hand-copied into these records. Plus
the packaging decisions recorded as decisions: distribution is the folder
(no installer/service/task/Run-key — verified by the AT-23 privacy
review), uninstall = stop processes + delete checkout + delete the
operator-stated journal file (prefer clean-exit restoration first),
login startup off by decision, code signing owner-gated. All of it is now
in the user-facing [FAQ_AND_PRIVACY.md](../../FAQ_AND_PRIVACY.md)
"Packaging, uninstall and provenance" section.

**Live evidence** (`docs/evidence/2026-10-27-rs027-provenance/`):
`provenance.json` generated at HEAD with all four release binaries
(agent, bench, cli, ui), lockfile hash and git state.

**RS-027 remaining (recorded):** the distribution decision itself,
signing (owner; certificate procurement), and any installer/startup work
that decision implies (new ADR required first).

**Workspace gates:** manifest generated and JSON-validated; the script is
read-only over the artifacts. Code unchanged — the workspace test/fmt/
clippy state is the prior record's (0.3.40: 382 passed, 0 failed).

---

# G3 gate assessment — consolidated, 2026-10-27

**Required cases:** G2 + AT-09–AT-14, AT-18, AT-20–AT-21, AT-23.
**Additional exit conditions:** a synthetic workload A/B report; an
eligibility allowlist; all recovery failures inspectable.

## Case-by-case status

| Case | Status | Evidence |
|---|---|---|
| AT-09 eligibility denials | **Passed** (pure + live) | RS-015 matrix: 11 named checks, every `Unknown` denies, explain-all reports; PolicyGate live tests (wrong instance/new identity/exclusions deny with reasons) |
| AT-10 durable prepare → one verified write | **Passed** (pure + live) | Coordinator ordering test (durable `Prepared` asserted *during* the write); real actuator on a spawned `ping` child: Normal → Below Normal with read-back + independent confirmation, restored (0.3.24) |
| AT-11 restore triggers | **Passed** (pure + live) | Restore engine: all 11 triggers, 1/5/30 s bounded retries, missed deadlines counted; live triggers: clean agent exit (0.3.33), session lock (0.3.37), suspend/resume (0.3.40), TTL + retry ticks (0.3.35), foreground via guardrails |
| AT-12 crash reconciliation | **Passed** (pure + real store) | Reconcile-from-current-value (never reapply; reused PIDs zero writes; nothing inferred) over the real SQLite journal across restarts; optimistic state guard |
| AT-13 external conflict | **Passed** | Third-value reconciliation → `Conflict` preserved untouched, pinned, surfaced via `ListActions` |
| AT-14 cap / cooldown / children | **Passed** (pure + live, twice-stable) | Guardrails books + live child-inheritance probe (consented `powershell.exe` parent, spawned child observed → parent restored, finding recorded; 0.3.35) |
| AT-18 storage faults | **Passed** (deterministic + adversarial) | Disk-full via page cap (typed `DiskFull`, recovery proceeds), clear-history retains pinned intent, newer-schema refusal untouched, quota blocks; +0.3.39 boundary-review adversarial probes |
| AT-20 analysis fixtures | **Passed** | Hand-computed nearest-rank/paired/bootstrap fixtures; four doc-08 labels; deterministic seed replays |
| AT-21 declared cohorts | **Not run — owner** | Cohort hardware unavailable; classification implemented and live-verified on the non-cohort baseline path (0.3.2/ADR-024) |
| AT-23 privacy/offline | **Passed** (G2 portions + uninstall disclosure) | Offline/no-network review (0.3.13); identity-free config exports; uninstall disclosure + journal-location guidance (0.3.41 FAQ); journal serves only to the same user over the DACL'd pipe |

## Exit conditions

- **Synthetic workload A/B report:** machinery complete end to end
  (`workload-ab` → counterbalanced pilot → preregistration enforcement →
  `analysis` → labels; the observe-only pilot runs live). The **real**
  report requires an owner-enabled trial (consent + a declared synthetic
  workload) — **owner**.
- **Eligibility allowlist:** intentionally empty; population requires
  RS-024 Stage B compatibility testing on real workloads — **owner**.
- **All recovery failures inspectable:** **Passed** — `Failed`,
  `Uncertain`, `Conflict`, `TargetGone` and pinned `RestorePending` states
  are journal records surfaced through `ListActions` and rendered verbatim
  in the Actions UI; bounded retries and missed verification deadlines are
  counted, never fabricated.

## Prerequisites and blockers

**G2 first.** G3 cannot be signed off until G2's own exit conditions
close: the **Slint license decision** (owner pick — the UI links
`slint 1.18.1` under its triple license today) and the **AT-19
interactive pass** (human: Narrator quality, 1366×768, the 100–200 % DPI
matrix; the automated portion is done). The Actions UI itself is complete
(RS-013 first slice: history/pending/conflict rendering, restore and
disable controls that answer explained refusals on gateless agents).

**Owner actions standing between the current state and a G3 sign-off:**

1. Slint license selection (G2 exit; owner).
2. AT-19 interactive pass (human; checklist prepared in
   `docs/design/2026-10-24-at19-interactive-checklist.md`).
3. Enable the consent flow for a declared synthetic workload and run the
   preregistered Stage B trial → the A/B report and the first allowlist
   entries (owner; the machinery — consent, gate, workload, prereg,
   analysis — is complete and tested). **Correction 2026-10-28:** the
   original text called the trial machinery "complete" — overstated; an
   actually runnable consented trial is blocked by four concrete gaps
   (no operator-facing allowlist path — the gate hardcodes an empty
   allowlist; classification Unknown-denies every non-self target and the
   override is constructor-only; no subject-process isolation — the
   workload runs in the bench process, which self-identifies as
   protected; no bench→gate treatment path). Design memo with options,
   recommendations and the follow-up ticket list:
   [design/2026-10-28-stage-b-trial-design.md](../design/2026-10-28-stage-b-trial-design.md).
4. AT-21 cohort hardware (owner; no substitute is possible by design).

**Not blocking, recorded:** the policy-loop ticker runs on a live 1 s
feed with the journal as its input (0.3.35); session-lock and suspend
triggers fire live (0.3.37/0.3.40); the preregistration enforcement makes
the frozen-thresholds rule tamper-evident (0.3.38); provenance manifests
automate artifact identification (0.3.41).

**Assessment:** every G3 case that can be evidenced without owner
resources is **Passed** with recorded limitations; the remaining items are
precisely the owner-gated set above. No automated work remains open on
G3's own track.

# RS-019 completion — `policies --dry-run` CLI command, 2026-10-27

**Scope implemented:** the command RS-019 explicitly deferred ("not in
this slice: the `policies --dry-run` CLI command") — contract 14 §8 names
it and gates it on the capability existing, which the RS-019 engine (20
tests, deterministic, zero OS writes) satisfied. `ramshaper-cli policies
--dry-run [--json]` now runs the decision layer over a built-in
**synthetic** trace (`ramshaper_core::policy::demo_trace`, 21 steps):
pressure transitions including the 3-sample Unknown ramp and the missing-
input reset, preview → confirm → simulated apply, idempotent replay, the
one-action cap, external-conflict preservation, journal-unavailability
blocking, an unknown-focus deny (reasons carried), preview expiry past
the inclusive 30 s window, TTL resolution, a stray confirmation, and the
disable path — every `PolicyDecision` variant and every `NoActionReason`
in one pinned sequence.

**Semantics discovered while pinning (recorded):** the pressure
classifier stays `Unknown` until three consecutive valid samples (so a
single elevated sample never offers); a `disable` step only acts on an
**active** experiment — with only a preview pending it is a no-op and
the preview re-offers with its id — so the demo trace confirms dry-4
before disabling, which additionally covers `ResolveCause::Disabled`.
The pinning test (`demo_trace_pins_the_full_decision_sequence`) is the
executable spec of the printed output and asserts byte-identical
determinism across runs.

**Surface:** `--json` emits the versioned `ramshaper.policy-dryrun.v1`
log verbatim; text mode carries a SYNTHETIC banner (contract §8 "fixture
data must be marked synthetic"), per-step lines
(`[i] ms pressure decision`) with stable machine codes, and the summary
counts. Usage errors exit 2 (`policies` without `--dry-run`, the flag on
other commands); the run itself exits 0 (§8: deterministic success).

**Gates (2026-10-27):** fmt clean; clippy `-D warnings` clean on
core+cli; core pinning test green; cli **9 parse + 22 binary passed,
0 failed** incl. byte-identical determinism over the real binary, stdout/
stderr separation and usage-error exits (`docs/evidence/2026-10-27-
policies-dryrun-cli/test-run.txt`). **Workspace note:** the full-suite
run recorded 2 failures in the sibling session's UNCOMMITTED in-flight
`job.rs` compatibility tests (fail serially too — theirs to finish;
recorded, not touched); every other crate green.

**Not in this slice:** user-supplied trace files (a `--trace <file>`
schema over `TargetObservation` — a future extension; **shipped
2026-10-28**, see the RS-019 addendum below); the managed-launch
`restore` command family (G4).

---

# RS-021 — job membership/child observation + compatibility report, 2026-10-27

**Scope implemented:** the read-only observation half of the managed-
launch track (doc 07 RS-021; doc 04 §D's "detect incompatible preexisting
jobs; do not escape them"), layered on the RS-020 spike's
`ramshaper-windows::job`:

- `observe_membership(pid, job?) -> MembershipObservation` — two
  documented `IsProcessInJob` queries through a query-limited handle:
  the environment fact (`in_any_job`) plus membership in one specific
  advisory job (`in_queried_job`). Nothing is attached, detached or
  broken away.
- `CompatibilityReport` / `compatibility_report(target?, job?)` — the
  launcher-environment preflight: `launcher_in_job` (managed children
  would nest), the target's membership, and **honest notes** — a jobbed
  launcher carries the nesting note ("the foreign job's limits are not
  queryable... the launcher must refuse or accept nesting explicitly"),
  a target in a *foreign* job carries the can't-undo note. The notes are
  the data; the report never guesses safety.
- `AdvisoryJob::active_processes()` — `JOBOBJECT_BASIC_ACCOUNTING_INFORMATION`
  ActiveProcesses, the RS-022 group-accounting groundwork.

**No-breakaway proof by lifecycle** (live test): a member is in the id
list and the count while alive; after it is terminated the count drains
to 0; the queried limit flags stay **0 through the whole lifecycle** —
this module has no removal path other than process exit (no
`SetInformationJobObject` call exists in it).

**Environmental finding, pinned by the tests (first observed here):**
the cargo-test process tree **is itself inside a job** on this machine
(an ancestor host job — modern Windows hosts organize processes into
jobs), and children inherit membership. Two consequences pinned:
"not in any job" must never be asserted absolutely (tests use relative
facts), and the compatibility report *correctly fires its nesting note
in ordinary development environments* — which is exactly the decision
input the future launcher's accept/refuse needs. Second pinned fact: a
console `powershell` child drags its `conhost.exe` into the job too —
**the whole subtree inherits, including invisible hosts** (3 members,
not 2), which is the compatibility risk the Stage B allowlist exists to
test.

**Live results** (10 job tests, all green; evidence
`docs/evidence/2026-10-27-rs021-job-observation/live-tests.txt`):
membership observation distinguishes any-job/advisory-job/no-job;
the compatibility report is self-consistent against a direct query and
scopes its notes correctly (foreign note only for a genuinely foreign
target job); grandchild inheritance shows every subtree member observing
as a member with the accounting count matching the id list.

**Boundary:** still no production caller — RS-022 (group accounting/
advisory thresholds) and RS-023 (launch consent, non-detachable
semantics) build on this; the launcher itself stays unavailable until
the G4 safety gate passes. No forced breakaway exists anywhere in the
module; "unmanaged retry" is absent by construction (creation-time
assignment has no retry window — RS-020's finding, unchanged).

**Workspace gates (2026-10-27):** windows crate fmt/clippy (`-D
warnings`) clean, **50 passed / 0 failed** (28→34 lib incl. the new
tests + integration + pipe probe).

---

# RS-022 — group accounting + advisory thresholds, 2026-10-27

**Scope implemented:** the accounting half of the managed-launch track
(doc 07 RS-022), layered on RS-021's observation surface in
`ramshaper-windows::job`:

- `AdvisoryJob::accounting() -> GroupAccounting` — the full
  `JOBOBJECT_BASIC_ACCOUNTING_INFORMATION` query with owned typed fields
  (total/active/terminated process counts, user/kernel CPU time in
  100-ns units, page faults) plus a `total_cpu_time_100ns()` helper.
- `AdvisoryThresholds` + `evaluate_advisory(accounting, thresholds) ->
  AdvisoryStatus` — **pure, software-side** threshold evaluation
  (equality is within: thresholds are inclusive bounds; zero = not
  evaluated; exceeded thresholds carry stable machine codes
  `advisory_active_processes` / `advisory_total_cpu_time`).
- **Advisory means advisory**: nothing is ever handed to
  `SetInformationJobObject` — no hard limits, no time limits, no
  process-count limits, no kill-on-close, no breakaway. Exceeding a
  threshold is a *reported fact*; what happens next belongs to the
  consent-gated layers above. A test pins that an evaluation never
  mutates the job (limit flags stay 0).
- **Survive-agent-exit evidence** (RS-022's own clause): every handle
  (job + child) dropped while the child runs → the child completes
  unaided. Honest limitation recorded: a crashed agent also loses the
  accounting *view* — it cannot read a job it no longer holds; survival
  is asserted by the child's own completion (the OS-level kill-everything
  crash test remains AT-15 rig work).

**Live findings (pinned by tests, not hidden):**
1. A freshly spawned child charges **zero CPU time** at the first query —
   time accumulates when the child wakes, so observations poll bounded
   and the child must outlive the poll window.
2. `TotalTerminatedProcesses` stayed 0 after a member was **force-terminated**
   (taskkill /F) while ActiveProcesses drained to 0 — the counter is only
   trustworthy for natural exits; an advisory report must not present it
   as a complete kill count.
3. A transient `CreateProcessW` **code 5** under parallel test spawn load
   (serial rerun green; suspected Defender first-launch scan) — recorded
   as the reason the suite runs serially.

**Live results** (54 passed / 0 failed serially, incl. 5 new tests:
accounting counts + CPU accumulation, the pure evaluator paths [within/
both-exceeded/zero-opt-out], advisory exceed without job mutation,
survive-agent-exit with the accounting view; evidence
`docs/evidence/2026-10-27-rs022-group-accounting/live-tests.txt`).

**Boundary:** still no production caller (RS-023 launch consent and the
G4 launcher come later; the launcher stays unavailable until its safety
gate passes). The pure evaluator is deliberately in the windows crate —
promoting it to core happens when a consumer (RS-023) needs it there.

**Workspace gates (2026-10-27):** windows crate fmt/clippy (`-D
warnings`) clean, **54 passed / 0 failed** serially (38 lib + 15
integration + 1 pipe probe). The sibling session's in-flight
edits (guardrails Stage-B wiring in core/windows eligibility surfaces)
were in progress during this slice and were not touched; the RS-021-era
flake fix on this module's compatibility test (`ping -n 5` → `-n 30`
under parallel suite load, attributed to the parallel session) folds
into this slice's commit.

---

# AT-22 rerun-bundle assembler — protocol tooling, 2026-10-27

**Scope implemented:** `scripts/rerun-bundle.ps1` — assembles a rerun
bundle (AT-22's evidence class: "Scripts, checksums, raw data and
reports") from a preregistration + analysis report (+ optional raw data):
inputs copied verbatim into `raw/`, build provenance regenerated into the
bundle, `SHA256SUMS.txt` covering every bundled file, and `RERUN.md` with
the exact second-operator procedure (rebuild at the recorded HEAD, verify
binary hashes, re-run per the frozen plan, re-analyze with the frozen
bootstrap parameters, compare decision structure rather than
machine-dependent latency values). **The bundle directory is immutable
once assembled** — re-assembly is refused with the reason. The real AT-22
rerun (second operator, three workload classes) remains G5/owner after
real trials; this is the protocol tooling.

**PS 5.1 quirks fixed during validation (recorded for future scripts):**
`$input` is a reserved automatic variable (clobbering it breaks later
pipelines); `ForEach-Object` threw `MethodNotFound` where a statement
loop works; `[IO.Path]::GetRelativePath` does not exist on .NET Framework
(use `Substring` from a resolved root); `Set-Content -Encoding UTF8`
writes a BOM (use `[IO.File]::WriteAllText` with
`UTF8Encoding($false)`); `$(Get-Date).ToUniversalTime()` stops at the
cmdlet boundary (assign the formatted string first); `Join-Path` with an
absolute child produces an unsupported path (root-check before joining).
The immutability guard was itself exercised: partial assemblies from
failed runs are refused on re-run until cleaned.

**Live evidence** (`docs/evidence/2026-10-27-at22-rerun-bundle/`): the
demo bundle (fixture + regenerated report + preregistration demo plan)
with working checksums and instructions, plus the guard's refusal.

# RS-026 continuation round 2 — policy-surface boundary review, 2026-10-27

**Surfaces reviewed (everything that changed since the 0.3.34 policy
review):** the policy-loop ticker + server safety feed (0.3.35), the WTS
session-lock adapter (0.3.37), the power/interrupt-time adapter + suspend
path (0.3.40), the consent `--policy-journal` CLI path (0.3.33/0.3.38),
and the `policies --dry-run` CLI (0.3.43).

**Static findings (all clean or recorded):**
- Dependency graph: **zero new crates** since 0.3.34 (`Cargo.lock` diff is
  empty); exactly one new binding surface — the
  `Win32_System_WindowsProgramming` feature on the pinned windows crate
  (0.3.40, the two interrupt-time queries), the same audit class as
  0.3.34's `Win32_System_JobObjects` finding;
- `unsafe` remains confined to `ramshaper-windows` (`#![deny(unsafe_code)]`
  everywhere else; verified by scan);
- the shipped agent's gate and ticker share **one** SQLite connection
  behind a mutex — no SQLITE_BUSY is reachable between them by
  construction.

**Finding 1 (fixed): unbounded consent stdin read.** The typed-consent
prompt used `stdin().read_line`, which grows the line buffer without
limit — a hostile or accidental oversized paste grows memory unboundedly
on the operator console path. Fixed with a 4 KiB `Take` bound (the
consent word is 6 bytes; anything beyond 4 KiB is discarded unread and
cannot enable the session — a truncated prefix can never equal the strict
consent word unless the real word was typed).

**Finding 2 (fixed): a second journal handle had no busy timeout.** The
gate/ticker share one connection, but `JournalStore::open` on the same
file from another connection (diagnostics exporter, a future reader
process) failed **immediately** with `SQLITE_BUSY` under write overlap —
classified as an opaque `Db` fault with no retry semantics. Fixed with a
bounded 2 s `busy_timeout` at open; the new probe
(`a_second_handle_waits_behind_the_first_under_write_load`, journal
crate) hammers 200 concurrent prepares+reads across two handles and
requires zero faults on either side, plus full post-run consistency.

**Probes that closed questions (pinned by test):**
- `a_second_restore_engine_on_a_terminal_record_writes_nothing` — the
  gate-disable vs ticker interleave: the second engine over an already
  restored record finds `NotRestorable` (zero writes, no error);
- `an_applied_record_cannot_survive_startup_recovery` — FocusStale at
  startup is unreachable: a restore failing during recovery lands the
  record in `RestorePending` (manual retry), never leaves it `Applied`,
  so the staleness trip cannot misfire on recovery residue;
- `policies_still_requires_a_loadable_config_file` — the shared run()
  preamble validates `--config` before any command, including the dry
  run that ignores config content: pinned exit 2 with the file named.

**Gates (2026-10-27):** fmt clean; clippy `-D warnings` clean on
journal/agent/cli; journal **11/0** (+1), agent **63/0** (+2), full
workspace **400 passed, 0 failed** parallel —
`docs/evidence/2026-10-27-rs026-round2/`. Not reviewed this round: the
sibling session's in-flight job.rs compatibility surface (0.3.44 landed
it; their round).

---

# RS-023 — launch consent + explicit non-sandbox/non-detachable semantics, 2026-10-28

**Scope implemented:** the managed-launch capability surface (doc 07
RS-023; P0-04; AT-16), layered on the RS-020/021/022 job work:

- `ramshaper-windows::launch` (new module) — the consent flow and the
  managed session:
  - `LaunchRequest` (exact executable path + verbatim argument string)
    with validation: non-empty, no NUL, existing regular file.
    Shortcuts, scripts, directories and non-executable files fail
    explicitly — never silently resolved (P0-04 scope).
  - Typed **per-launch** consent: strict word `LAUNCH` (alone on the
    line; EOF and read errors decline, never guess), a distinct word
    from the policy stage's `ENABLE` so the two consent surfaces cannot
    be confused. `LaunchConsent` is a move-consumed token with a private
    field: **one consent buys at most one launch attempt** — an
    automatic retry or duplicate launch is unrepresentable, and the
    type system enforces it (the "no automatic duplicate launch" clause
    of AT-15's shape).
  - `render_consent_prompt` displays the exact command line, the
    preflight compatibility notes and the four semantics statements
    BEFORE the consent is read — nesting in a pre-existing launcher job
    (this dev machine's normal state) is shown and accepted explicitly
    by consenting after it is displayed (the RS-021 "refuse or accept
    nesting explicitly" decision, resolved as explicit acceptance).
  - `ManagedLaunchSession::launch`: fresh `AdvisoryJob` +
    creation-time spawn + the membership proof; `remove_monitoring`
    queries the final accounting and limit flags BEFORE dropping the
    monitoring handles and returns the child handle plus the semantics
    statements. Removal is a handle drop — **never a detach, never a
    kill** (no such call exists in the module).
- `ramshaper-windows::job` additions: `ManagedChild::spawn_exact` —
  explicit `lpApplicationName`, so the OS launches exactly the named
  image: no PATH search, no shell, no shortcut/association resolution.
  The shared creation core was factored into `create_managed`; the
  membership proof now distinguishes a query failure from a false
  answer via the new `JobError::MembershipUnverified` (the child may
  exist unmanaged — reported, never retried). `exact_command_line` is
  shared with the consent display so what is shown is byte-identical to
  what is executed.
- `ramshaper-cli launch [options] -- <PATH> [ARGS...]` — the shipped
  command (contract 14 §7: "managed-launch commands appear only with
  their gated capabilities"; the gate here is the typed per-launch
  consent, default closed): prompt → consent → launch → advisory
  monitoring loop (accounting samples, optional thresholds) →
  `remove_monitoring` summary → exit. `--advisory-max-active` /
  `--advisory-max-cpu-ms` feed the RS-022 pure evaluator (exceeding is
  reported, never enforced); `--json` emits `ramshaper.launch.v1` /
  `ramshaper.launch-sample.v1`. Exit codes: 0 launched+removed,
  1 declined or refused (explicit outcome), 2 invalid request. The
  child inherits the console, so its output interleaves with the
  CLI's — cosmetic, documented.

**AT-16 pins (all live):** queried limit flags are 0 at launch, in every
sample and at removal; the CLI process exits and the launched app
remains alive (`docs/evidence/2026-10-28-rs023-launch-consent/
survival-check.txt`: CLI gone at t+1.2 s, PING.EXE alive at t+6 s,
explicit-PID cleanup); membership persists after every monitoring handle
is dropped (the kernel keeps the job while a member exists — the
non-detachable fact, asserted in the library test); a non-executable
file fails with the explicit `CreateProcessW` error 193.

**Boundaries kept:** no kill/suspend/detach/retry/limit path anywhere in
the new surface; no persisted or configuration-derived consent (a
profile/import is not consent — AGENTS rule 13); existing processes are
never touched. The managed-beta RELEASE gate stays as the matrix defines
it (G3 sign-off + the full AT-15 actual-lifecycle traces = rig work):
this is the gated capability surface, the same pattern as the policy
stage. The UI-copy clause of AT-16 ("UI never promises detachment or
sandboxing") is carried by the CLI prompt/statements; a launch UI does
not exist yet, and the statements are the module-level API any future UI
must surface.

**Gates (2026-10-28):** fmt + clippy (`-D warnings`) clean workspace-wide
(one mechanical `unused_mut` fix on an RS-022-era job test that the
0.3.45 gates missed — the flake fix folded in after their clippy run);
workspace **416 passed / 0 failed** serially (windows crate serially per
the RS-022 note; the one parallel-run failure was the recorded transient
`CreateProcessW` code-5 under spawn load, green on the serial rerun).
New tests: launch module 10 (2 live) + CLI parser 6. Evidence:
`docs/evidence/2026-10-28-rs023-launch-consent/`. ADR-044.

---

# RS-019 addendum — `policies --dry-run --trace <FILE>`, 2026-10-28

The user-supplied trace extension recorded as future work in the RS-019
0.3.43 slice:

- `ramshaper-core::policy` gained the **closed JSON file schema**
  `ramshaper.policy-trace.v1` (`ParsedTrace::parse`): a target identity
  (pid + exact creation FILETIME), an optional assumed allowlist
  (default `SYNTHETIC-APP.EXE` — a walk input, never evidence), and
  ordered steps. Unknown fields are refused; serde errors carry file
  line/column; a wrong schema tag or an empty step list is a named
  error.
- The step schema mirrors `TraceStep` with owned strings and
  `TraceStep::new`'s defaults: absent `target` observations are
  all-pass, the four step flags default false, the journal defaults
  available, and `low_memory` absent-or-null means missing/stale inputs
  (pressure Unknown). The eligibility vocabulary deserializes as
  snake_case variants; `ProtectedCategory` parses from its stable
  machine codes (`protected_audio`, … — the inverse of its existing
  custom `Serialize`, so audit strings round-trip).
- `identity_check` uses file vocabulary: `matches`, `{mismatch: {pid,
  creation_filetime}}` (fabricates the observed instance from the
  author's stated facts) and `{unreachable: {win32_code}}` (modeled as
  the exited-target query failure). Honest-by-statement, not guessed.
- CLI: `policies --dry-run [--json] [--trace <FILE>]`. The demo path is
  byte-identical to 0.3.43 (the walk + rendering were factored into one
  `print_log`); the file path prints the same SYNTHETIC-marked walk with
  the source path named. Read/parse failures exit 2 (configuration-error
  class).

**Tests:** core +6 (parse/defaults, full preview→apply→disable walk,
refusals [unknown field / wrong schema / empty steps], null-pressure,
ineligible deny codes incl. the check-level `protected_category` code,
unreachable mapping, the ProtectedCategory code round-trip), CLI +2.
Semantics surfaced while writing the walk test (pinned, not assumed):
pressure elevates on the third consecutive elevated sample; a pending
preview is re-offered; stale pressure does NOT resolve an active
experiment. Workspace **425 passed / 0 failed** serially; fmt/clippy
clean. Evidence: `docs/evidence/2026-10-28-policies-trace/`.

---

# AT-15 — launcher-crash lifecycle trace (in-process portion), 2026-10-28

The AT-15 clause "the launcher crashes at each stage" traced live
(`crates/ramshaper-windows/tests/at15_lifecycle.rs`, 2 tests): the crash
stages discretize into three, and the two flanks are now real
two-process evidence — the "launcher" is a re-exec of the test binary
(env-var helper branch) that is **force-terminated** (`taskkill /F`; the
same OS-level fact as a crash without an interactive WER dialog):

- **After creation** (the money shot): the helper creates the fresh
  advisory job + managed child, reports the child's PID, and is killed
  the moment the report arrives. The parent then pins, across the real
  process death: **exactly one child was ever reported** (no automatic
  duplicate launch — now traced across a process boundary, not only
  pinned by the consumed-consent token); the child is alive immediately
  after; the child is **still in a job** (the job object survives while
  a member exists — the non-detachable fact across a crash,
  complementing RS-023's graceful removal); and the child **completes
  unaided** (no suspended orphan, no kill-on-close).
- **Before creation**: a launcher killed pre-spawn reports zero
  children — nothing to orphan.
- **Mid-creation** is not externally interruptible by construction:
  with the creation-time `PROC_THREAD_ATTRIBUTE_JOB_LIST` (RS-020)
  membership and birth are one syscall, so no crash can observe a
  half-created child. That absence IS the no-suspended-orphan guarantee
  (the superseded suspended-create design had the window).

Method gotchas pinned in the test source: the stdout reader must stream
(waiting for EOF deadlocks — EOF requires the kill, the kill requires
the pid line), and the pid line must be matched anywhere in the line
(libtest's partial `test <name> ... ` prefix concatenates onto it).

**Committed alongside (mechanical, attributed):** the agent's
`child_spawned_after_apply_is_inherited_then_trips_the_restore` probe
window widened 25→60 s after two full-workspace-gate failures — root
cause recorded: on a freshly built tree, Defender's real-time scan slows
the powershell parent's cold start past 22 s, so the old window could
expire while the parent was provably still running (success breaks out
early, green runs pay nothing); the panic diagnostic now reads the held
`Child` handle (`try_wait`) instead of `resolve_identity`, which could
report a reused PID as "alive".

**Gates (2026-10-28):** fmt/clippy (`-D warnings`) clean; workspace
**427 passed / 0 failed** serially. Evidence:
`docs/evidence/2026-10-28-at15-launcher-crash/`. Remaining for full
AT-15: the instrumented-rig staged-failure matrix (debugger-injected
crash boundaries inside CreateProcessW) — rig work; the creation-time
design leaves only the two flanks as interruptible stages.

---

# AT-08 UI portion — config surfaces end to end, 2026-10-28

The AT-08 UI-side clause completed (core/config portion Passed
2026-10-24 with RS-012/RS-030):

- **OS call audit:** `ramshaper-ui`'s complete OS surface is
  `window::{find_window_by_title, is_iconic}` (query-only visibility for
  the hidden-window rule) and the named-pipe IPC client. No setters, no
  file writes, no process/job/priority APIs, no config writes. Settings
  knowledge = one read-only `GetConfig` serve-time summary. There is NO
  profile-selection control (the Profiles page marks the active preset;
  selection lives in config/import — and a profile is not consent).
  The UI's two policy commands reach the agent's consent gate; a
  gateless production agent answers `Unsupported`, and even behind a
  gate the OS write happens in the agent, never in the UI process.
- **Live fixtures** (release build at this commit;
  `docs/evidence/2026-10-28-at08-ui-config-surfaces/live-fixtures.txt`):
  fresh install (no config) → GetConfig = observe_only/balanced/[] with
  the inertness note; export → switch profile to low_ram + exclusion →
  validated atomic import → agent serves low_ram + exclusion with mode
  STILL observe_only; agent restart on the same file → identical
  summary; unknown field and unknown profile → exit 2 (field-level
  errors) on both `config import` and agent startup. Profiles-page
  visuals: the 2026-10-24 PrintWindow captures are referenced, not
  redone.
- **Verdict:** AT-08 UI portion **Passed**. The G2 gate row's remaining
  blockers are unchanged (Slint license = owner; AT-19 interactive =
  human).

---

# ramshaper-subject — Stage B trial subject binary, 2026-10-28

Follow-up ticket 2 of the Stage B design memo, built ahead of ADR-045 as
**inert capability** (a workload-runner binary; the ADR gates whether the
agent may act on it, not whether it exists — nothing in the shipped
surface references it yet):

- New crate `ramshaper-subject`: runs the same bounded-CPU workload as
  `ramshaper-bench::workload` by CALLING `run_workload` (spec fidelity by
  construction; the doc-08 >= 100-events minimum is enforced by the bench
  validator, never re-implemented); takes a closed-schema JSON spec
  (field names identical to `WorkloadSpec`); is always observe-only by
  itself (the agent applies any treatment externally through its consent
  gate); emits one `ramshaper.subject-run.v1` JSON record (latencies in
  ms, completion status) and exits 0/1/2 for complete/stopped-early/
  usage.
- **Soak-constraint decisions (recorded in source):** the crate depends
  on the bench lib rather than adding a bin to the bench crate — the
  AT-06 soak holds `ramshaper-bench.exe`, and the release build compiled
  only the new binary (verified live; bench/cli untouched). The spec is
  mirrored because the bench types carry no serde derives and the bench
  crate is frozen during the soak.
- Image name NOT in the agent's self-identification list — it is the
  experiment's subject, not RAMShaper machinery; that decision is
  ADR-045's to ratify.
- **Live evidence** (`docs/evidence/2026-10-28-ramshaper-subject/`):
  release binary, 250-event demo spec, completion complete, p50 0.84 ms
  / p95 1.11 ms. Gates: fmt/clippy clean; workspace **436/0** serially
  (+9 tests, incl. 5 over the real binary and the closed-schema
  refusal).

---

# ramshaper-trial — Stage B trial driver, 2026-10-28

Design-memo follow-up ticket 3 (after the subject binary, ticket 2):

- New crate `ramshaper-trial` (driver split lib + thin main): per treated
  run — spawn the held subject, wait `SUBJECT_READY`, `PreviewAction`
  (identity resolved fresh; FILETIME as the decimal-string wire form),
  `ConfirmAction`, release `GO`, collect the run record, `RestoreAction`
  while the subject still lives (`--exit-delay-ms`) so the journal reads
  `Restored`. Baseline runs issue no wire calls. Seeded per-block order
  coin (local SplitMix64), deterministic action ids (`trial-{run_id}`),
  `analyze-ab`-compatible blocks output. **Fail-fast everywhere**: no
  blocks file for a half-executed experiment; an abandoned held subject
  exits 1 without running untreated.
- **The gate decision under which this was tested (recorded honestly):**
  the happy path is tested at the library boundary with a scripted
  `AgentRpc` — the real wire gate honestly refuses previews without
  fresh Elevated memory-pressure evidence (`pressure_not_elevated`),
  this machine is not pressurized, and fabricating pressure during the
  AT-06 soak is off-limits. The apply/restore mechanics behind the
  scripted answers are proven live in the agent crate's own tests (which
  inject the pressure level through the in-process gate API). The wire
  REFUSAL path is tested live against a real gateless agent over a real
  pipe.
- **A real bug only the live run could catch:** the driver initially
  passed the raw pipe name to `client_connect` while the agent
  normalizes bare names on both of its sides — attempts 1/1b/1c of the
  evidence file are the debugging trail (`OS error 2/3 from CreateFileW`
  where the probe succeeded); fixed by mirroring the agent's
  normalization, after which the typed refusal landed verbatim:
  `Unsupported: this agent was started without the policy gate`, exit 1,
  no blocks file, and the abandoned held subject's own honest line
  (`stdin closed before the go signal; nothing ran`).
- Gates: fmt/clippy clean; workspace **442 passed / 0 failed** serially
  (+6 tests). Evidence: `docs/evidence/2026-10-28-ramshaper-trial/`.
  Remaining for a real trial (unchanged): ADR-045, real Elevated
  pressure evidence at trial time, and the owner's typed `ENABLE`.
