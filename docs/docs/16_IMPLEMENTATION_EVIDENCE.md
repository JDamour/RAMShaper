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
