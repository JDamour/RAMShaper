//! Pure request dispatch for protocol v1 (RS-030).
//!
//! Maps a parsed [`RequestEnvelope`] to a [`Response`] using an injectable
//! context — no OS calls, fully unit-testable. Version checking happens
//! before command dispatch (contract 14 §7). `SubscribeSnapshots`
//! acknowledges and lets the connection layer switch to streaming.

use ramshaper_core::ipc::{
    ErrorCode, RequestEnvelope, Response, COMMAND_GET_CAPABILITIES, COMMAND_GET_PROCESSES,
    COMMAND_GET_SNAPSHOT, COMMAND_LIST_ACTIONS, COMMAND_SUBSCRIBE_SNAPSHOTS, PROTOCOL_VERSION,
};
use serde_json::json;

/// Everything dispatch needs to answer a request.
pub struct DispatchContext<'a> {
    /// Serving agent-instance id.
    pub instance_id: &'a str,
    /// Capability payload (built once at agent start).
    pub capabilities: serde_json::Value,
    /// Newest collected snapshot (JSON value, its sequence and monotonic
    /// collection time in ms).
    pub latest: Option<&'a LatestSnapshot>,
    /// A snapshot older than this (2 × cadence) is stale, not served.
    pub max_age_ms: u64,
    /// Current monotonic position (age baseline).
    pub now_monotonic_ms: u64,
    /// Fresh `GetProcesses` material for this one request (RS-011): the
    /// connection handler collects it on demand just before dispatch.
    pub processes: ProcessesContext<'a>,
}

