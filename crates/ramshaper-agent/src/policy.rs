//! The agent's policy gate (RS-016 completion, contract 14 §5–§7).
//!
//! Owns the policy machinery — the pure coordinator, the restore engine,
//! the SQLite journal, the real actuator and the eligibility observations —
//! behind one lock, and serves the four policy-stage IPC commands
//! (`PreviewAction`, `ConfirmAction`, `RestoreAction`, `DisableActions`).
//!
//! Safety shape:
//! - the gate is **absent** in agents started without it (`serve` takes an
//!   `Option`), and disabled inside until explicitly enabled — startup is
//!   always Observe Only; enabling is the G3 consent flow, never a
//!   protocol command;
//! - **startup reconciliation runs before the pipe accepts**: applied
//!   records restore through the engine, and crash-window records
//!   (`Prepared`/`Uncertain`) reconcile from the current value — never a
//!   re-apply (contract 14 §6);
//! - every observation the eligibility gate consumes is taken fresh
//!   through the R-03 adapters at request time; the client's request is a
//!   candidate, never authority;
//! - the compatibility allowlist is an injected evidence input (empty in
//!   production — nothing has passed RS-024 Stage B testing), and the
//!   classification override is the seam where the tested signal matrix
//!   will plug in; production classification is self-identification only.

use std::collections::HashMap;
use std::sync::Mutex;

use ramshaper_core::coordinator::{
    ConfirmOutcome, Coordinator, PreviewRequest, PreviewVerdict, RestoreEngine, RestoreOutcome,
    RestoreTrigger,
};
use ramshaper_core::eligibility::Allowlist;
use ramshaper_core::identity::ProcessIdentity;
use ramshaper_core::ipc::{
    ConfirmActionRequest, DisableActionsRequest, ErrorCode, PreviewActionRequest, RequestEnvelope,
    Response, RestoreActionRequest,
};
use ramshaper_core::journal::ActionState;
use ramshaper_core::policy::PolicyRules;
use ramshaper_core::policy::{TargetObservation, BELOW_NORMAL_PRIORITY_CLASS};
use ramshaper_core::pressure::PressureLevel;
use ramshaper_journal::JournalStore;
use ramshaper_windows::eligibility as adapters;
use ramshaper_windows::priority::WindowsPriorityActuator;

/// Classification override seam: `None` in production (self-identification
/// only); tests and the future RS-024 Stage B tested matrix inject here.
pub type ClassificationOverride = Box<dyn Fn(u32, &str) -> SafetyClassification + Send + Sync>;

use ramshaper_core::eligibility::SafetyClassification;

/// Everything policy, behind one lock. The classification override lives
/// outside the lock (immutable after construction) so target observation
/// never re-enters it.
pub struct PolicyGate {
    classification: Option<ClassificationOverride>,
    inner: Mutex<PolicyInner>,
}

struct PolicyInner {
    coordinator: Coordinator,
    engine: RestoreEngine,
    journal: JournalStore,
    actuator: WindowsPriorityActuator,
    allowlist: Allowlist,
    user_exclusions: Vec<String>,
    /// Previewed bindings: action_id -> (identity, image name). The
    /// confirmation replays this binding against fresh observations.
    previewed: HashMap<String, (ProcessIdentity, String)>,
    enabled: bool,
}

impl PolicyGate {
    /// Open a gate over a journal file. The allowlist is evidence (RS-024
    /// Stage B), never user config; exclusions come from the loaded
    /// configuration and can only reduce eligibility. The gate starts
    /// **disabled** — startup is Observe Only.
    ///
    /// # Errors
    /// [`ramshaper_journal::JournalError`] when the journal cannot be
    /// opened (an unusable journal blocks policy, not the read-only agent).
    pub fn open(
        journal_path: &std::path::Path,
        allowlist: Allowlist,
        user_exclusions: Vec<String>,
    ) -> Result<Self, ramshaper_journal::JournalError> {
        let journal = JournalStore::open(journal_path)?;
        Ok(Self::from_parts(journal, allowlist, user_exclusions, None))
    }

