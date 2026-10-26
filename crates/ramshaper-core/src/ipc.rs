//! IPC protocol v1: frames, envelopes and error codes (contract 14 §7).
//!
//! Wire format: a frame is a 4-byte little-endian payload length followed by
//! exactly that many bytes of UTF-8 JSON. The payload cap is 1 MiB; larger
//! frames are rejected before allocation. Every request envelope carries
//! `protocol_version`, `request_id`, `command` and an optional `payload`;
//! incompatible versions are rejected before dispatch. Responses carry
//! `ok`, `request_id`, `agent_instance_id` and either `data` or
//! `error { code, message, retryable }`.
//!
//! This module is pure (no OS calls): encode/decode, envelope validation
//! and the error-code vocabulary are unit-testable without a pipe.

use serde::{Deserialize, Serialize};

/// Protocol version implemented by this build.
pub const PROTOCOL_VERSION: u32 = 1;
/// Maximum frame payload in bytes (1 MiB, contract 14 §7).
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Maximum pending snapshot frames per subscriber (contract 14 §3).
pub const MAX_PENDING_SUBSCRIBER_FRAMES: usize = 2;

/// Error codes of protocol v1 (contract 14 §7). Serialized verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// The client's protocol version is not supported.
    VersionMismatch,
    /// The command (or its shape) is not supported by this build.
    Unsupported,
    /// The peer failed validation or the OS denied access.
    AccessDenied,
    /// The requested snapshot is too old to serve.
    StaleSnapshot,
    /// The referenced process identity no longer matches.
    IdentityMismatch,
    /// The target is not eligible for the requested action.
    Ineligible,
    /// The stored consent has expired or was revoked.
    ConsentExpired,
    /// Another writer owns the setting (policy stage).
    Conflict,
    /// The action journal is unavailable (policy stage).
    JournalUnavailable,
    /// The agent cannot accept the request right now.
    Busy,
    /// An OS error occurred; `message` carries the preserved code.
    OsError,
    /// A policy request argument was malformed or references nothing
    /// actionable (additive: policy stage).
    InvalidArgument,
    /// The outcome of a policy write is unknown; recovery will reconcile
    /// (additive: policy stage).
    Uncertain,
}

impl ErrorCode {
    /// Whether retrying the same request later could succeed.
    #[must_use]
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorCode::Busy | ErrorCode::OsError | ErrorCode::StaleSnapshot
        )
    }
}

/// Request envelope (one frame's JSON payload).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    /// Client's protocol version; anything but [`PROTOCOL_VERSION`] is
    /// rejected before dispatch.
    pub protocol_version: u32,
    /// Client-chosen correlation id, echoed in the response.
    pub request_id: String,
    /// Optional client agent-instance id (informational in v1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_instance_id: Option<String>,
    /// Command name (one of the read-only commands in v1).
    pub command: String,
    /// Command payload (shape checked at dispatch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Read-only commands of protocol v1. `GetProcesses` is additive (RS-011):
/// agents older than its introduction answer `Unsupported`, which clients
/// treat as an empty table with an explanation, never as data.
/// `GetConfig` is additive the same way (RS-012 UI portion): the inert
/// profile selection, mode and user exclusions of the serving agent's
/// configuration.
pub const COMMAND_GET_CAPABILITIES: &str = "GetCapabilities";
pub const COMMAND_GET_SNAPSHOT: &str = "GetSnapshot";
pub const COMMAND_SUBSCRIBE_SNAPSHOTS: &str = "SubscribeSnapshots";
pub const COMMAND_LIST_ACTIONS: &str = "ListActions";
pub const COMMAND_GET_PROCESSES: &str = "GetProcesses";
pub const COMMAND_GET_CONFIG: &str = "GetConfig";

/// Policy-stage commands (contract 14 §7), additive to protocol v1: agents
/// built before policy (or with the policy gate absent) answer
/// `Unsupported`, which clients render as an explained refusal, never as
/// silence. Actuation stays behind the agent's policy gate — disabled at
/// startup; enabling is the G3 consent flow, not a protocol command.
pub const COMMAND_PREVIEW_ACTION: &str = "PreviewAction";
pub const COMMAND_CONFIRM_ACTION: &str = "ConfirmAction";
pub const COMMAND_RESTORE_ACTION: &str = "RestoreAction";
pub const COMMAND_DISABLE_ACTIONS: &str = "DisableActions";

