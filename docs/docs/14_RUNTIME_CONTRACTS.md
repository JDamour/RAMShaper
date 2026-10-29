# RAMShaper — Runtime, Policy and Recovery Contracts

**Revision:** 0.2 • **Date:** 2026-10-22 • **Status:** accepted for planning; all numeric defaults below are design choices, not measured results.

This document defines observable behavior for [02](02_PRODUCT_REQUIREMENTS.md), [03](03_SYSTEM_ARCHITECTURE.md) and [04](04_WINDOWS_API_IMPLEMENTATION.md). Changes to these contracts require corresponding acceptance and ADR updates.

## 1. Process and session identity

Windows identity is `(pid: u32, creation_filetime_100ns: u64)`, using the unmodified creation value from `GetProcessTimes` [S23]. Creation time is an OS identity marker; it is **not a monotonic timestamp**. Persist the owning user SID and session ID as authorization context, separately from the identity.

Open a handle with only the required rights, verify identity/ownership through that handle, and use that same handle for the read/change/verification sequence. A failed identity query makes the target ineligible. Never validate one PID and then reopen it for a write without checking again. A new PID/start pair requires new consent. Process names, paths and parent IDs are labels, not sufficient identity or authorization.

Wall-clock UTC timestamps describe records. A monotonic clock measures sample intervals and live TTLs. Do not serialize a runtime monotonic deadline for reuse after restart. Startup always reconciles outstanding actions before accepting new ones. Resume from sleep, unlock, or reconnect invalidates recommendations and triggers restoration/revalidation before new actions; do not assume timer behavior across suspend.

## 2. Metric envelope and units

Each metric carries `value`, `status`, `source`, `observed_at_utc`, `sample_sequence`, and `age_ms` computed in the current agent instance. Status is `ok`, `warming_up`, `access_denied`, `unsupported`, `exited`, `error`, or `stale`. A non-OK metric is not zero. A stale display may retain its last value with its timestamp, but policy input must ignore it. Include a machine-readable error code where available, with redacted human text.

| Field | Internal value / source | Display and interpretation |
|---|---|---|
| Physical total / available | Checked `u64` page count × page size, `GetPerformanceInfo` [S01] | MiB/GiB; available is not merely unused pages |
| Physical in use | Total minus available from the same successful snapshot | Derived; reject impossible available > total |
| Commit used / limit / headroom | Page counts × page size [S01] | System commit; not pagefile occupancy; zero/invalid limit => unavailable ratio |
| System cache | `SystemCache` pages × page size [S01] | Overlaps other memory classifications; never stack as a disjoint slice beside available |
| Process working set | `WorkingSetSize`, bytes [S27] | Resident pages; shared memory can be counted in multiple processes |
| Process private commit | `PrivateUsage`, bytes [S27] | Private committed memory, not private resident RAM |
| Process CPU | Delta of kernel + user time / monotonic interval / logical CPU count [S23] | Percent of total machine capacity; one fully busy core on N cores ≈ 100/N% |
| Process I/O read/write | Delta of `GetProcessIoCounters` byte counters [S28] | I/O B/s; do not label as physical disk throughput or paging |
| Low-memory event | Successful query of OS resource condition [S29] | Signaled / not signaled / unavailable; no latency claim |
| Hard faults / task latency | Dedicated future instrumentation | `Not measured` until implemented; total page-fault count is not hard faults |

First counter sample, counter decrease, changed identity, nonpositive interval, or logical CPU topology change yields `warming_up` and resets the baseline. No negative/NaN/infinite rates. JSON carries byte counts as integers, FILETIME identities as decimal strings to avoid consumer precision loss, and fractions/ratios only where defined.

## 3. Sampling, retention and backpressure