    /// The same gate with an injected classification matrix (the RS-024
    /// Stage B seam; tests inject, production passes `None`).
    pub fn open_with_classification(
        journal: JournalStore,
        allowlist: Allowlist,
        user_exclusions: Vec<String>,
        classification: ClassificationOverride,
    ) -> Self {
        Self::from_parts(journal, allowlist, user_exclusions, Some(classification))
    }

    fn from_parts(
        journal: JournalStore,
        allowlist: Allowlist,
        user_exclusions: Vec<String>,
        classification: Option<ClassificationOverride>,
    ) -> Self {
        Self {
            classification,
            inner: Mutex::new(PolicyInner {
                coordinator: Coordinator::new(PolicyRules::default_experiment()),
                engine: RestoreEngine::default(),
                journal,
                actuator: WindowsPriorityActuator,
                allowlist,
                user_exclusions,
                previewed: HashMap::new(),
                enabled: false,
            }),
        }
    }

    /// Enable the policy session (the G3 consent flow calls this on the
    /// gate the agent was started with; startup is always disabled).
    pub fn enable(&self) {
        let mut inner = self.inner.lock().expect("policy lock");
        inner.coordinator.enable();
        inner.enabled = true;
    }

    /// Whether the gate accepts policy requests right now.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.inner.lock().expect("policy lock").enabled
    }

    /// Read-only `ListActions` material: unresolved records first (the
    /// pending/conflict set the UI must not miss), then the most recent
    /// history window, deduplicated by action ID. Bounded so a response
    /// can never approach the 1 MiB frame cap.
    #[must_use]
    pub fn list_actions(&self, history_limit: usize) -> serde_json::Value {
        let inner = self.inner.lock().expect("policy lock");
        let unresolved = inner.journal.unresolved().unwrap_or_default();
        let unresolved_ids: Vec<String> = unresolved
            .iter()
            .map(|r| r.intent.action_id.clone())
            .collect();
        let history = inner.journal.recent(history_limit).unwrap_or_default();
        let mut rows: Vec<serde_json::Value> =
            unresolved.iter().map(|r| action_row(r, true)).collect();
        for record in &history {
            if !unresolved_ids.contains(&record.intent.action_id) {
                rows.push(action_row(record, false));
            }
        }
        serde_json::json!({
            "actions": rows,
            "unresolved": unresolved.len(),
            "note": "records from the action journal; unresolved records need attention or a due restore",
        })
    }

    /// A clone of the gate's journal store for the restore-direction loop
    /// (`policy_loop`): one SQLite database, two handles.
    #[must_use]
    pub fn journal_clone(&self) -> JournalStore {
        self.inner.lock().expect("policy lock").journal.clone()
    }

    /// Fresh observations of one candidate target through the R-03
    /// adapters. Every unknown is carried verbatim — the gate decides.
    fn observe_target(
        &self,
        pid: u32,
        creation_filetime: u64,
        image_name: &str,
    ) -> TargetObservation {
        let identity = ProcessIdentity::new(pid, creation_filetime);
        let classification = match &self.classification {
            Some(matrix) => matrix(pid, image_name),
            None => adapters::classify_target(
                pid,
                image_name,
                std::process::id(),
                &[
                    "ramshaper-agent.exe",
                    "ramshaper-cli.exe",
                    "ramshaper-ui.exe",
                ],
            ),
        };
        TargetObservation {
            identity_check: ramshaper_windows::identity::verify_identity(&identity),
            owner_session: adapters::observe_owner_session(pid),
            focus: adapters::observe_focus(pid),
            classification,
            priority_class: adapters::observe_priority_class(pid),
            children: adapters::observe_children(pid),
            capability: adapters::probe_priority_capability(pid),
        }
    }

    /// `PreviewAction`: evaluate a candidate against fresh observations,
    /// the live pressure level (from the newest snapshot; stale or missing
    /// means unknown) and every gate. The identity binding is remembered
    /// for the confirmation.
    pub fn preview(
        &self,
        request: &PreviewActionRequest,
        pressure: PressureLevel,
        now_unix_ms: u64,
    ) -> ResponseData {
        let creation = match request.creation_filetime.parse::<u64>() {
            Ok(value) => value,
            Err(_) => {
                return ResponseData::error(
                    ErrorCode::InvalidArgument,
                    "creation_filetime must be a decimal string of the 100-ns FILETIME",
                )
            }
        };
        {
            let inner = self.inner.lock().expect("policy lock");
            if !inner.enabled {
                return ResponseData::error(
                    ErrorCode::Busy,
                    "policy is disabled for this session; enabling is an explicit operator action",
                );
            }
        }
        let identity = ProcessIdentity::new(request.pid, creation);
        let target = self.observe_target(request.pid, creation, &request.image_name);
        let mut inner = self.inner.lock().expect("policy lock");
        let PolicyInner {
            coordinator,
            journal,
            allowlist,
            user_exclusions,
            previewed,
            ..
        } = &mut *inner;
        let verdict = coordinator.preview(
            &*journal,
            PreviewRequest {
                identity: &identity,
                image_name: &request.image_name,
                target,
                allowlist,
                user_exclusions,
                pressure,
                action_id: &request.action_id,
                now_unix_ms,
                ttl_ms: request.ttl_ms,
                desired: BELOW_NORMAL_PRIORITY_CLASS,
            },
        );
        match verdict {
            PreviewVerdict::Offered(pending) => {
                previewed.insert(
                    request.action_id.clone(),
                    (identity, request.image_name.clone()),
                );
                ResponseData::ok(serde_json::json!({
                    "verdict": "offered",
                    "action_id": pending.action_id,
                    "created_at_unix_ms": pending.created_at_unix_ms,
                    "expires_at_unix_ms": pending.expires_at_unix_ms,
                    "ttl_ms": pending.ttl_ms,
                    "desired": pending.desired,
                }))
            }
            PreviewVerdict::PressureNotElevated => ResponseData::ok(serde_json::json!({
                "verdict": "pressure_not_elevated",
                "note": "no fresh Elevated low-memory evidence; nothing to optimize",
            })),
            PreviewVerdict::Denied(report) => ResponseData::error_with_detail(
                ErrorCode::Ineligible,
                "the target is not eligible for the requested action",
                serde_json::to_value(&report).unwrap_or(serde_json::Value::Null),
            ),
            PreviewVerdict::TtlOutOfRange {
                requested_ms,
                min_ms,
                max_ms,
            } => ResponseData::error(
                ErrorCode::InvalidArgument,
                format!("ttl {requested_ms} ms is outside {min_ms}..={max_ms} ms"),
            ),
            PreviewVerdict::JournalUnavailable => ResponseData::error(
                ErrorCode::JournalUnavailable,
                "the action journal is unavailable; no previews are offered",
            ),
            PreviewVerdict::ActionOutstanding => ResponseData::error(
                ErrorCode::Busy,
                "an action is already outstanding for this session",
            ),
            PreviewVerdict::Disabled => ResponseData::error(
                ErrorCode::Busy,
                "policy is disabled for this session; enabling is an explicit operator action",
            ),
        }
    }

    /// `ConfirmAction`: replay the previewed binding against fresh
    /// observations; durable prepare, verified write, journaled outcome.
    pub fn confirm(&self, request: &ConfirmActionRequest, now_unix_ms: u64) -> ResponseData {
        let (identity, image_name) = {
            let inner = self.inner.lock().expect("policy lock");
            match inner.previewed.get(&request.action_id) {
                Some(binding) => binding.clone(),
                None => {
                    // Lost-reply replay (contract 14 §7): a durable action
                    // ID that already resolved replays its recorded outcome
                    // — never a second write, never "unknown request".
                    if let Ok(Some(record)) = inner.journal.get(&request.action_id) {
                        if record.state == ActionState::Applied {
                            if let Some(deadline) = record.applied_deadline_unix_ms {
                                return ResponseData::ok(serde_json::json!({
                                    "outcome": "replayed",
                                    "action_id": request.action_id,
                                    "deadline_unix_ms": deadline,
                                }));
                            }
                        }
                    }
                    return ResponseData::error(
                        ErrorCode::InvalidArgument,
                        "no pending preview for this action id",
                    );
                }
            }
        };
        let target =
            self.observe_target(identity.pid, identity.creation_filetime_100ns, &image_name);
        // Freshness is enforced by the coordinator's own re-checks; the
        // observation here guarantees the adapters ran at confirm time.
        let _ = target;
        let mut inner = self.inner.lock().expect("policy lock");
        let PolicyInner {
            coordinator,
            journal,
            actuator,
            previewed,
            ..
        } = &mut *inner;
        let outcome = coordinator.confirm(
            &*journal,
            actuator,
            &identity,
            &request.action_id,
            now_unix_ms,
        );
        match outcome {
            ConfirmOutcome::Applied { deadline_unix_ms } => {
                previewed.remove(&request.action_id);
                ResponseData::ok(serde_json::json!({
                    "outcome": "applied",
                    "action_id": request.action_id,
                    "deadline_unix_ms": deadline_unix_ms,
                }))
            }
            ConfirmOutcome::Replayed { deadline_unix_ms } => ResponseData::ok(serde_json::json!({
                "outcome": "replayed",
                "action_id": request.action_id,
                "deadline_unix_ms": deadline_unix_ms,
            })),
            ConfirmOutcome::PreviewExpired => {
                previewed.remove(&request.action_id);
                ResponseData::error(
                    ErrorCode::ConsentExpired,
                    "the preview expired before confirmation",
                )
            }
            ConfirmOutcome::UnknownPreview => {
                previewed.remove(&request.action_id);
                ResponseData::error(
                    ErrorCode::InvalidArgument,
                    "no pending preview for this action id",
                )
            }
            ConfirmOutcome::StorageBlocked(reason) => ResponseData::error(
                ErrorCode::JournalUnavailable,
                format!("the journal refused the action: {reason}"),
            ),
            ConfirmOutcome::FailedClean(reason) => {
                previewed.remove(&request.action_id);
                ResponseData::error(ErrorCode::OsError, reason)
            }
            ConfirmOutcome::Uncertain(reason) => {
                previewed.remove(&request.action_id);
                ResponseData::error(
                    ErrorCode::Uncertain,
                    format!("the write outcome is unknown; recovery will reconcile: {reason}"),
                )
            }
            ConfirmOutcome::IdentityGone => {
                previewed.remove(&request.action_id);
                ResponseData::error(
                    ErrorCode::IdentityMismatch,
                    "the target identity changed between preview and confirm",
                )
            }
            ConfirmOutcome::Disabled => ResponseData::error(
                ErrorCode::Busy,
                "policy is disabled for this session; enabling is an explicit operator action",
            ),
        }
    }

    /// `RestoreAction`: restore one action by durable ID (user intent;
    /// audited as the Disable trigger). The journal record supplies the
    /// identity and the values.
    pub fn restore(&self, request: &RestoreActionRequest, now_unix_ms: u64) -> ResponseData {
        let identity = {
            let inner = self.inner.lock().expect("policy lock");
            match inner.journal.get(&request.action_id) {
                Ok(Some(record)) => {
                    ProcessIdentity::new(record.intent.pid, record.intent.creation_filetime)
                }
                Ok(None) => {
                    return ResponseData::error(
                        ErrorCode::InvalidArgument,
                        "no journal record for this action id",
                    )
                }
                Err(error) => {
                    return ResponseData::error(ErrorCode::JournalUnavailable, error.to_string())
                }
            }
        };
        let mut inner = self.inner.lock().expect("policy lock");
        let PolicyInner {
            engine,
            journal,
            actuator,
            ..
        } = &mut *inner;
        let outcome = engine.restore_once(
            &*journal,
            actuator,
            &identity,
            &request.action_id,
            RestoreTrigger::Disable,
            now_unix_ms,
        );
        restore_response(outcome, &request.action_id)
    }

    /// `DisableActions`: stop accepting new applies and restore everything
    /// outstanding now (contract 14 §6: stop accepting immediately on
    /// disable).
    pub fn disable(&self, _request: &DisableActionsRequest, now_unix_ms: u64) -> ResponseData {
        let mut inner = self.inner.lock().expect("policy lock");
        let PolicyInner {
            coordinator,
            engine,
            journal,
            actuator,
            previewed,
            enabled,
            ..
        } = &mut *inner;
        coordinator.disable();
        *enabled = false;
        let mut restored = 0u32;
        let mut unresolved = 0u32;
        for record in journal.unresolved().unwrap_or_default() {
            let identity = ProcessIdentity::new(record.intent.pid, record.intent.creation_filetime);
            match engine.restore_once(
                &*journal,
                actuator,
                &identity,
                &record.intent.action_id,
                RestoreTrigger::Disable,
                now_unix_ms,
            ) {
                RestoreOutcome::Restored { .. } | RestoreOutcome::TargetGone => restored += 1,
                _ => unresolved += 1,
            }
        }
        previewed.clear();
        ResponseData::ok(serde_json::json!({
            "disabled": true,
            "restored": restored,
            "unresolved": unresolved,
        }))
    }
}

