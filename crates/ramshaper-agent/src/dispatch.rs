//! Pure request dispatch for protocol v1 (RS-030).
//!
//! Maps a parsed [`RequestEnvelope`] to a [`Response`] using an injectable
//! context — no OS calls, fully unit-testable. Version checking happens
//! before command dispatch (contract 14 §7). `SubscribeSnapshots`
//! acknowledges and lets the connection layer switch to streaming.

use crate::policy::{PolicyGate, ResponseData};
use ramshaper_core::clock::WallClock;
use ramshaper_core::ipc::{
    ErrorCode, RequestEnvelope, Response, COMMAND_CONFIRM_ACTION, COMMAND_DISABLE_ACTIONS,
    COMMAND_GET_CAPABILITIES, COMMAND_GET_CONFIG, COMMAND_GET_PROCESSES, COMMAND_GET_SNAPSHOT,
    COMMAND_LIST_ACTIONS, COMMAND_PREVIEW_ACTION, COMMAND_RESTORE_ACTION,
    COMMAND_SUBSCRIBE_SNAPSHOTS, PROTOCOL_VERSION,
};
use serde::{Deserialize, Serialize};
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
    /// The serving agent's configuration summary (RS-012 UI portion): the
    /// inert mode/profile/exclusions material `GetConfig` serves. Built once
    /// from the loaded configuration at agent start.
    pub config: ConfigSummary,
    /// The policy gate when the agent was started with one (`None` =
    /// policy-stage commands answer `Unsupported`, exactly like older
    /// agents). Startup is always disabled inside the gate.
    pub policy: Option<&'a PolicyGate>,
}

/// The configuration material `GetConfig` serves (RS-012 UI portion): the
/// startup mode, the inert profile selection and the user exclusions. The
/// profile is a label — selecting or displaying it never grants permission
/// to write process settings (contract 14 §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSummary {
    /// Startup mode; only `observe_only` exists in this slice.
    pub mode: String,
    /// Active profile preset name (one of `ramshaper_core::config::PROFILES`).
    pub profile: String,
    /// User exclusion entries, stored verbatim.
    pub exclusions: Vec<String>,
}

