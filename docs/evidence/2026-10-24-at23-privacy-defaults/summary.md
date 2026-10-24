# AT-23 privacy/offline review — portions applicable to the read-only increment, 2026-10-24

Scope: AT-23's G2-applicable portions ("no hidden upload/update request occurs,
default exports redact identities") against the current offline, observe-only
build. The journal/uninstall-report portions belong to RS-031/G3 capabilities
that do not exist yet and are recorded as such.

## 1. No hidden upload / update request

- **Static dependency audit:** no network API in workspace code — zero uses of
  `std::net`, `TcpStream`, `UdpSocket`, `WinHTTP`, `WinINet`, `WSAStartup`,
  reqwest/ureq/http crates across `crates/**` (grep evidence in the record).
  In the Windows host dependency graph there are **no network-capable crates**;
  the only async-stack entry (`tokio 1.54.1`) resolves exclusively through
  `wasm-bindgen-futures → winit` — the Slint UI's **wasm32 build path**
  (`cargo tree -i tokio` on the host target: "nothing to print"; with
  `--target all` the chain is tokio ← wasm-bindgen-futures ← winit ←
  accesskit_winit ← i-slint-backend-winit ← slint ← ramshaper-ui). No socket
  code is compiled into any Windows binary.
- **Live socket observation:** the long-running observer processes hold no
  sockets — `Get-NetTCPConnection` / `Get-NetUDPEndpoint` by owning process:
  soak watch PID 41836 (1 Hz observer, running ~4.5 h at capture time):
  **tcp=0 udp=0**; session agent PID 8540: **tcp=0 udp=0**.
- **No automatic update check:** the contract (doc 14 §"No network upload or
  automatic update check in the MVP") is implemented by absence: no version/
  update endpoint exists anywhere in the codebase (covered by the static
  audit above).

## 2. Default exports carry no identities

Real exports captured in this directory (release-adjacent debug build,
`ramshaper-cli 0.1.0` — the `config` command postdates the frozen release
exe; the release rebuild stays deferred while the AT-06 soak holds it):

- `export-default.toml` — the complete file: `schema_version`, `mode =
  "observe_only"`, watch cadence/retention, empty `[agent]`,
  `cohort.declared = "none"`, `profiles.active = "balanced"`, empty
  `[exclusions]`. No machine name, user name, SID, path, instance id or
  timestamp appears.
- `export-populated.toml` — with user-supplied content (`agent.pipe_name =
  "my-custom-pipe"`, `exclusions.processes = ["vendor-tool.exe"]`): the export
  contains exactly the user's fields and nothing else. The two free-text
  fields in the closed schema are the only places user content can appear;
  the tool injects no identity material.

## 3. Uninstall story (current increment status)

No installer exists (dev increment: binaries live under `target/`), so the
uninstall surface is "delete the folder". Verified today: **no** ramshaper
scheduled task, **no** Windows service, **no** HKCU Run-key entry. The
contract's uninstall-report obligation (retained-recovery location disclosure)
binds when the journal (RS-031) and packaging (RS-027) exist — Not applicable
yet, recorded as such.

## Boundaries of this evidence

- The UI crate's window was not part of this capture (UI process network
  state belongs to the UI session's evidence; the Slint/winit stack is
  included in the static audit).
- Live socket observation is point-in-time; the static audit is the
  structural argument (no socket code exists to call), the live check is the
  behavioral corroboration.