| Setting | Planning default | Constraint |
|---|---|---|
| Visible dashboard system sampling | 1 s | One in-flight collection; skip missed ticks, no catch-up burst |
| Hidden/headless observation | 5 s | No graph rendering or animation |
| Process inventory refresh | 5 s | Bounded work queue; at most 4,096 identities; report truncation |
| Display staleness | Age > 2 × configured interval | Explicit age and chart gaps |
| Active policy safety checks | 1 s, plus available focus events | Eligibility/focus older than 2 s => restore and block new apply |
| Apply telemetry freshness | At most 2 s | Collect fresh input at confirmation if background sampling is slower |
| In-memory history | 15 min, hard cap 8 MiB serialized metric payload | Evict oldest; separate from measured total agent memory |
| Persistent numerical history | Off by default | Optional aggregate-only history ≤30 days and 32 MiB; no raw per-process history |
| Diagnostic/action history | Terminal records ≤30 days, 16 MiB combined | Active/unresolved journal records are pinned; see below |
| Active actions / recovery records | 1 active action globally; at most 128 unresolved records | At cap, block new actions; never evict recovery intent |

Optional persistence must batch writes no more often than once per 60 s. Safety journal durability is an exception and occurs before each mutation. SQLite checkpointing, database/WAL file bounds and pruning must be measured. If space, quota, schema or integrity prevents a durable journal, continue observation and disable new actions. Ordinary telemetry must not crowd out restoration work.

Slow IPC subscribers receive the newest snapshot with a gap marker, not an unbounded backlog. Limit each subscriber to two pending snapshot frames. Action results remain discoverable by ID in the journal; they are not silently dropped as telemetry.

## 4. Pressure evidence and recommendations

Version `pressure_v1` is an observation classifier, not an intervention trigger or a diagnostic claim about a slowdown:

- `Unknown`: global physical/commit values or low-memory state missing, stale or invalid.
- `Elevated`: the low-memory condition is signaled in three consecutive valid observations, with at least two seconds between the first and last.
- `Normal`: five consecutive valid observations, spanning at least four seconds, have no low-memory condition. The UI says **No low-memory signal observed**; this does not establish absence of CPU/I/O contention.
- Retain the previous valid state while a transition is pending; expose the pending evidence. Any missing observation resets consecutive counts and makes the state Unknown. `Critical` is reserved until an additional validated rule exists; v1 never emits it.

Show commit headroom and memory trends separately; never convert used RAM into an invented pressure percentage or numeric confidence. Evidence quality is `complete`, `partial`, or `unavailable`, with the missing inputs listed. Future thresholds require versioned rules, preregistration and tests; all profiles initially share these defaults.

0.1 has no actionable policy recommendations. 0.2 permits a reviewed experiment only when Elevated evidence is fresh and the target satisfies every eligibility check. The UI explains that a CPU-priority change may slow background work and may have no memory benefit. No claim that the target caused the pressure is inferred from its memory rank.

## 5. Consent and eligibility

Mode and profile are independent: `ObserveOnly` is the mode at startup; `Balanced` is a settings preset. Selecting a profile, importing configuration, reconnecting UI, or enabling login startup never grants permission to write process settings.

The first policy is a **single manually confirmed Normal → Below Normal CPU priority class change**, with default TTL 120 s and allowed TTL 10–300 s [S24]. Other original classes are skipped; no priority boosting. Memory priority and EcoQoS remain research-only. There is no automatic repeat or persistent actuation grant in 0.2.

An approval binds to an agent-instance ID, action ID, exact process identity, user/session, action kind, original/desired setting, TTL, rule version and current exclusion/configuration revision. A pending preview expires after 30 s or any eligibility/configuration change. Replay returns the existing result, never extends TTL or creates another action. Stored app preferences may prefill a preview; executable names or imported rules cannot grant authority.

All of the following must hold at apply time: current user/session; fresh foreground observation with target not foreground; tested compatibility allowlist; verified noncritical/nonprotected classification; no user exclusion; original Normal priority; no outstanding action/conflict; available journal. Unknown ownership, classification, focus or API capability means deny. Audio, security, accessibility-critical, install/update, game/anti-cheat and RAMShaper processes are excluded. Detection coverage is not universal; the initial allowlist is limited to tested leaf workloads. Do not enroll new descendants or respawned processes. An observed new child invalidates the experiment and restores the parent; inherited child settings remain a compatibility risk to test before adding an app.