/// `PreviewAction` payload. The client names the target and the durable
/// action ID; the agent re-resolves the identity, takes fresh observations
/// through its own adapters and applies every eligibility gate itself — a
/// client request is a *candidate*, never authority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewActionRequest {
    /// Caller-chosen durable action ID (replays return the same preview).
    pub action_id: String,
    /// Target PID.
    pub pid: u32,
    /// Exact creation FILETIME (100-ns units) as a decimal string — full
    /// 64-bit precision does not survive JSON numbers.
    pub creation_filetime: String,
    /// Target image name (label only; the agent re-reads it never — this
    /// is the compatibility label the allowlist and exclusions match).
    pub image_name: String,
    /// Requested TTL (ms); `None` uses the policy default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
}

/// `ConfirmAction` payload: the durable ID only. The exact process
/// identity, original value and TTL were bound at preview time on the
/// agent side; confirmation replays that binding against fresh
/// observations (contract 14 §5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfirmActionRequest {
    /// The previewed action's durable ID.
    pub action_id: String,
}

/// `RestoreAction` payload: restore one action by durable ID (manual
/// trigger; the record supplies identity, original and desired values).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestoreActionRequest {
    /// The applied action's durable ID.
    pub action_id: String,
}

/// `DisableActions` payload: stop accepting new applies and restore
/// everything outstanding. Empty in v1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DisableActionsRequest {}

/// Error body of a failed response.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErrorBody {
    /// Machine-readable error code.
    pub code: ErrorCode,
    /// Redacted human-readable explanation.
    pub message: String,
    /// Whether a retry could succeed.
    pub retryable: bool,
    /// Optional structured detail (e.g. the explain-all eligibility report
    /// of an `Ineligible` refusal). Additive: absent in older responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

/// One response, serialized as one frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    /// Whether the request succeeded.
    pub ok: bool,
    /// Echoed correlation id (empty when the request could not be parsed).
    pub request_id: String,
    /// Serving agent instance id.
    pub agent_instance_id: String,
    /// Payload on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// Error on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Response {
    /// Successful response.
    #[must_use]
    pub fn ok(request_id: &str, agent_instance_id: &str, data: serde_json::Value) -> Self {
        Self {
            ok: true,
            request_id: request_id.to_string(),
            agent_instance_id: agent_instance_id.to_string(),
            data: Some(data),
            error: None,
        }
    }

    /// Error response (retryability derives from the code).
    #[must_use]
    pub fn error(
        request_id: &str,
        agent_instance_id: &str,
        code: ErrorCode,
        message: String,
    ) -> Self {
        Self {
            ok: false,
            request_id: request_id.to_string(),
            agent_instance_id: agent_instance_id.to_string(),
            data: None,
            error: Some(ErrorBody {
                code,
                message,
                retryable: code.retryable(),
                detail: None,
            }),
        }
    }

    /// A failed response carrying structured detail (e.g. the explain-all
    /// eligibility report of an `Ineligible` refusal).
    pub fn error_with_detail(
        request_id: &str,
        agent_instance_id: &str,
        code: ErrorCode,
        message: String,
        detail: Option<serde_json::Value>,
    ) -> Self {
        Self {
            ok: false,
            request_id: request_id.to_string(),
            agent_instance_id: agent_instance_id.to_string(),
            data: None,
            error: Some(ErrorBody {
                code,
                message,
                retryable: code.retryable(),
                detail,
            }),
        }
    }
}

/// A snapshot delivered to a subscriber, with an explicit gap marker when
/// frames were dropped for a slow consumer (contract 14 §3).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SnapshotDelivery {
    /// The newest snapshot payload.
    pub snapshot: serde_json::Value,
    /// Frames dropped since the previous delivery (0 when none).
    pub gap_dropped: u32,
}

/// Frame encoding/decoding errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The length prefix exceeds the frame cap.
    #[error("frame length {0} exceeds the {1}-byte cap")]
    Oversized(u32, usize),
    /// The prefix or payload is not a well-formed frame.
    #[error("malformed frame")]
    Malformed,
}

/// Encode one frame: 4-byte little-endian length + payload.
#[must_use]
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// Parse one frame (length + payload) from the head of `buffer`.
/// Returns the payload slice bounds; the caller consumes them.
pub fn decode_frame_prefix(prefix: [u8; 4]) -> Result<usize, FrameError> {
    let len = u32::from_le_bytes(prefix) as usize;
    if len == 0 || len > MAX_FRAME_BYTES {
        let capped = u32::from_le_bytes(prefix);
        if capped as usize > MAX_FRAME_BYTES {
            return Err(FrameError::Oversized(capped, MAX_FRAME_BYTES));
        }
        return Err(FrameError::Malformed); // zero-length payload
    }
    Ok(len)
}

/// Serialize a response to its frame **payload** bytes (the transport adds
/// the length prefix). Enforces the frame cap.
pub fn response_frame(response: &Response) -> Result<Vec<u8>, FrameError> {
    let json = serde_json::to_vec(response).map_err(|_| FrameError::Malformed)?;
    if json.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized(json.len() as u32, MAX_FRAME_BYTES));
    }
    Ok(json)
}

