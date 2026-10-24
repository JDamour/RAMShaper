# RAMShaper — Security, Privacy and Safety

**Revision:** 0.2 • **Date:** 2026-10-22 • **Status:** required product constraints, not a completed security assessment.

## Safety invariants

- Observe Only at startup. Profile selection, import, login startup and reconnect are not consent.
- No policy-driven kill, suspension, restart, cache/working-set trim, hard quota, High/Realtime boost, code injection or kernel driver.
- No writes to user documents or process memory contents. OS setting changes are narrowly scoped and explicitly confirmed.
- No automatic attachment of running processes to a Job Object; managed launch is separately gated.
- No kill-on-job-close, hard/time/process-count enforcement in MVP jobs.
- No claim of immediate rollback after the agent crashes; no all-restored status with unresolved recovery.
- Missing identity, ownership, focus, safety classification or journal capability means no new action.

## Trust model

The CLI, UI and agent run as the current user. The MVP has no elevated helper or Windows system service. One agent belongs to one user/logon session; it cannot act for other sessions or users. Access denial is an expected result with an explanation, not a prompt to run the whole app as administrator.

| Asset / failure | Required control | Residual limit |
|---|---|---|
| Wrong process after PID reuse | Native creation identity and same-handle validation | Unreadable identity prevents action |
| Unsafe target selection | Tested compatibility allowlist, foreground/session/owner checks, hard exclusions | No universal security/audio/anti-cheat detection claim |
| Local command spoofing | Explicit pipe DACL, session/peer validation, bounded typed commands [S26] | Same-user malware is not isolated by a SID ACL |
| Conflicting setting owner | Read current value before restore, verify afterward | OS lacks an atomic ownership transaction |
| Agent crash | Durable Prepared record and reconciliation before new actions | Setting may persist until restart or target exit |
| Corrupt/full storage | Reject new actions, retain recovery, observation stays usable | Recovery may require compatible version/manual help |
| Crafted paths/arguments | Explicit executable path, tested encoding, no shell | Executable itself remains user-selected untrusted software |
| Job lifecycle | New advisory jobs; creation-time spike and interruption tests | Membership is not reversible; group monitoring may be lost on crash |
| Release tampering | Lockfile, dependency review, signing/provenance decision | No signing or reproducibility claim until demonstrated |

No API request grants arbitrary process execution or privileged access. Treat executable labels, errors, exports and imported configuration as untrusted text.

## Eligibility and consent

Never target OS-critical/protected processes, security products, other users/sessions, audio workloads, accessibility-critical tools, game/anti-cheat, trusted installs/updates or RAMShaper itself. Users can add exclusions but cannot weaken mandatory safeguards through imports or advanced settings.

The first policy alpha supports a tested leaf-workload allowlist and one manually confirmed CPU-priority action. Approval is bound to current identity, setting, TTL, config revision and agent instance. It does not transfer to another executable with the same name, a child, a respawn or a reconnect. All exact expiry/cooldown behavior is in [14](14_RUNTIME_CONTRACTS.md).

## Recovery and data lifecycle

Persist original state and durable intent before the setting write. The policy coordinator serializes apply/restore/disable. Reconcile after every uncertain outcome and startup. Restore the recorded original value only when the current value is still RAMShaper's value; record external conflicts without knowingly overwriting them.

On disable, stop new applies immediately and attempt restoration. On access denial, show Needs attention; bounded retries must not become a tight loop. On PID mismatch/exit, do not touch a replacement process. An invalid profile does not prevent journal recovery; an unsupported/corrupt journal blocks new actions and is preserved.

Active and unresolved recovery records are exempt from age pruning and clear-history. Erase-all and uninstall first disable actions and attempt recovery. If restoration is unresolved, explain partial deletion and retain the minimal recovery journal/location. Never erase the only record needed to restore a live process. Record caps block new actions rather than evicting recovery data.

## Privacy contract

Transient local display may include executable names and numerical process metrics. Do not collect/store contents, URLs, window titles, clipboard, keystrokes, browsing history, screenshots or full command lines by default. Launch arguments are used for explicit launch, not retained in diagnostic history.

Default numerical history is memory-only. Optional aggregate persistence, bounded diagnostic history and recovery records follow [14](14_RUNTIME_CONTRACTS.md). PID/start/user identifiers and executable paths are identifying or pseudonymous data, never called anonymous.

Exports are explicit and default to redacted aliases scoped to that export, with paths, SIDs, raw identities, arguments and stable device IDs omitted. Recovery records do not enter standard exports. No telemetry, automatic update checks, crash upload, fleet reporting or network connection is part of the MVP.

## Security and recovery evidence

Run AT-03, AT-08–AT-18 and AT-23 at the relevant release slice. Include same-name masquerading, cross-session peers, malformed/oversized requests, replay/lost replies, configuration downgrade, journal migration failure, disk-full and all crash boundaries. Test child inheritance and simultaneous changes by another tool. Verify file/network behavior and recovery on uninstall.

No data-loss, protected-target or unexpected-termination failure can be waived by a good latency result. A failure disables the affected capability until resolved.

## Distribution and incident handling

Choose project/dependency licensing and review assets before public contributions/distribution; these decisions remain open. Declare exact supported Windows builds and legacy security limitations rather than implying RAMShaper supplies operating-system security updates.

For a reported slowdown/crash: disable actions, attempt restoration, offer a redacted local evidence export, identify action ID/OS error and reproduce in an isolated environment. A temporal correlation is not automatic causal diagnosis. Suspend the implicated policy from the compatibility allowlist, document findings and add a regression case. Do not repeatedly experiment on the user's important work.