After completion, impose a 60 s per-identity cooldown and require a new confirmation. A failed apply, verification or restore disables that target's policy until the user explicitly retries after the issue is resolved.

## 6. Journal and action state machine

SQLite is required for the policy stage, optional/unneeded for the first CLI. The policy coordinator is the only writer. One action changes one property, avoiding partially applied bundles.

| State | Required transition and evidence |
|---|---|
| `Proposed` | Preview only; no OS write; expires or is denied without a recovery record |
| `Prepared` | Persist and durably commit identity, authorization context, original/desired values, consent, rule revision, TTL and action ID before the OS call |
| `Applied` | Write through the validated handle; read back; record the actual result and live deadline |
| `RestorePending` | Block new applies; revalidate identity and read the current property |
| `Restored` | Current property already equals original, or restoration write and read-back match original |
| `TargetGone` | Original process exited or identity no longer matches; do not write to replacement |
| `Conflict` | Current property differs from both original and RAMShaper's desired value; preserve the external change and require attention |
| `Failed` / `Uncertain` | Record error/unknown outcome; if a write may have occurred, retain recovery intent and reconcile |

If the OS write fails, read back when possible rather than assuming no effect. Crash after `Prepared` is indistinguishable from a write followed by a crash: startup uses current-value reconciliation, never reapplies the change. Crash after restore but before its result is committed is safe to reconcile again. Storage failure blocks new applies, not recovery: if original state is still readable from the durable record or validated in-memory action, attempt restoration and retain the unresolved durable intent if its result cannot be committed. Do not infer a missing original value from defaults or a corrupt record.

Restore only if the current value still equals the value written by RAMShaper; otherwise record an external conflict. These Win32 operations have no compare-and-swap transaction: a race with another writer remains possible. Minimize the interval using the same handle, verify afterward, and exclude known competing priority managers. Do not promise perfect ownership detection if another writer sets the same value.

Manual disable, consent revocation, foreground return, TTL expiry, stale safety input, configuration invalidation, session lock, suspend notification, error, clean agent exit and startup recovery initiate restoration. Stop accepting new apply requests immediately on disable. While the agent and OS are responsive, target verification within 2 s of a restore trigger; record missed deadlines, not fabricated success. Retry transient restore failures at 1 s, 5 s and 30 s, then retain `RestorePending` with manual retry. Access denial needs attention immediately. No busy retry loop.

An agent crash can leave a priority changed until recovery runs or the process exits. No independent watchdog is included in the MVP. Restart restores outstanding records before enabling any new actions and returns to Observe Only. UI closure does not stop an explicitly enabled resident agent; otherwise it requests orderly agent shutdown/restoration. Never say **all restored** while unresolved records remain.

## 7. IPC and configuration contract

The first CLI calls its adapter directly. The later UI communicates with a single non-elevated agent per user/logon session through a local named pipe. Set an explicit session-scoped DACL, reject remote clients, check peer ownership/session, and avoid default pipe security [S26]. ACLs do not authenticate human consent or isolate malicious software running as the same user. This is not a privileged boundary; no elevated helper is included.

Protocol v1 uses length-prefixed UTF-8 JSON, a maximum 1 MiB frame, and envelopes containing `protocol_version`, `request_id`, `agent_instance_id`, `command`, and `payload`. Reject oversized/malformed frames and incompatible versions before dispatch. Read-only commands: `GetCapabilities`, `GetSnapshot`, `SubscribeSnapshots`, `ListActions`, and (additive since the M2 UI slice, RS-011) `GetProcesses` — one fresh inventory scan per request, never cached on the collector cadence, with rates spanning the gap since the previous scan and a served-row cap noted explicitly so a response can never exceed the frame cap; agents without the command answer `Unsupported`, which clients render as an explained empty table, never as data. `GetConfig` is additive the same way (RS-012 UI portion): it serves the loaded configuration's inert summary — startup mode, active profile preset, user exclusions and an explicit inertness note — and never the whole config file; agents without the command answer `Unsupported`. Policy-stage commands: `PreviewAction`, `ConfirmAction`, `RestoreAction`, `DisableActions`. Managed launch is a separately gated capability: since RS-023 (2026-10-28) it ships as the CLI command `launch -- <PATH> [ARGS...]` (no agent/IPC surface yet), gated by a typed per-launch console consent — the strict word `LAUNCH` displayed with the exact command line, preflight notes and semantics statements; one consent buys at most one attempt (a consumed token), the image is launched exactly as named (explicit `lpApplicationName` — no PATH search, shell or association resolution) into a fresh advisory job, and removing monitoring drops the monitoring handles only: no detach, no kill, membership persists for the process lifetime, and the four semantics statements are served with every launch report. No raw arbitrary property setter or shell command RPC.