/// The process-inventory material a dispatch sees for `GetProcesses`.
pub enum ProcessesContext<'a> {
    /// This agent does not serve process inventories (no provider wired):
    /// dispatch answers `Unsupported` and clients show an explained empty
    /// table, never fabricated data.
    NotSupported,
    /// A fresh inventory was collected for this request.
    Ready(&'a serde_json::Value),
    /// The on-demand collection failed; the reason is preserved verbatim.
    Failed(&'a str),
}

/// The newest snapshot available to dispatch.
#[derive(Debug, Clone)]
pub struct LatestSnapshot {
    /// Serialized snapshot payload.
    pub snapshot: serde_json::Value,
    /// Snapshot sequence number.
    pub sequence: u64,
    /// Monotonic collection time (ms) within this agent instance.
    pub monotonic_ms: u64,
}

/// Dispatch one request. Malformed payloads on payload-less commands are
/// ignored (the command defines the shape); unknown commands and versions
/// are rejected here.
pub fn dispatch(envelope: &RequestEnvelope, context: &DispatchContext<'_>) -> Response {
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Response::error(
            &envelope.request_id,
            context.instance_id,
            ErrorCode::VersionMismatch,
            format!(
                "protocol version {} is not supported; this agent speaks {PROTOCOL_VERSION}",
                envelope.protocol_version
            ),
        );
    }
    match envelope.command.as_str() {
        COMMAND_GET_CAPABILITIES => Response::ok(
            &envelope.request_id,
            context.instance_id,
            context.capabilities.clone(),
        ),
        COMMAND_GET_SNAPSHOT => match context.latest {
            Some(latest) => {
                let age = context.now_monotonic_ms.saturating_sub(latest.monotonic_ms);
                if age > context.max_age_ms {
                    Response::error(
                        &envelope.request_id,
                        context.instance_id,
                        ErrorCode::StaleSnapshot,
                        format!(
                            "newest snapshot is {age} ms old (limit {} ms)",
                            context.max_age_ms
                        ),
                    )
                } else {
                    let mut payload = latest.snapshot.clone();
                    if let serde_json::Value::Object(map) = &mut payload {
                        map.insert("age_ms".to_string(), json!(age));
                        map.insert("sample_sequence".to_string(), json!(latest.sequence));
                    }
                    Response::ok(&envelope.request_id, context.instance_id, payload)
                }
            }
            None => Response::error(
                &envelope.request_id,
                context.instance_id,
                ErrorCode::StaleSnapshot,
                "no snapshot has been collected yet".to_string(),
            ),
        },
        COMMAND_SUBSCRIBE_SNAPSHOTS => Response::ok(
            &envelope.request_id,
            context.instance_id,
            json!({ "subscribed": true, "protocol_version": PROTOCOL_VERSION }),
        ),
        COMMAND_LIST_ACTIONS => Response::ok(
            &envelope.request_id,
            context.instance_id,
            json!({ "actions": [], "note": "no policy actions exist in this build (observe only)" }),
        ),
        COMMAND_GET_PROCESSES => match context.processes {
            ProcessesContext::Ready(inventory) => {
                Response::ok(&envelope.request_id, context.instance_id, inventory.clone())
            }
            ProcessesContext::Failed(reason) => Response::error(
                &envelope.request_id,
                context.instance_id,
                ErrorCode::OsError,
                reason.to_string(),
            ),
            ProcessesContext::NotSupported => Response::error(
                &envelope.request_id,
                context.instance_id,
                ErrorCode::Unsupported,
                "this agent does not serve process inventories".to_string(),
            ),
        },
        other => Response::error(
            &envelope.request_id,
            context.instance_id,
            ErrorCode::Unsupported,
            format!("unknown command '{other}'"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope(version: u32, command: &str) -> RequestEnvelope {
        RequestEnvelope {
            protocol_version: version,
            request_id: "r-1".to_string(),
            agent_instance_id: None,
            command: command.to_string(),
            payload: None,
        }
    }

    fn capabilities() -> serde_json::Value {
        json!({ "protocol_version": 1 })
    }

    #[test]
    fn version_mismatch_is_rejected_before_dispatch() {
        let ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: capabilities(),
            latest: None,
            max_age_ms: 2000,
            now_monotonic_ms: 0,
            processes: ProcessesContext::NotSupported,
        };
        let response = dispatch(&envelope(2, "GetSnapshot"), &ctx);
        assert!(!response.ok);
        let error = response.error.expect("error");
        assert_eq!(error.code, ErrorCode::VersionMismatch);
        assert!(!error.retryable);
    }

    #[test]
    fn capabilities_and_list_actions_succeed() {
        let ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: json!({ "protocol_version": 1, "capabilities": [] }),
            latest: None,
            max_age_ms: 2000,
            now_monotonic_ms: 0,
            processes: ProcessesContext::NotSupported,
        };
        let caps = dispatch(&envelope(1, "GetCapabilities"), &ctx);
        assert!(caps.ok);
        assert_eq!(caps.data.expect("data")["protocol_version"], 1);
        let actions = dispatch(&envelope(1, "ListActions"), &ctx);
        assert!(actions.ok);
        assert_eq!(
            actions.data.expect("data")["actions"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn fresh_snapshot_is_served_with_age() {
        let latest = LatestSnapshot {
            snapshot: json!({ "schema": "ramshaper.snapshot.v1" }),
            sequence: 9,
            monotonic_ms: 5_000,
        };
        let ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: capabilities(),
            latest: Some(&latest),
            max_age_ms: 2_000,
            now_monotonic_ms: 5_500,
            processes: ProcessesContext::NotSupported,
        };
        let response = dispatch(&envelope(1, "GetSnapshot"), &ctx);
        assert!(response.ok);
        let data = response.data.expect("data");
        assert_eq!(data["age_ms"], 500);
        assert_eq!(data["sample_sequence"], 9);
    }

    #[test]
    fn stale_or_missing_snapshot_is_an_error_never_old_data() {
        let latest = LatestSnapshot {
            snapshot: json!({ "old": true }),
            sequence: 1,
            monotonic_ms: 0,
        };
        let ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: capabilities(),
            latest: Some(&latest),
            max_age_ms: 2_000,
            now_monotonic_ms: 9_000,
            processes: ProcessesContext::NotSupported,
        };
        let response = dispatch(&envelope(1, "GetSnapshot"), &ctx);
        assert!(!response.ok);
        assert_eq!(
            response.error.expect("error").code,
            ErrorCode::StaleSnapshot
        );

        let none_ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: capabilities(),
            latest: None,
            max_age_ms: 2_000,
            now_monotonic_ms: 0,
            processes: ProcessesContext::NotSupported,
        };
        let response = dispatch(&envelope(1, "GetSnapshot"), &none_ctx);
        assert!(!response.ok);
        assert_eq!(
            response.error.expect("error").code,
            ErrorCode::StaleSnapshot
        );
    }

    #[test]
    fn subscribe_acknowledges_and_unknown_commands_are_unsupported() {
        let ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: capabilities(),
            latest: None,
            max_age_ms: 2000,
            now_monotonic_ms: 0,
            processes: ProcessesContext::NotSupported,
        };
        let sub = dispatch(&envelope(1, "SubscribeSnapshots"), &ctx);
        assert!(sub.ok);
        assert_eq!(sub.data.expect("data")["subscribed"], true);
        let bad = dispatch(&envelope(1, "SetPriorityClass"), &ctx);
        assert!(!bad.ok);
        let error = bad.error.expect("error");
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert!(!error.retryable);
    }

    #[test]
    fn response_correlates_request_ids() {
        let ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: capabilities(),
            latest: None,
            max_age_ms: 2000,
            now_monotonic_ms: 0,
            processes: ProcessesContext::NotSupported,
        };
        let response = dispatch(&envelope(1, "GetCapabilities"), &ctx);
        assert_eq!(response.request_id, "r-1");
        assert_eq!(response.agent_instance_id, "agent-1");
    }

    #[test]
    fn get_processes_serves_a_fresh_inventory_verbatim() {
        let inventory = json!({
            "schema": "ramshaper.processes.v1",
            "processes": [{ "identity": { "pid": 4242 } }],
        });
        let ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: capabilities(),
            latest: None,
            max_age_ms: 2000,
            now_monotonic_ms: 0,
            processes: ProcessesContext::Ready(&inventory),
        };
        let response = dispatch(&envelope(1, "GetProcesses"), &ctx);
        assert!(response.ok);
        let data = response.data.expect("data");
        assert_eq!(data["schema"], "ramshaper.processes.v1");
        assert_eq!(data["processes"][0]["identity"]["pid"], 4242);
    }

    #[test]
    fn get_processes_collection_failure_is_a_retryable_os_error() {
        let ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: capabilities(),
            latest: None,
            max_age_ms: 2000,
            now_monotonic_ms: 0,
            processes: ProcessesContext::Failed("scan failed: OS error 299"),
        };
        let response = dispatch(&envelope(1, "GetProcesses"), &ctx);
        assert!(!response.ok);
        let error = response.error.expect("error");
        assert_eq!(error.code, ErrorCode::OsError);
        assert!(error.retryable);
        assert!(error.message.contains("299"));
    }

    #[test]
    fn get_processes_without_a_provider_is_unsupported_never_empty() {
        let ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: capabilities(),
            latest: None,
            max_age_ms: 2000,
            now_monotonic_ms: 0,
            processes: ProcessesContext::NotSupported,
        };
        let response = dispatch(&envelope(1, "GetProcesses"), &ctx);
        assert!(!response.ok);
        let error = response.error.expect("error");
        assert_eq!(error.code, ErrorCode::Unsupported);
        // An unsupported command must look different from an empty table so
        // clients can explain the gap instead of showing fake "no processes".
        assert!(!error.retryable);
    }
}
