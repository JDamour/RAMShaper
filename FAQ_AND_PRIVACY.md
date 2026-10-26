# RAMShaper — FAQ and Privacy

Answers below reflect the implemented build (documentation revision
0.3.36). Every claim links to its evidence in the
[implementation evidence record](docs/docs/16_IMPLEMENTATION_EVIDENCE.md).

## Privacy

### Does RAMShaper send anything anywhere?

**No.** The workspace compiles zero network capability into the Windows
host binaries — `cargo tree -i tokio` on the host target resolves to
nothing (the only tokio path in the dependency graph is behind the UI
crate's wasm32 rendering path, which never builds on Windows), and a
grep for network APIs (`TcpStream`, `UdpSocket`, sockets, HTTP clients)
across all shipped crates finds no use. Live checks confirm the running
agent and watchers hold **no TCP or UDP sockets** (AT-23, re-verified in
the RS-026 policy-stage boundary review). There is no telemetry, no
update check, no crash reporter.

### What does RAMShaper store on my machine?

Three kinds of files, all local:

- **Configuration** (TOML, only where you put it): watch cadence, the
  inert profile preset, your process exclusions. `config export`
  produces identity-free files — schema markers and exactly the fields
  you supplied (AT-23 evidence captures real exports; no username, SID,
  hostname or hardware identifiers appear).
- **The action journal** (only when you start the agent with
  `--policy-journal <PATH>`): a SQLite database of policy action records
  — action IDs, target process names and PIDs, original and desired
  priority values, timestamps and recovery state. It exists so a restart
  can *undo* whatever was applied. The file is **restricted to your user
  account** at creation (a single explicit grant-all ACE; inherited
  permissions are stripped — verified in the RS-026 review). No journal,
  no policy stage: a plain `ramshaper-agent serve` never creates one.
- **Session history in memory only**: process/boundary history lives in
  RAM and is printed on exit; it is not written to disk.

Uninstall is deleting the folder: no scheduled task, no service, no Run
key, no shell extension (AT-23 registry/task audit). If you used a
policy journal, delete that file too — nothing recreates it.

### Does it read my documents, browsing, or anything personal?

No. The observer reads OS memory counters (`GetPerformanceInfo` plus the
`CreateMemoryResourceNotification` low-memory condition), the process
table (names, PIDs, parent PIDs, per-process memory/CPU/I/O counters)
and — in a policy session — process priority classes and owner/session
context of candidate targets. Process
names are used as *labels* only; identity is the PID plus the exact
creation time. No file contents, no browsing data, no keystrokes.

## Consent and actions

### Can RAMShaper change my processes without asking?

**No.** Every agent starts Observe Only. Even when you opt into the
policy stage (`--policy-journal`), the gate is present but *disabled*:
startup reconciliation (undoing anything left over from a previous
session) runs, but no new action can be applied until you type the exact
word `ENABLE` on that agent's console. The consent is per session — it
is never persisted, never derivable from a config file or profile, and
never grantable over the pipe. Closing stdin or typing anything else
leaves the session read-only.

A profile preset or an imported configuration is *not* consent. Neither
is having consented yesterday.

### What can the first policy action actually do?

One thing: a manually confirmed `Normal → Below Normal` CPU priority
change on one process, for a bounded time (default 120 s, max 300 s),
offered only while a genuine low-memory condition is measured. It is
restored automatically on TTL expiry, foreground return, session lock,
config change, child-process appearance, manual disable, agent exit —
and on agent restart, from the journal, without re-applying anything.
External changes made by other tools are detected and *preserved*, never
overwritten.

### Why can't I enable it for my favorite app?

The compatibility allowlist **ships empty on purpose**: no workload has
passed the Stage B compatibility testing yet (does the app tolerate a
temporary priority change without misbehaving? do its children inherit
the setting safely?). Until a workload is tested and evidence recorded,
*every* target is ineligible by design — and any unknown observation
(owner, foreground state, classification, capability) denies the action
rather than guessing.

## Behavior

### Will this make my PC faster?

RAMShaper cannot create RAM and does not promise performance gains. It
measures memory pressure honestly (`pressure_v1` says "Unknown" when the
evidence is missing — never a made-up percentage), shows which processes
use memory and CPU, and — in a consented session — offers one
low-priority experiment under sustained low-memory pressure. Whether it
helps *your* workload is exactly what the benchmark harness (RS-024)
exists to measure; negative results are recorded, not hidden.

### Why does it refuse to touch system processes?

Protected categories (audio paths, security software, accessibility
tools, installers, games/anti-cheat, RAMShaper itself) and every
unknown-classified target are excluded by the eligibility matrix. A
process whose owner/session, foreground state or priority class cannot
be *read* is ineligible — unknown always denies. This is the core safety
property, pinned by tests before any actuation code existed.

### Which Windows versions are supported?

Only what was tested: Windows 11, 64-bit, on build 26200.9711, running
as a standard user (no administrator rights required or used). Other
builds and platforms are unverified; the Linux/macOS roadmap is
explicitly deferred with no parity promise.

## Reporting problems

See [CONTRIBUTING.md](CONTRIBUTING.md) and the issue templates. Useful
bug reports include the exact command, the full raw output (the tools
print typed error codes — `access_denied`, `exited_87`, `win32_<code>` —
instead of failing silently), your Windows build (`winver`), and whether
an agent with a policy journal was running. Never attach memory dumps or
process listings from sensitive machines without redacting first — and
remember the tools are offline, so a report is *you* sending it, not
RAMShaper.