Responses carry `ok`, `data` or `error { code, message, retryable }`. Error codes include `VersionMismatch`, `Unsupported`, `AccessDenied`, `StaleSnapshot`, `IdentityMismatch`, `Ineligible`, `ConsentExpired`, `Conflict`, `JournalUnavailable`, `Busy` and `OsError`. An IPC timeout has an **unknown outcome**; query action/request status before retry. Durable action IDs make confirmations idempotent across lost replies. A reconnect does not preserve unconfirmed previews or enable policy mode.

TOML configuration has an integer schema version. Reject unknown fields, unsupported versions, out-of-range intervals/TTLs and malformed targets with a field-level error; retain the last valid configuration. Persist atomically. Migrations back up the original and validate before replacing it. Import always starts in Observe Only, cannot weaken mandatory exclusions, and cannot overwrite the action journal. Recovery proceeds independently of a bad profile file. An unreadable/newer journal schema blocks all new policy actions and remains untouched for compatible recovery.

## 8. CLI and local-data contract

First increment: `ramshaper-cli info [--json]`, `low-memory-watch [--json]`, `--version`, `--help`. Later `processes`, `policies --dry-run` and managed-launch commands appear only with their gated capabilities: `processes` (RS-007), `policies --dry-run` (RS-019, synthetic only; since 2026-10-28 with an optional `--trace <FILE>` closed JSON schema — `ramshaper.policy-trace.v1` — over user-authored observations, still no OS reads or writes, output still SYNTHETIC-marked, invalid files exit 2) and `launch` (RS-023, typed per-launch consent; monitoring removal included, never detachment) have all shipped in that form; `restore` (2026-10-29) is a thin wire client over the same RestoreAction/DisableActions verbs the UI uses — exactly one verb per invocation, the agent's explained refusals (Unsupported on a gateless agent, Conflict on an external change) printed and non-retryable; it adds no authority and the default pipe name derives from the logon session like the agent's. No mutating command in the first increment.

Text output states units and unavailable reasons. JSON emits one versioned object per snapshot (newline-delimited in watch mode), with UTC timestamp, OS/build metadata, agent-instance ID, sequence and metric envelopes. Stdout contains data only; diagnostics go to stderr. Exit codes: 0 complete success, 1 runtime failure/no usable global sample, 2 usage/configuration error, 3 partial snapshot/unavailable optional capability. A healthy user-cancelled watch returns 0; fatal collector errors return 1. Ctrl+C cancels waits and releases handles. JSON examples and fixture data must be marked synthetic.

Use a per-user local application-data directory with restricted access. Do not persist full command lines, window titles, or per-process timelines by default. Local identity records are pseudonymous, not anonymous. Diagnostics export is explicit, with fresh per-export aliases and no stable device ID, paths, SIDs, raw process identities or arguments by default. Recovery data never enters a default export.

Clear-history removes samples and terminal diagnostics; active/unresolved recovery intent is retained with explanation. **Erase all** first disables actions and attempts recovery; if unresolved, report partial deletion and retain the minimal journal. Uninstall follows the same contract and tells the user the retained recovery location. Journal pinning can exceed age retention but not the record cap: block new actions instead of silently deleting recovery data. No network upload or automatic update check in the MVP.
