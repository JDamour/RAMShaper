# Stage B trial — runbook

**Date:** 2026-10-28 • **Status:** the executable procedure for the first
consent-enabled Stage B trial (RS-024 Stage B; the design memo's ticket 5
runbook half). Everything here is machinery-tested except the two places
marked **[ADR-045]** — those are the owner decision, and the rehearsal
(practice run with a real consenting gate) which needs its outcome.

The machinery this runbook drives, all shipped and tested: the subject
binary (0.3.53), the trial driver (0.3.54), executable preregistration
(0.3.38), the A/B analyzer with prereg enforcement (0.3.38/0.3.39), and
the rerun-bundle assembler (0.3.46).

## 0. Preconditions (each must be true before starting)

| # | Precondition | Status |
|---|---|---|
| P1 | ADR-045 decided (operator-declared trial subjects: the agent's `--allowlist-image` flag + the classification semantics for a declared subject) | **owner** |
| P2 | The subject is named to the agent per ADR-045 so eligibility passes | **[ADR-045]** |
| P3 | **Real Elevated memory-pressure evidence.** The gate honestly refuses previews without it (`pressure_not_elevated` — pinned by the driver's wire tests): the pressure classifier needs three consecutive elevated samples of real counters. On trial day the machine must be *genuinely* under memory pressure (the condition RAMShaper exists for) — e.g. a memory-resident workload holding commit near the limit. Do not run the trial on an idle machine and do not try to fake the counters; establish the real condition first and verify `ramshaper-cli info` reports Elevated (watch until the state holds). | **trial-day operational fact** |
| P4 | The agent's policy session consent: typed `ENABLE` on the agent console by the operator (never a flag, never IPC, never persisted) | **owner-typed at run time** |
| P5 | AT-06 soak not running (the soak holds the release exes; also the trial must not perturb its measurements) | check before step 2 |
| P6 | Declared workload parameters (events per run, blocks, thresholds, seed) chosen and written down BEFORE step 1 — doc 08: thresholds are frozen before data collection | **owner** |

## 1. Write the preregistration

`ramshaper-bench` refuses deviations (0.3.38), so the plan IS the contract:

```
ramshaper-bench workload-ab --preregistration trial.plan.json ...   # validate the plan against a pilot, or
```

Author `trial.plan.json` (closed schema — see
`docs/evidence/2026-10-26-rs024b-preregistration/` for a worked example):
workload declaration, events ≥ 100/run, warmup, task-iters, block count
(≥ 5 for a confirmatory label; fewer is labelled exploratory — fine for a
first trial, say so in the declaration), bootstrap resamples + seed, the
5%/2%/10% thresholds.

## 2. Start the consenting agent

```
ramshaper-agent serve --policy-journal <J> --config <trial config> [ADR-045 flags]
```

Then type `ENABLE` on that console (P4). Verify with a probe that the
gate is present, and `GetConfig` that mode is still observe_only.

**[ADR-045]** the exact flag(s) naming `ramshaper-subject.exe` as the
declared trial subject.

## 3. Run the trial

```
ramshaper-trial --pipe-name <agent pipe> --subject ramshaper-subject.exe ^
    --spec trial-spec.json --blocks <N> --seed <S> ^
    --preregistration ... [--out trial-blocks.json]
```

Behavior you will see, per block: the subject announces `SUBJECT_READY`,
the agent applies Below Normal to it (typed-consented, journaled), the
measured run executes, the agent restores while the subject still lives.
Any refusal fails the trial fast with no blocks file — that is by design;
fix the precondition, do not retry past a refusal.

## 4. Analyze under the frozen plan

```
ramshaper-bench analyze-ab --blocks trial-blocks.json --preregistration trial.plan.json [--json]
```

The analyzer builds its bootstrap and guardrails FROM the plan and refuses
plans that freeze other values. **Negative, inconclusive and exploratory
labels are outcomes, not failures** (doc 08 §7): the first trial's most
likely honest result on a synthetic workload is "no measured improvement" —
that is a real compatibility-data point for the allowlist decision, not a
reason to re-roll.

## 5. Assemble the rerun bundle

```
scripts/rerun-bundle.ps1 -Prereg trial.plan.json -Report <report.json> -Out <bundle dir>
```

Immutable once assembled; this is what a second operator reruns (AT-22
tooling).

## 6. Record

Doc 16 gets: the report label verbatim (positive or negative), the raw
counts, the journal's action records (the trial's applies/restores are
ordinary policy actions — `ListActions` shows them), and the first
allowlist-entry decision **[ADR-045 ratifies what a passing trial
implies]**.

## Rehearsal (before the real trial)

One practice pass of steps 2–4 with a small block count. It needs P1–P4
(a real consenting gate — the test-suite gates are not a substitute on the
shipped surface) and P3's real pressure, so it is scheduled **after**
ADR-045, alongside the real trial rather than before it. The machinery
itself is rehearsal-covered by the test suite (driver↔analyzer seam,
hold protocol, refusal paths — 0.3.54).
