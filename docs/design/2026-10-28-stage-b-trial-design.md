# Consent-enabled Stage B trial — design memo

**Date:** 2026-10-28 • **Status:** proposal for the owner decision; converts the
fourth G3 blocker from a claim into an executable plan.

The G3 gate assessment (doc 16, 2026-10-27) records blocker 3 as: *"Enable the
consent flow for a declared synthetic workload and run the preregistered Stage B
trial → the A/B report and the first allowlist entries (owner; the machinery —
consent, gate, workload, prereg, analysis — is complete and tested; the treatment
injection point is `run_workload`'s closure)."*

That sentence is **overstated**. The consent/gate/journal/workload/analysis
machinery is indeed complete and tested — but an actually runnable consented
trial is blocked by four concrete gaps that no code path covers today. This memo
names them, gives options, and recommends one per gap. None of them is large;
all of them touch the safety surface, so each needs an ADR rather than a silent
fix.

## What the trial must produce

- RS-024 Stage B (doc 07): a real policy comparison run — the preregistered
  A/B between observe and treatment on a declared workload — whose report and
  first allowlist entries close the allowlist ticket and the synthetic-A/B
  report clause of G3.
- The evidence chain already exists end to end once a run happens:
  `workload-ab` → `analyze-ab` (AT-20) under executable preregistration
  (0.3.38) → rerun bundle (0.3.46). Negative/inconclusive outcomes are
  first-class (doc 08 §7).

## Gap 1 — no operator-facing allowlist path

`consent::open_session_gate` hardcodes the gate's allowlist to empty
(`Allowlist::new(ELIGIBILITY_RULES_VERSION, &[])`, consent.rs) and the agent has
no flag or config section that can populate it. Correct by design — *user config
can never populate the allowlist; only tested evidence can* — but the first
trial target must be in the allowlist **before** any tested evidence exists.
Chicken-and-egg, unresolved.

**Options.**
(a) `serve --allowlist-image <NAME>` (repeatable): the operator names the trial
subject at agent startup, session-scoped, never persisted, never carried by
config import (the closed schema has no such field — verified). Every other
eligibility check still applies; the allowlist remains a compatibility label,
never authority.
(b) A trial-only gate builder in the bench process — rejected: it would move
OS-writing authority out of the agent (see Gap 4).

**Recommendation:** (a). The operator already consents every session and every
apply with a typed word; naming the declared subject on the same command line
adds no new trust boundary. Needs **ADR-045**.

## Gap 2 — classification is Unknown-deny for any non-self target

`classify_target` (ramshaper-windows) implements self-identification only: the
agent's own PID/image names are `protected_ramshaper`; everything else is
`Unknown` — which **denies** — "until RS-024 Stage B adds tested coverage". The
live policy-IPC tests work around this with the constructor-only
`ClassificationOverride` closure (ramshaper-agent/src/policy.rs); production
passes `None`. So a first trial subject is ineligible on classification alone,
independently of the allowlist.

**Options.**
(a) The same `--allowlist-image` flag also feeds the gate's classification
override: for images named on the command line ONLY, the override returns a new
`SafetyClassification::OperatorDeclaredTrial` variant (or
`VerifiedNonCritical` with the journal record noting the declaration). It is an
honest operator assertion — "this is my declared experiment subject" — not a
tested signal; the variant's wording should say so.
(b) Treat allowlist membership as sufficient classification — rejected: it
collapses two independent checks into one and weakens the matrix's semantics.

**Recommendation:** (a), with a new vocabulary variant carrying the
operator-declared semantics. Needs the same **ADR-045** (the two decisions are
one surface).

## Gap 3 — the subject process does not exist

`workload-ab` runs the workload **in the bench process**, and the self-
identification list deliberately protects RAMShaper's own binaries — so an
in-process trial subject is ineligible **by construction**, and measuring
inside the treated process is also poor methodology (the driver and the subject
share a process). A real trial needs a separate subject.

**Options.**
(a) A dedicated subject binary (new `[[bin]]`, e.g. `ramshaper-subject.exe`,
reusing the same bounded-CPU task-latency workload from `ramshaper-bench::workload`),
which measures its own per-task latencies and emits them per run; the bench
orchestrates subjects per run and aggregates. Its image name stays **out of**
`self_image_names` — it is a workload simulator, the *subject* of the
experiment, not RAMShaper machinery acting on itself — with that reasoning
recorded in ADR-045.
(b) Use an arbitrary existing image (ping, powershell) — rejected: unbounded,
unmeasurable task latencies, no declared workload.

**Recommendation:** (a). Machine-doable follow-up (no ADR beyond the naming
note).

## Gap 4 — no bench→gate treatment path

`run_workload`'s `treatment: &dyn Fn()` fires pre-task inside the measurement
loop and is observe-only today; nothing connects the bench to the agent's
consented gate. The right seam is **not** that closure.

**Recommendation:** orchestration-level treatment. The bench driver wraps each
treated run: `PreviewAction` → `ConfirmAction` on the agent pipe (protocol v1
commands already exist and are live-tested), runs the subject, then
`RestoreAction`, and records the action id in the run metadata. The **agent
stays the only OS-writing process** (single-writer principle preserved); the
workload crate is untouched (`treatment` stays `none`); the consent model is
exactly the shipped one (typed `ENABLE` on the agent console; a profile/import
is not consent; nothing is grantable over IPC). Machine-doable follow-up.

## Runbook sketch (once gaps 1–4 close)

1. Owner writes the preregistration file (workload spec, blocks, thresholds,
   seed) — the executable plan 0.3.38 enforces.
2. Owner starts the agent: `ramshaper-agent serve --policy-journal <J>
   --allowlist-image ramshaper-subject.exe --config <trial config>` and types
   `ENABLE` (session consent; the one act no flag or file can perform).
3. Operator (or owner) runs the bench trial mode: counterbalanced blocks, the
   agent applies/restores per treated run.
4. `analyze-ab --preregistration <file>` → A/B report with labels; negative
   outcomes recorded as-is.
5. `scripts/rerun-bundle.ps1` → second-operator bundle (AT-22 tooling, ready).
6. Report + first allowlist entries recorded in doc 16 → G3 blocker 3 closes.

## Follow-up tickets (in dependency order)

1. **ADR-045** (owner decision): operator-declared trial subjects — the
   `--allowlist-image` serve flag + the `OperatorDeclaredTrial` classification
   semantics + the subject image's placement outside self-identification.
2. Subject binary (machine-doable after 1; reuses `ramshaper-bench::workload`).
3. Bench trial-orchestrator mode driving the policy IPC (machine-doable after
   2).
4. Agent serve flags (machine-doable after 1).
5. Runbook doc + a rehearsal with the observe-only path (machine-doable;
   consent still owner-typed).

What remains owner after all of the above: the ADR-045 pick, typing `ENABLE`,
the declared workload parameters, and running the trial.