/// Parse a request envelope from a frame payload.
pub fn parse_request(payload: &[u8]) -> Result<RequestEnvelope, FrameError> {
    serde_json::from_slice(payload).map_err(|_| FrameError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> RequestEnvelope {
        RequestEnvelope {
            protocol_version: 1,
            request_id: "r-7".to_string(),
            agent_instance_id: Some("cli-1-2".to_string()),
            command: "GetSnapshot".to_string(),
            payload: None,
        }
    }

    #[test]
    fn frame_roundtrip() {
        let payload = br#"{"protocol_version":1}"#;
        let frame = encode_frame(payload);
        assert_eq!(&frame[..4], &(payload.len() as u32).to_le_bytes()[..]);
        let len = decode_frame_prefix([frame[0], frame[1], frame[2], frame[3]]).expect("len");
        assert_eq!(len, payload.len());
        assert_eq!(&frame[4..4 + len], payload.as_slice());
    }

    #[test]
    fn oversized_prefix_is_rejected_before_allocation() {
        let huge = (MAX_FRAME_BYTES + 1) as u32;
        assert_eq!(
            decode_frame_prefix(huge.to_le_bytes()),
            Err(FrameError::Oversized(huge, MAX_FRAME_BYTES))
        );
    }

    #[test]
    fn zero_length_prefix_is_malformed() {
        assert_eq!(
            decode_frame_prefix([0, 0, 0, 0]),
            Err(FrameError::Malformed)
        );
    }

    #[test]
    fn request_envelope_parses_and_rejects_shapes() {
        let json = serde_json::to_vec(&sample_request()).expect("ser");
        let envelope = parse_request(&json).expect("parse");
        assert_eq!(envelope.command, "GetSnapshot");
        assert_eq!(envelope.protocol_version, 1);
        // Missing required fields -> malformed.
        assert!(parse_request(b"{\"command\":\"GetSnapshot\"}").is_err());
        assert!(parse_request(b"not json").is_err());
        // Optional fields may be absent.
        let minimal = br#"{"protocol_version":1,"request_id":"x","command":"ListActions"}"#;
        let envelope = parse_request(minimal).expect("minimal");
        assert_eq!(envelope.agent_instance_id, None);
        assert_eq!(envelope.payload, None);
    }

    #[test]
    fn response_shape_matches_contract() {
        let ok = Response::ok("r-1", "agent-1", serde_json::json!({"a": 1}));
        let value = serde_json::to_value(&ok).expect("ser");
        assert_eq!(value["ok"], true);
        assert_eq!(value["request_id"], "r-1");
        assert_eq!(value["agent_instance_id"], "agent-1");
        assert_eq!(value["data"]["a"], 1);
        assert!(value.get("error").is_none());

        let err = Response::error(
            "r-2",
            "agent-1",
            ErrorCode::VersionMismatch,
            "expected 1".into(),
        );
        let value = serde_json::to_value(&err).expect("ser");
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "VersionMismatch");
        assert_eq!(value["error"]["retryable"], false);
        assert!(value.get("data").is_none());
    }

    #[test]
    fn retryability_follows_the_contract() {
        assert!(ErrorCode::Busy.retryable());
        assert!(ErrorCode::OsError.retryable());
        assert!(ErrorCode::StaleSnapshot.retryable());
        assert!(!ErrorCode::VersionMismatch.retryable());
        assert!(!ErrorCode::AccessDenied.retryable());
        assert!(!ErrorCode::Conflict.retryable());
    }

    #[test]
    fn snapshot_delivery_carries_gap_marker() {
        let delivery = SnapshotDelivery {
            snapshot: serde_json::json!({}),
            gap_dropped: 3,
        };
        let value = serde_json::to_value(&delivery).expect("ser");
        assert_eq!(value["gap_dropped"], 3);
    }

    #[test]
    fn response_frame_respects_cap() {
        let ok = Response::ok("r", "agent", serde_json::json!({"x": "y"}));
        assert!(response_frame(&ok).is_ok());
    }

    #[test]
    fn response_frame_rejects_payload_over_cap() {
        // A response whose payload exceeds the 1 MiB cap must be refused by
        // the codec, never truncated onto the wire or left to the transport.
        let huge = "x".repeat(MAX_FRAME_BYTES + 1);
        let bloated = Response::ok("r", "agent", serde_json::json!({ "blob": huge }));
        match response_frame(&bloated) {
            Err(FrameError::Oversized(size, cap)) => {
                assert!(size as usize > MAX_FRAME_BYTES);
                assert_eq!(cap, MAX_FRAME_BYTES);
            }
            other => panic!("expected Oversized, got {:?}", other.map(|_| ())),
        }
    }
}