impl ConfigSummary {
    /// Build the summary from a validated configuration.
    #[must_use]
    pub fn from_config(config: &ramshaper_core::config::CoreConfig) -> Self {
        Self {
            mode: config.mode.clone(),
            profile: config.profiles.active.clone(),
            exclusions: config.exclusions.processes.clone(),
        }
    }
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
        COMMAND_LIST_ACTIONS => match context.policy {
            // A journal read failure is a typed error, never an empty list
            // (contract 14 §7's JournalUnavailable exists for exactly this:
            // "no records" and "could not read" are different outcomes).
            Some(gate) => match gate.list_actions(ACTIONS_HISTORY_LIMIT) {
                Ok(data) => Response::ok(&envelope.request_id, context.instance_id, data),
                Err(error) => Response::error(
                    &envelope.request_id,
                    context.instance_id,
                    ErrorCode::JournalUnavailable,
                    format!("the action journal could not be read: {error}"),
                ),
            },
            None => Response::ok(
                &envelope.request_id,
                context.instance_id,
                json!({ "actions": [], "note": "no policy actions exist in this build (observe only)" }),
            ),
        },
        COMMAND_GET_CONFIG => Response::ok(
            &envelope.request_id,
            context.instance_id,
            json!({
                "mode": context.config.mode,
                "profile": context.config.profile,
                "exclusions": context.config.exclusions,
                "note": "profiles and exclusions are inert in this build; they never enable actions",
            }),
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
        COMMAND_PREVIEW_ACTION
        | COMMAND_CONFIRM_ACTION
        | COMMAND_RESTORE_ACTION
        | COMMAND_DISABLE_ACTIONS => match context.policy {
            None => Response::error(
                &envelope.request_id,
                context.instance_id,
                ErrorCode::Unsupported,
                "this agent was started without the policy gate".to_string(),
            ),
            Some(gate) => {
                let outcome = match envelope.command.as_str() {
                    COMMAND_PREVIEW_ACTION => {
                        match envelope
                            .payload
                            .as_ref()
                            .cloned()
                            .map(
                                serde_json::from_value::<ramshaper_core::ipc::PreviewActionRequest>,
                            )
                            .transpose()
                        {
                            Ok(Some(request)) => {
                                let pressure = pressure_level_of(context.latest);
                                let now = ramshaper_core::SystemClock.unix_epoch_ms();
                                gate.preview(&request, pressure, now)
                            }
                            Ok(None) | Err(_) => ResponseData::error(
                                ErrorCode::InvalidArgument,
                                "PreviewAction requires a PreviewActionRequest payload",
                            ),
                        }
                    }
                    COMMAND_CONFIRM_ACTION => {
                        match envelope
                            .payload
                            .as_ref()
                            .cloned()
                            .map(
                                serde_json::from_value::<ramshaper_core::ipc::ConfirmActionRequest>,
                            )
                            .transpose()
                        {
                            Ok(Some(request)) => {
                                let now = ramshaper_core::SystemClock.unix_epoch_ms();
                                gate.confirm(&request, now)
                            }
                            Ok(None) | Err(_) => ResponseData::error(
                                ErrorCode::InvalidArgument,
                                "ConfirmAction requires a ConfirmActionRequest payload",
                            ),
                        }
                    }
                    COMMAND_RESTORE_ACTION => {
                        match envelope
                            .payload
                            .as_ref()
                            .cloned()
                            .map(
                                serde_json::from_value::<ramshaper_core::ipc::RestoreActionRequest>,
                            )
                            .transpose()
                        {
                            Ok(Some(request)) => {
                                let now = ramshaper_core::SystemClock.unix_epoch_ms();
                                gate.restore(&request, now)
                            }
                            Ok(None) | Err(_) => ResponseData::error(
                                ErrorCode::InvalidArgument,
                                "RestoreAction requires a RestoreActionRequest payload",
                            ),
                        }
                    }
                    _ => {
                        match envelope
                            .payload
                            .as_ref()
                            .cloned().map(serde_json::from_value::<ramshaper_core::ipc::DisableActionsRequest>)
                            .transpose()
                        {
                            Ok(Some(request)) => {
                                let now = ramshaper_core::SystemClock.unix_epoch_ms();
                                gate.disable(&request, now)
                            }
                            Ok(None) | Err(_) => ResponseData::error(
                                ErrorCode::InvalidArgument,
                                "DisableActions requires a DisableActionsRequest payload",
                            ),
                        }
                    }
                };
                outcome.into_response(envelope, context.instance_id)
            }
        },
        other => Response::error(
            &envelope.request_id,
            context.instance_id,
            ErrorCode::Unsupported,
            format!("unknown command '{other}'"),
        ),
    }
}

/// History rows served by `ListActions` beyond the unresolved set
/// (bounded: a response can never approach the 1 MiB frame cap).
pub const ACTIONS_HISTORY_LIMIT: usize = 50;

/// Classify the newest snapshot's pressure for the policy gate: missing or
/// malformed evidence is `Unknown` (never guessed) — the preview then
/// answers "pressure not elevated".
fn pressure_level_of(latest: Option<&LatestSnapshot>) -> ramshaper_core::pressure::PressureLevel {
    use ramshaper_core::pressure::PressureLevel;
    let Some(latest) = latest else {
        return PressureLevel::Unknown;
    };
    match latest
        .snapshot
        .get("pressure")
        .and_then(|p| p.get("level"))
        .and_then(|level| level.as_str())
    {
        Some("elevated") => PressureLevel::Elevated,
        Some("normal") => PressureLevel::Normal,
        _ => PressureLevel::Unknown,
    }
}

#[cfg(test)]
mod policy_dispatch_tests {
    use super::*;
    use ramshaper_core::ipc::{
        RequestEnvelope, COMMAND_CONFIRM_ACTION, COMMAND_DISABLE_ACTIONS, COMMAND_PREVIEW_ACTION,
        COMMAND_RESTORE_ACTION,
    };

