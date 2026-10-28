# ramshaper-trial — Stage B trial driver, 2026-10-28

Design-memo follow-up ticket 3 (docs/design/2026-10-28-stage-b-trial-design.md).
The orchestrator for counterbalanced A/B runs of the `ramshaper-subject`
workload, with the treated runs wrapped in the agent's shipped consent
machinery **over the wire** — the agent remains the only OS-writing process.

## What was built

New crate `crates/ramshaper-trial` (driver split into `lib.rs` + thin
`main.rs`):

- Per treated run: spawn the subject in `--hold-until-go` mode → wait for
  `SUBJECT_READY` → `PreviewAction` (identity resolved fresh through the OS;
  creation FILETIME as the decimal-string wire form) → `ConfirmAction` →
  release with `GO` → collect the run record → `RestoreAction` while the
  subject is still alive (`--exit-delay-ms`), so the journal records
  `Restored`, not `TargetGone`. Baseline runs issue NO wire calls.
- Seeded per-block order coin (local SplitMix64, the bench's recorded-seed
  convention) → counterbalanced AB/BA; run ids `{prefix}b{block}-{A|B}`;
  action ids `trial-{run_id}` (deterministic, replay-friendly).
- Output: `analyze-ab`-compatible blocks JSON (`blocks_to_wire` — the
  `analysis::Block` input shape, untagged plain-number latencies) + human
  summary. **Fail-fast everywhere**: a refusal at any step stops the trial,
  no blocks file is ever written for a half-executed experiment, and an
  abandoned held subject exits 1 without running untreated.
- The [`AgentRpc`] seam: production is the named-pipe client (the UI's
  one-fresh-connection-per-call pattern, with the agent's own pipe-name
  normalization); tests script the seam.

## The gate decision this was tested under (recorded honestly)

The happy path is tested at the library boundary with a scripted RPC —
NOT against the real wire gate, because the gate honestly refuses
previews without fresh Elevated memory-pressure evidence
(`pressure_not_elevated`), this machine is not pressurized, and
**fabricating memory pressure during the AT-06 soak is off-limits**. The
apply/restore mechanics behind the scripted answers are proven live in
the agent crate's own tests (which inject the pressure level through the
in-process gate API). The wire refusal path IS tested against a real
gateless production agent over a real pipe.

## Live evidence (`live-refusal.txt`, release driver + release subject)

1. The real driver vs a real gateless agent: baseline run completes
   observe-only (150 events), the treated run's preview gets the typed
   `Unsupported: this agent was started without the policy gate`, the
   driver stops with exit 1, **no blocks file exists**, and the abandoned
   held subject prints its own honest `stdin closed before the go signal;
   nothing ran`.
2. The hold protocol: `SUBJECT_READY` → `GO` → complete run record.
3. (The earlier `1/1b/1c` entries in the file document a REAL bug the
   live runs caught and the fix's trail: the driver initially passed the
   raw pipe name to `client_connect` — the agent normalizes on both of
   its sides and the driver must too. FakeRpc tests could never catch
   this; only the wire run did. Fixed + covered.)

## Gates

fmt/clippy (`-D warnings`) clean; workspace **442 passed / 0 failed**
serially (+6 trial tests: order-coin determinism, usage rules, the
FakeRpc happy path with the exact three-verb sequence pinned, the
refused-preview fail-fast, the gateless wire refusal, the hold protocol).

## Remaining for a real trial (unchanged, owner-gated)

ADR-045 (the operator-facing `--allowlist-image` +
operator-declared-classification flags) and real Elevated memory-pressure
evidence at trial time — plus the owner's typed `ENABLE`.