/// One journal record as the `ListActions` wire row. `unresolved` marks
/// the pending/conflict set; `pinned` flags recovery intent.
fn action_row(record: &ramshaper_journal::JournalRecord, unresolved: bool) -> serde_json::Value {
    serde_json::json!({
        "action_id": record.intent.action_id,
        "state": format!("{:?}", record.state),
        "target_name": record.intent.target_name,
        "pid": record.intent.pid,
        "original": record.intent.property.original,
        "desired": record.intent.property.desired,
        "created_at_unix_ms": record.intent.created_at_unix_ms,
        "applied_deadline_unix_ms": record.applied_deadline_unix_ms,
        "terminal_at_unix_ms": record.terminal_at_unix_ms,
        "unresolved": unresolved,
        "pinned": record.is_pinned(),
    })
}

/// The handler result before envelope framing: data or typed failure.
#[derive(Debug)]
pub enum ResponseData {
    /// Success payload.
    Ok(serde_json::Value),
    /// Typed failure with machine code, message and optional detail.
    Err {
        /// Machine-readable code.
        code: ErrorCode,
        /// Redacted human-readable explanation.
        message: String,
        /// Optional structured detail (e.g. the explain-all report).
        detail: Option<serde_json::Value>,
    },
}

impl ResponseData {
    #[allow(dead_code)] // wired by dispatch.rs for malformed payloads
    pub(crate) fn ok(value: serde_json::Value) -> Self {
        Self::Ok(value)
    }

