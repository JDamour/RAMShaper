# AT-17 adversarial IPC coverage map (read-only IPC portion), 2026-10-24

Completes the same-session adversarial portion of AT-17 on top of the RS-030
record. Contract: "Given wrong user/session, remote client, oversized/malformed
request, replay or version mismatch, when connecting/dispatching, then request
is rejected with bounded memory and no OS mutation."

| AT-17 threat | Evidence | Status |
|---|---|---|
| wrong user/session | `validate_peer` unit verdicts (session mismatch / foreign SID / unresolvable SID → Rejected); user-only DACL (SDDL 64 bytes from serving user's SID) enforced at pipe creation; accept-loop queries client session + PID and resolves the peer SID **before** any dispatch | Passed (unit + wiring); live cross-session peer **Not run locally** (single interactive session, standard user) — the documented same-user limitation |
| remote client | named pipes are local-machine objects; no remote endpoint exists to create | Not run (structurally impossible locally); covered by the same session/SID validation |
| oversized request | `oversized_frame_prefix_is_rejected_without_allocation` (e2e: raw 256 MiB prefix → rejected **before allocation**, connection terminated); `oversized_prefix_is_rejected_before_allocation` (unit); **new:** `response_frame_rejects_payload_over_cap` (unit — response side refuses > 1 MiB with `Oversized`) | Passed |
| malformed request | `malformed_frame_gets_one_error_then_close` (e2e: garbage JSON → one `Unsupported` frame, then close); `request_envelope_parses_and_rejects_shapes` (unit); **new:** `malformed_framing_prefix_ends_the_connection_silently` (e2e: zero-length prefix → no response frame possible, silent close) | Passed |
| version mismatch | `version_mismatch_is_rejected_before_dispatch` (dispatch unit) + e2e on `GetSnapshot`; `get_processes_rejects_wrong_version_before_any_scan` (e2e — proves a wrong-version request never triggers an on-demand scan, i.e. no resource consumption) | Passed |
| replay | **new:** `replayed_request_is_served_identically_on_the_same_connection` (e2e — identical frame twice → two identical successful responses, same correlation, identical data). v1 has **no replay cache**: `request_id` is correlation, not anti-replay. Accepted because every v1 command is read-only and idempotent; a protocol v2 must not inherit this if policy-stage (state-mutating) commands arrive | Passed (characterized + documented) |
| bounded memory | 1 MiB frame cap enforced in `decode_frame_prefix` **before** buffer allocation (`read_frame`); subscriber queue bounded at 2 frames with gap markers (`subscriber_queue_is_bounded_with_gap_markers`); `max_instances` bounds concurrent pipe instances | Passed |
| no OS mutation | every v1 command is read-only; policy-shaped command (`SetPriorityClass`) → `Unsupported`; `ListActions` honestly empty; workspace-wide actuation code does not exist in this build | Passed (structural) |

Run: `cargo test -p ramshaper-agent --test ipc --locked` → **13 passed**;
`cargo test -p ramshaper-core --locked` → **110 passed** (incl. new response-cap
test); fmt clean; clippy `-p ramshaper-agent -p ramshaper-core --all-targets
-D warnings` clean. Raw output: `test-run.txt`.
