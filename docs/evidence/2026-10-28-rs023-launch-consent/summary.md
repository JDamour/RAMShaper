# RS-023 — launch consent + non-sandbox/non-detachable semantics (2026-10-28)

Captured live on the dev machine (Win11 26200.9711, standard user) with the
debug build at this commit. Files:

- `live-tests.txt` — `cargo test -p ramshaper-windows launch` (11 tests,
  incl. the two live ping-child tests) and `cargo test -p ramshaper-cli`
  (23 tests), both green.
- `cli-transcript.txt` / `cli-transcript.stderr.txt` — a full CLI session:
  typed `LAUNCH` consent → ping launched into the fresh advisory job
  (samples every 500 ms) → `--stop-after-ms 2500` removes monitoring →
  final accounting + the four semantics statements, exit 0.
- `cli-survival-transcript.txt` — the same flow with a 25 s ping; the
  CLI exited at 1.2 s and ping's replies continue in the captured console
  afterwards (the child outlived the monitoring session by construction).
- `survival-check.txt` — the clean AT-16 check: CLI gone at t+1.2 s,
  `tasklist` shows the launched PING.EXE **alive at t+6 s** (≈5 s after
  monitoring removal); explicit-PID cleanup afterwards (no image-name
  kills).
- `cli-json-transcript.txt` — `--json` mode: `ramshaper.launch.v1`
  launched/monitoring_removed objects and `ramshaper.launch-sample.v1`
  NDJSON samples, `--advisory-max-active 4` accepted. (ping's own console
  output interleaves with stdout — the child inherits the console; the
  JSON lines themselves are well-formed.)

What the evidence pins (AT-16):

1. Queried, not assumed: `limit_flags` is 0 at launch, in every sample,
   and at removal (`final_limit_flags: 0`).
2. Monitoring removal is a handle drop: the CLI process exits, the app
   remains alive and observable (`survival-check.txt`).
3. Membership is not detachable: the library test
   `consented_launch_monitors_and_removal_leaves_the_app_alive` observes
   the child still `in_any_job` after every monitoring handle is dropped.
4. The consent is real: a non-executable file fails with the explicit
   `CreateProcessW` error 193 (nothing launched, nothing resolved through
   a broker), and any word other than exactly `LAUNCH` (or EOF) declines.

The managed-beta release gate (G3 sign-off + the full AT-15 lifecycle
traces) remains a separate owner/gate decision; this is the gated
capability surface, exactly like the policy stage's precedent.