    pub(crate) fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Err {
            code,
            message: message.into(),
            detail: None,
        }
    }

    fn error_with_detail(code: ErrorCode, message: &str, detail: serde_json::Value) -> Self {
        Self::Err {
            code,
            message: message.to_string(),
            detail: Some(detail),
        }
    }

    /// Frame into a protocol response for one request.
    #[must_use]
    pub fn into_response(self, envelope: &RequestEnvelope, instance_id: &str) -> Response {
        match self {
            Self::Ok(data) => Response::ok(&envelope.request_id, instance_id, data),
            Self::Err {
                code,
                message,
                detail,
            } => Response::error_with_detail(
                &envelope.request_id,
                instance_id,
                code,
                message,
                detail,
            ),
        }
    }
}

/// Map a restore outcome to its wire response.
fn restore_response(outcome: RestoreOutcome, action_id: &str) -> ResponseData {
    match outcome {
        RestoreOutcome::Restored { wrote_back } => ResponseData::ok(serde_json::json!({
            "outcome": "restored",
            "action_id": action_id,
            "wrote_back": wrote_back,
        })),
        RestoreOutcome::Conflict => ResponseData::error(
            ErrorCode::Conflict,
            "an external change holds the setting; it is preserved and needs attention",
        ),
        RestoreOutcome::TargetGone => ResponseData::error(
            ErrorCode::IdentityMismatch,
            "the target exited or its identity moved on; nothing was written",
        ),
        RestoreOutcome::RetryScheduled { due_unix_ms } => ResponseData::ok(serde_json::json!({
            "outcome": "retry_scheduled",
            "action_id": action_id,
            "due_unix_ms": due_unix_ms,
        })),
        RestoreOutcome::ManualRetry => ResponseData::ok(serde_json::json!({
            "outcome": "manual_retry",
            "action_id": action_id,
            "note": "the bounded retries are exhausted; the record stays pending for manual retry",
        })),
        RestoreOutcome::NeedsAttention => ResponseData::error(
            ErrorCode::AccessDenied,
            "the OS denied a restore step; attention required, no retries",
        ),
        RestoreOutcome::NotRestorable => ResponseData::error(
            ErrorCode::InvalidArgument,
            "no applied action with this id is in a restorable state",
        ),
        RestoreOutcome::StorageBlocked(reason) => {
            ResponseData::error(ErrorCode::JournalUnavailable, reason)
        }
    }
}