    fn envelope(seq: u32, command: &str) -> RequestEnvelope {
        RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: format!("r{seq}"),
            agent_instance_id: None,
            command: command.to_string(),
            payload: None,
        }
    }

    fn ctx() -> DispatchContext<'static> {
        DispatchContext {
            instance_id: "agent-1",
            capabilities: json!({}),
            latest: None,
            max_age_ms: 2000,
            now_monotonic_ms: 0,
            processes: ProcessesContext::NotSupported,
            config: ConfigSummary {
                mode: "observe_only".to_string(),
                profile: "balanced".to_string(),
                exclusions: vec![],
            },
            policy: None,
        }
    }

    #[test]
    fn policy_commands_answer_unsupported_without_a_gate() {
        for command in [
            COMMAND_PREVIEW_ACTION,
            COMMAND_CONFIRM_ACTION,
            COMMAND_RESTORE_ACTION,
            COMMAND_DISABLE_ACTIONS,
        ] {
            let response = dispatch(&envelope(1, command), &ctx());
            assert!(!response.ok, "{command} must not succeed without a gate");
            let error = response.error.expect("error");
            assert_eq!(error.code, ErrorCode::Unsupported, "{command}");
            assert!(error.message.contains("policy gate"));
        }
    }

    #[test]
    fn list_actions_serves_journal_records_when_the_gate_exists() {
        use crate::policy::PolicyGate;
        use ramshaper_core::eligibility::{
            Allowlist, SafetyClassification, ELIGIBILITY_RULES_VERSION,
        };
        use ramshaper_core::ipc::COMMAND_LIST_ACTIONS;
        use ramshaper_journal::JournalStore;

        let store = JournalStore::open_in_memory().expect("journal");
        let gate = PolicyGate::open_with_classification(
            store,
            Allowlist::new(ELIGIBILITY_RULES_VERSION, &[]),
            Vec::new(),
            Box::new(|_, _| SafetyClassification::VerifiedNonCritical),
        );
        // Seed one Applied record through the gate's own store.
        use ramshaper_core::journal::{
            ActionIntent, ActionState, Event, Property, ACTION_KIND_CPU_PRIORITY,
        };
        let intent = ActionIntent {
            action_id: "act-1".to_string(),
            agent_instance_id: "agent-1".to_string(),
            pid: 4242,
            creation_filetime: 133_000_000_000_000_000,
            target_name: "leaf.exe".to_string(),
            property: Property {
                kind: ACTION_KIND_CPU_PRIORITY,
                original: Some(0x20),
                desired: 0x4000,
            },
            consent_id: "c".to_string(),
            rule_version: "policy_v1".to_string(),
            ttl_ms: 120_000,
            created_at_unix_ms: 1_000,
        };
        {
            // journal_clone is pub; seed through the concrete store.
            let journal = gate.journal_clone();
            journal.record_prepared(&intent).expect("prepare");
            journal.record_original("act-1", 0x20).expect("pin");
            journal
                .transition(
                    "act-1",
                    ActionState::Prepared,
                    &Event::ApplyVerified {
                        deadline_unix_ms: 2_000,
                    },
                    1_000,
                    serde_json::json!({}),
                )
                .expect("apply");
        }

        let mut ctx = ctx();
        ctx.policy = Some(&gate);
        let response = dispatch(&envelope(1, COMMAND_LIST_ACTIONS), &ctx);
        assert!(response.ok, "ListActions with a gate must succeed");
        let data = response.data.expect("data");
        let actions = data["actions"].as_array().expect("actions array");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["action_id"], "act-1");
        assert_eq!(actions[0]["state"], "Applied");
        assert_eq!(actions[0]["original"], 0x20);
        assert_eq!(actions[0]["desired"], 0x4000);
        assert_eq!(actions[0]["unresolved"], true, "Applied is outstanding");
        assert_eq!(data["unresolved"], 1, "one outstanding record");
    }

    #[test]
    fn list_actions_journal_read_failure_is_a_typed_error_not_an_empty_list() {
        // Contract-14 audit: "the journal could not be read" must reach the
        // client as JournalUnavailable, never as a healthy empty action
        // list. Fault injection through the journal store's test seam;
        // clones share the armed state.
        use crate::policy::PolicyGate;
        use ramshaper_core::eligibility::{
            Allowlist, SafetyClassification, ELIGIBILITY_RULES_VERSION,
        };
        use ramshaper_core::ipc::COMMAND_LIST_ACTIONS;
        use ramshaper_journal::JournalStore;

        let store = JournalStore::open_in_memory().expect("journal");
        let arming_handle = store.clone();
        let gate = PolicyGate::open_with_classification(
            store,
            Allowlist::new(ELIGIBILITY_RULES_VERSION, &[]),
            Vec::new(),
            Box::new(|_, _| SafetyClassification::VerifiedNonCritical),
        );
        arming_handle.set_read_fault(Some("storage offline"));

        let mut ctx = ctx();
        ctx.policy = Some(&gate);
        let response = dispatch(&envelope(1, COMMAND_LIST_ACTIONS), &ctx);
        assert!(!response.ok, "a failed read must not report success");
        let error = response.error.expect("typed error");
        assert_eq!(error.code, ErrorCode::JournalUnavailable);
        assert!(
            error.message.contains("could not be read"),
            "the failure is explained: {}",
            error.message
        );
    }

    #[test]
    fn still_unknown_commands_stay_unsupported() {
        let response = dispatch(&envelope(2, "SetPriorityClass"), &ctx());
        assert!(!response.ok);
        assert_eq!(response.error.expect("error").code, ErrorCode::Unsupported);
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
            config: config_summary(),
            policy: None,
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
            config: config_summary(),
            policy: None,
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
            config: config_summary(),
            policy: None,
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
            config: config_summary(),
            policy: None,
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
            config: config_summary(),
            policy: None,
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
            config: config_summary(),
            policy: None,
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
            config: config_summary(),
            policy: None,
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
            config: config_summary(),
            policy: None,
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
            config: config_summary(),
            policy: None,
        };
        let response = dispatch(&envelope(1, "GetProcesses"), &ctx);
        assert!(!response.ok);
        let error = response.error.expect("error");
        assert_eq!(error.code, ErrorCode::OsError);
        assert!(error.retryable);
        assert!(error.message.contains("299"));
    }

    fn config_summary() -> ConfigSummary {
        ConfigSummary {
            mode: "observe_only".to_string(),
            profile: "balanced".to_string(),
            exclusions: vec!["obs64.exe".to_string()],
        }
    }

    #[test]
    fn get_config_serves_the_inert_summary() {
        let ctx = DispatchContext {
            instance_id: "agent-1",
            capabilities: capabilities(),
            latest: None,
            max_age_ms: 2000,
            now_monotonic_ms: 0,
            processes: ProcessesContext::NotSupported,
            config: ConfigSummary {
                mode: "observe_only".to_string(),
                profile: "low_ram".to_string(),
                exclusions: vec![],
            },
            policy: None,
        };
        let response = dispatch(&envelope(1, "GetConfig"), &ctx);
        assert!(response.ok);
        let data = response.data.expect("data");
        assert_eq!(data["mode"], "observe_only");
        assert_eq!(data["profile"], "low_ram");
        assert_eq!(data["exclusions"], json!([]));
        assert!(data["note"].as_str().expect("note").contains("inert"));
    }

    #[test]
    fn get_config_summary_builds_from_a_config() {
        let config = ramshaper_core::config::CoreConfig::parse(
            "schema_version = 1
[profiles]
active = \"focus\"
[exclusions]
processes = [\"a.exe\"]
",
        )
        .expect("valid");
        let summary = ConfigSummary::from_config(&config);
        assert_eq!(summary.mode, "observe_only");
        assert_eq!(summary.profile, "focus");
        assert_eq!(summary.exclusions, ["a.exe"]);
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
            config: config_summary(),
            policy: None,
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
