//! Deterministic policy dry-run over synthetic traces (RS-019).
//!
//! Simulates the 0.2 policy decision layer — **without any OS write** — by
//! composing the real building blocks the policy coordinator will use:
//! the `pressure_v1` classifier (RS-009), the eligibility matrix
//! (RS-015) and the journal transition table (RS-031). Given the same
//! rules and the same trace, [`dry_run`] produces the identical decision
//! log; this pins the policy's cause/effect behavior (including every
//! deny/no-action path) before `PreviewAction`/`ConfirmAction` exist
//! (RS-016) and long before any actuator is enabled.
//!
//! The 0.2 policy (contract 14 §§4–5): one manually confirmed
//! Normal → Below Normal CPU-priority experiment per trace, offered only
//! under fresh `Elevated` pressure with a fully eligible target,
//! confirmed within a 30 s preview window, applied with a TTL of
//! 10–300 s (default 120 s), and resolved by the contract's restore
//! triggers. Simulated applies/restores always succeed here — the dry
//! run validates *decisions*, not OS latency or failure injection
//! (that is AT-12/AT-18 territory over the real store).
//!
//! Deliberately not modeled (recorded, not hidden): the per-identity
//! 60 s cooldown and the stale-input safety *trip* (RS-018 owns both —
//! a completed experiment may be re-offered in this simulation), and
//! pressure returning to `Normal` is **not** a restore trigger (the
//! contract's trigger list does not include it).

use serde::Serialize;

use crate::eligibility::{
    evaluate, Allowlist, ChildObservation, DenyReason, EligibilityInputs, FocusObservation,
    OwnerSessionObservation, PriorityCapability, PriorityClassObservation, SafetyClassification,
};
use crate::identity::{IdentityVerification, ProcessIdentity};
use crate::journal::{transition, ActionState, Event};
use crate::pressure::{PressureClassifier, PressureInput, PressureLevel, PRESSURE_RULE_VERSION};

/// Version of the dry-run decision rules.
pub const POLICY_RULES_VERSION: &str = "policy_v1";

/// Default experiment TTL (contract 14 §5: 120 s).
pub const DEFAULT_TTL_MS: u64 = 120_000;

/// Allowed TTL range (contract 14 §5: 10–300 s).
pub const MIN_TTL_MS: u64 = 10_000;
pub const MAX_TTL_MS: u64 = 300_000;

/// A pending preview expires after this long (contract 14 §5).
pub const PREVIEW_VALIDITY_MS: u64 = 30_000;

/// Schema marker for a serialized dry-run log.
pub const DRY_RUN_SCHEMA: &str = "ramshaper.policy-dryrun.v1";

/// The experiment rules in force. Values come from contract 14 §5; profiles
/// are inert labels and do not change these numbers (RS-012).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRules {
    /// Rules version stamped into previews and applies.
    pub rules_version: String,
    /// Experiment TTL, ms (10 s–300 s inclusive).
    pub ttl_ms: u64,
}

/// Why a rules value is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyRuleError {
    /// TTL outside the allowed 10–300 s window.
    #[error("ttl_ms {ttl_ms} outside allowed range {MIN_TTL_MS}..={MAX_TTL_MS}")]
    TtlOutOfRange { ttl_ms: u64 },
}

impl PolicyRules {
    /// Build rules, rejecting a TTL outside 10–300 s.
    pub fn new(ttl_ms: u64) -> Result<Self, PolicyRuleError> {
        if !(MIN_TTL_MS..=MAX_TTL_MS).contains(&ttl_ms) {
            return Err(PolicyRuleError::TtlOutOfRange { ttl_ms });
        }
        Ok(Self {
            rules_version: POLICY_RULES_VERSION.to_string(),
            ttl_ms,
        })
    }

    /// The default experiment: 120 s TTL.
    #[must_use]
    pub fn default_experiment() -> Self {
        Self {
            rules_version: POLICY_RULES_VERSION.to_string(),
            ttl_ms: DEFAULT_TTL_MS,
        }
    }
}

/// The target-side observations one trace step provides. These feed the
/// eligibility matrix verbatim; `all_pass` is a convenience for traces
/// that vary only pressure.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetObservation {
    /// Fresh identity re-check.
    pub identity_check: IdentityVerification,
    /// Owner/session observation.
    pub owner_session: OwnerSessionObservation,
    /// Foreground observation.
    pub focus: FocusObservation,
    /// Safety classification.
    pub classification: SafetyClassification,
    /// Current priority class.
    pub priority_class: PriorityClassObservation,
    /// Live-children observation.
    pub children: ChildObservation,
    /// Priority API capability.
    pub capability: PriorityCapability,
}

impl TargetObservation {
    /// An observation where every eligibility input passes (still subject
    /// to allowlist/exclusions/active-action checks the engine adds).
    #[must_use]
    pub fn all_pass() -> Self {
        Self {
            identity_check: IdentityVerification::Matches,
            owner_session: OwnerSessionObservation::CurrentUserSession,
            focus: FocusObservation::Observed {
                target_foreground: false,
                observed_age_ms: 100,
            },
            classification: SafetyClassification::VerifiedNonCritical,
            priority_class: PriorityClassObservation::Normal,
            children: ChildObservation::Observed { live_children: 0 },
            capability: PriorityCapability::Supported,
        }
    }
}

/// One synthetic trace step: everything observed/decided at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceStep<'a> {
    /// Monotonic time of this step (ms) — the dry run has no clock.
    pub monotonic_ms: u64,
    /// Low-memory signal: `Some(signaled)` when the observation is valid,
    /// `None` when inputs are missing/stale (pressure → Unknown).
    pub low_memory: Option<bool>,
    /// Target image name (compatibility label only).
    pub image_name: &'a str,
    /// Target observations for the eligibility matrix.
    pub target: TargetObservation,
    /// Synthetic manual confirmation of a pending preview.
    pub confirm: bool,
    /// Synthetic manual disable (restores any active experiment; stops new
    /// previews for the rest of the trace).
    pub disable: bool,
    /// Synthetic external priority change on an active experiment
    /// (resolve path: conflict preserved, never overwritten).
    pub external_priority_change: bool,
    /// Synthetic configuration invalidation (restore trigger per contract).
    pub config_invalidated: bool,
    /// Journal availability at this step (blocks new applies, never
    /// recovery — an active experiment still resolves when false).
    pub journal_available: bool,
}

impl<'a> TraceStep<'a> {
    /// A step with a valid non-signaling observation and an all-pass target;
    /// convenience for building traces field by field.
    #[must_use]
    pub fn new(monotonic_ms: u64, image_name: &'a str) -> Self {
        Self {
            monotonic_ms,
            low_memory: Some(false),
            image_name,
            target: TargetObservation::all_pass(),
            confirm: false,
            disable: false,
            external_priority_change: false,
            config_invalidated: false,
            journal_available: true,
        }
    }
}

/// Environment the trace runs against.
pub struct DryRunConfig<'a> {
    /// Experiment rules.
    pub rules: &'a PolicyRules,
    /// Tested compatibility allowlist.
    pub allowlist: &'a Allowlist,
    /// User exclusion names (static for the run; mid-trace invalidation is
    /// the explicit `config_invalidated` step flag).
    pub user_exclusions: &'a [String],
    /// The exact candidate identity the trace describes.
    pub target_identity: &'a ProcessIdentity,
}

/// Why nothing was offered/applied at a step. Every deny path is explicit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum NoActionReason {
    /// Pressure inputs missing/stale.
    PressureUnknown,
    /// Pressure classified but not Elevated (including Normal).
    PressureNotElevated,
    /// Manual disable is in force.
    Disabled,
    /// Another experiment is still active (one-action cap).
    ActionActive {
        /// The outstanding action.
        action_id: String,
    },
    /// The target failed eligibility; every deny reason is carried.
    Ineligible {
        /// All deny reasons, in check order.
        denies: Vec<DenyReason>,
    },
    /// A confirmation arrived with no pending preview.
    NoValidPreview,
    /// The preview was valid once but older than 30 s at confirmation.
    PreviewExpired {
        /// When the preview was offered.
        offered_at_ms: u64,
        /// When the confirmation arrived.
        confirmed_at_ms: u64,
    },
    /// Journal unavailable — no durable intent possible.
    JournalUnavailable,
}

impl NoActionReason {
    /// Stable machine code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::PressureUnknown => "pressure_unknown",
            Self::PressureNotElevated => "pressure_not_elevated",
            Self::Disabled => "disabled",
            Self::ActionActive { .. } => "action_active",
            Self::Ineligible { .. } => "ineligible",
            Self::NoValidPreview => "no_valid_preview",
            Self::PreviewExpired { .. } => "preview_expired",
            Self::JournalUnavailable => "journal_unavailable",
        }
    }
}

/// What ended an active experiment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveCause {
    /// Manual disable.
    Disabled,
    /// Target exited or identity no longer matches.
    TargetVanished,
    /// An external writer changed the priority.
    ExternalConflict,
    /// Target returned to the foreground.
    ForegroundReturned,
    /// A live child appeared (R-10).
    ChildAppeared,
    /// Foreground observation went stale (stale safety input).
    StaleSafetyInput,
    /// Configuration invalidation.
    ConfigInvalidated,
    /// Deadline reached.
    TtlExpired,
}

impl ResolveCause {
    /// Stable machine code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::TargetVanished => "target_vanished",
            Self::ExternalConflict => "external_conflict",
            Self::ForegroundReturned => "foreground_returned",
            Self::ChildAppeared => "child_appeared",
            Self::StaleSafetyInput => "stale_safety_input",
            Self::ConfigInvalidated => "config_invalidated",
            Self::TtlExpired => "ttl_expired",
        }
    }
}

/// The decision at one trace step.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum PolicyDecision {
    /// Nothing offered, applied or resolved; the reason is explicit.
    NoAction {
        /// Why.
        reason: NoActionReason,
    },
    /// A preview would be offered (manual confirmation required — the dry
    /// run never "auto-applies").
    PreviewOffered {
        /// Deterministic dry-run action id (`dry-N`).
        action_id: String,
        /// TTL the experiment would carry.
        ttl_ms: u64,
        /// Rules version.
        rules_version: String,
    },
    /// A confirmed experiment was simulated through the journal state
    /// machine: `Prepared` (durable intent) → `ApplyVerified` → `Applied`.
    /// No OS write happened; the fake actuator always succeeds here.
    ApplySimulated {
        /// The action's id.
        action_id: String,
        /// Live deadline (monotonic ms) = confirm time + TTL.
        deadline_monotonic_ms: u64,
    },
    /// An active experiment was resolved through the journal state machine
    /// (`Applied` → `RestorePending` → terminal), driven by `cause`.
    ActionResolved {
        /// The action's id.
        action_id: String,
        /// What triggered the resolution.
        cause: ResolveCause,
        /// Final journal state: `Restored`, `TargetGone` or `Conflict`.
        final_state: ActionState,
    },
    /// A confirmation replayed against an already-active action: the
    /// existing result stands, no second apply (idempotency).
    ReplayIgnored {
        /// The already-active action.
        action_id: String,
    },
}

/// One logged step.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepDecision {
    /// Trace index (0-based).
    pub step: usize,
    /// Monotonic time of the step.
    pub monotonic_ms: u64,
    /// Pressure level after feeding this step's observation.
    pub pressure_level: PressureLevel,
    /// The decision.
    pub decision: PolicyDecision,
}

/// Aggregate counts over a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct DryRunSummary {
    /// Steps that offered a preview.
    pub previews_offered: u32,
    /// Simulated applies.
    pub applies_simulated: u32,
    /// Resolutions (any final state).
    pub actions_resolved: u32,
    /// Idempotent confirm replays.
    pub replays_ignored: u32,
    /// No-action steps (each with an explicit reason).
    pub no_actions: u32,
}

/// The full deterministic decision log for one trace.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DryRunLog {
    /// Schema marker.
    pub schema: String,
    /// Policy rules version.
    pub rules_version: String,
    /// Pressure rule version used.
    pub pressure_rule_version: String,
    /// One entry per trace step, in order.
    pub steps: Vec<StepDecision>,
    /// Aggregate counts.
    pub summary: DryRunSummary,
}

struct ActiveAction {
    action_id: String,
    state: ActionState,
    deadline_monotonic_ms: u64,
}

struct PendingPreview {
    action_id: String,
    offered_at_ms: u64,
}

/// Evaluate one synthetic trace. Pure and deterministic: same rules, same
/// trace → byte-identical log.
pub fn dry_run(config: &DryRunConfig<'_>, trace: &[TraceStep<'_>]) -> DryRunLog {
    let mut classifier = PressureClassifier::new();
    let mut active: Option<ActiveAction> = None;
    let mut preview: Option<PendingPreview> = None;
    let mut disabled = false;
    let mut next_action_number: u32 = 1;
    let mut steps = Vec::with_capacity(trace.len());
    let mut summary = DryRunSummary::default();

    for (index, step) in trace.iter().enumerate() {
        let observation = classifier.observe(step.low_memory.map(|signaled| PressureInput {
            low_memory_signaled: signaled,
            monotonic_ms: step.monotonic_ms,
        }));
        let decision = decide(
            config,
            step,
            observation.level,
            &mut active,
            &mut preview,
            &mut disabled,
            &mut next_action_number,
        );
        match &decision {
            PolicyDecision::PreviewOffered { .. } => summary.previews_offered += 1,
            PolicyDecision::ApplySimulated { .. } => summary.applies_simulated += 1,
            PolicyDecision::ActionResolved { .. } => summary.actions_resolved += 1,
            PolicyDecision::ReplayIgnored { .. } => summary.replays_ignored += 1,
            PolicyDecision::NoAction { .. } => summary.no_actions += 1,
        }
        steps.push(StepDecision {
            step: index,
            monotonic_ms: step.monotonic_ms,
            pressure_level: observation.level,
            decision,
        });
    }

    DryRunLog {
        schema: DRY_RUN_SCHEMA.to_string(),
        rules_version: config.rules.rules_version.clone(),
        pressure_rule_version: PRESSURE_RULE_VERSION.to_string(),
        steps,
        summary,
    }
}

/// Deterministic trigger precedence when several fire at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    Disabled,
    TargetVanished,
    ExternalConflict,
    ForegroundReturned,
    ChildAppeared,
    StaleSafetyInput,
    ConfigInvalidated,
    TtlExpired,
}

fn identity_vanished(check: &IdentityVerification) -> bool {
    matches!(
        check,
        IdentityVerification::Mismatch { .. } | IdentityVerification::Unreachable { .. }
    )
}

fn focus_is_stale(focus: &FocusObservation) -> bool {
    matches!(
        focus,
        FocusObservation::Observed {
            target_foreground: false,
            observed_age_ms,
        } if *observed_age_ms > crate::eligibility::APPLY_INPUT_FRESHNESS_MS,
    ) || matches!(focus, FocusObservation::Unknown { .. })
}

fn first_trigger(step: &TraceStep<'_>, active: &ActiveAction, step_ms: u64) -> Option<Trigger> {
    // Deterministic precedence; the contract imposes none, so the dry run
    // fixes one and pins it in tests.
    if step.disable {
        return Some(Trigger::Disabled);
    }
    if identity_vanished(&step.target.identity_check) {
        return Some(Trigger::TargetVanished);
    }
    if step.external_priority_change {
        return Some(Trigger::ExternalConflict);
    }
    if let FocusObservation::Observed {
        target_foreground: true,
        ..
    } = step.target.focus
    {
        return Some(Trigger::ForegroundReturned);
    }
    if let ChildObservation::Observed { live_children } = step.target.children {
        if live_children > 0 {
            return Some(Trigger::ChildAppeared);
        }
    }
    if focus_is_stale(&step.target.focus) {
        return Some(Trigger::StaleSafetyInput);
    }
    if step.config_invalidated {
        return Some(Trigger::ConfigInvalidated);
    }
    if step_ms >= active.deadline_monotonic_ms {
        return Some(Trigger::TtlExpired);
    }
    None
}

fn resolve_cause(trigger: Trigger) -> ResolveCause {
    match trigger {
        Trigger::Disabled => ResolveCause::Disabled,
        Trigger::TargetVanished => ResolveCause::TargetVanished,
        Trigger::ExternalConflict => ResolveCause::ExternalConflict,
        Trigger::ForegroundReturned => ResolveCause::ForegroundReturned,
        Trigger::ChildAppeared => ResolveCause::ChildAppeared,
        Trigger::StaleSafetyInput => ResolveCause::StaleSafetyInput,
        Trigger::ConfigInvalidated => ResolveCause::ConfigInvalidated,
        Trigger::TtlExpired => ResolveCause::TtlExpired,
    }
}

fn decide(
    config: &DryRunConfig<'_>,
    step: &TraceStep<'_>,
    pressure_level: PressureLevel,
    active: &mut Option<ActiveAction>,
    preview: &mut Option<PendingPreview>,
    disabled: &mut bool,
    next_action_number: &mut u32,
) -> PolicyDecision {
    // 1. Resolve an active experiment first (recovery is never blocked, not
    //    even by journal unavailability).
    if let Some(current) = active.as_ref() {
        if let Some(trigger) = first_trigger(step, current, step.monotonic_ms) {
            let action_id = current.action_id.clone();
            // The record's real state (Applied after a simulated confirm)
            // → RestoreRequested → RestorePending (identity and current
            // value revalidated there, per the contract table).
            let restore_pending = transition(current.state, &Event::RestoreRequested)
                .expect("active record --RestoreRequested--> RestorePending is legal");
            let final_state = match trigger {
                Trigger::TargetVanished => transition(restore_pending, &Event::TargetVanished)
                    .expect("RestorePending --TargetVanished--> TargetGone is legal"),
                Trigger::ExternalConflict => transition(restore_pending, &Event::ExternalChange)
                    .expect("RestorePending --ExternalChange--> Conflict is legal"),
                _ => transition(restore_pending, &Event::RestoreVerified)
                    .expect("RestorePending --RestoreVerified--> Restored is legal"),
            };
            *active = None;
            preview.take();
            if trigger == Trigger::Disabled {
                *disabled = true;
            }
            return PolicyDecision::ActionResolved {
                action_id,
                cause: resolve_cause(trigger),
                final_state,
            };
        }
        // 2. A confirmation against an active action is an idempotent
        //    replay: the existing result stands.
        if step.confirm {
            return PolicyDecision::ReplayIgnored {
                action_id: current.action_id.clone(),
            };
        }
        // 3. One-action cap: nothing new is offered while an experiment is
        //    outstanding.
        return PolicyDecision::NoAction {
            reason: NoActionReason::ActionActive {
                action_id: current.action_id.clone(),
            },
        };
    }

    // 4. A confirmation with no active action consumes a pending preview.
    if step.confirm {
        let Some(pending) = preview.take() else {
            return PolicyDecision::NoAction {
                reason: NoActionReason::NoValidPreview,
            };
        };
        if step.monotonic_ms.saturating_sub(pending.offered_at_ms) > PREVIEW_VALIDITY_MS {
            return PolicyDecision::NoAction {
                reason: NoActionReason::PreviewExpired {
                    offered_at_ms: pending.offered_at_ms,
                    confirmed_at_ms: step.monotonic_ms,
                },
            };
        }
        if !step.journal_available {
            return PolicyDecision::NoAction {
                reason: NoActionReason::JournalUnavailable,
            };
        }
        // Eligibility must still hold at confirmation (a preview expires on
        // any eligibility change — here that surfaces as an ineligible
        // confirmation, never a surprise apply).
        let report = evaluate(&inputs_for(config, step, 0));
        if !report.eligible {
            return PolicyDecision::NoAction {
                reason: NoActionReason::Ineligible {
                    denies: report.deny_reasons().into_iter().cloned().collect(),
                },
            };
        }
        let deadline = step.monotonic_ms + config.rules.ttl_ms;
        // Prepared (durable intent) → ApplyVerified → Applied.
        let applied = transition(
            ActionState::Prepared,
            &Event::ApplyVerified {
                deadline_unix_ms: deadline,
            },
        )
        .expect("Prepared --ApplyVerified--> Applied is legal");
        let action_id = pending.action_id;
        *active = Some(ActiveAction {
            action_id: action_id.clone(),
            state: applied,
            deadline_monotonic_ms: deadline,
        });
        return PolicyDecision::ApplySimulated {
            action_id,
            deadline_monotonic_ms: deadline,
        };
    }

    // 5. Offer evaluation: offer only under fresh Elevated pressure with a
    //    fully eligible target.
    if *disabled {
        return PolicyDecision::NoAction {
            reason: NoActionReason::Disabled,
        };
    }
    if !step.journal_available {
        return PolicyDecision::NoAction {
            reason: NoActionReason::JournalUnavailable,
        };
    }
    match pressure_level {
        PressureLevel::Unknown => {
            return PolicyDecision::NoAction {
                reason: NoActionReason::PressureUnknown,
            }
        }
        PressureLevel::Normal => {
            return PolicyDecision::NoAction {
                reason: NoActionReason::PressureNotElevated,
            }
        }
        PressureLevel::Elevated => {}
    }
    let report = evaluate(&inputs_for(config, step, 0));
    if !report.eligible {
        return PolicyDecision::NoAction {
            reason: NoActionReason::Ineligible {
                denies: report.deny_reasons().into_iter().cloned().collect(),
            },
        };
    }
    // Re-offering while a valid preview is pending keeps the same action id
    // (a preview is not a new action); an expired preview is replaced.
    let pending_id = match preview.as_ref() {
        Some(p) if step.monotonic_ms.saturating_sub(p.offered_at_ms) <= PREVIEW_VALIDITY_MS => {
            p.action_id.clone()
        }
        _ => {
            let id = format!("dry-{next_action_number}");
            *next_action_number += 1;
            *preview = Some(PendingPreview {
                action_id: id.clone(),
                offered_at_ms: step.monotonic_ms,
            });
            id
        }
    };
    PolicyDecision::PreviewOffered {
        action_id: pending_id,
        ttl_ms: config.rules.ttl_ms,
        rules_version: config.rules.rules_version.clone(),
    }
}

fn inputs_for<'a>(
    config: &DryRunConfig<'a>,
    step: &TraceStep<'a>,
    outstanding_actions: usize,
) -> EligibilityInputs<'a> {
    EligibilityInputs {
        identity: config.target_identity,
        image_name: step.image_name,
        identity_check: step.target.identity_check.clone(),
        owner_session: step.target.owner_session.clone(),
        focus: step.target.focus.clone(),
        classification: step.target.classification.clone(),
        priority_class: step.target.priority_class.clone(),
        children: step.target.children.clone(),
        capability: step.target.capability.clone(),
        allowlist: config.allowlist,
        user_exclusions: config.user_exclusions,
        outstanding_actions,
        journal_available: step.journal_available,
    }
}

/// `NORMAL_PRIORITY_CLASS` — the only original value the 0.2 policy finds
/// actionable. The real actuator asserts this equals the OS constant.
pub const NORMAL_PRIORITY_CLASS: u32 = 0x0020;

/// `BELOW_NORMAL_PRIORITY_CLASS` — the only value the 0.2 policy action
/// ever writes. The real actuator asserts this equals the OS constant.
pub const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x4000;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eligibility::{Allowlist, ChildObservation, FocusObservation};
    use crate::identity::ProcessIdentity;
    use crate::snapshot::QueryFailure;

    const IMAGE: &str = "leafapp.exe";

    fn tested_allowlist() -> Allowlist {
        Allowlist::new("eligibility-v1", &["leafapp.exe"])
    }

    fn env<'a>(
        r: &'a PolicyRules,
        allowlist: &'a Allowlist,
        id: &'a ProcessIdentity,
    ) -> DryRunConfig<'a> {
        DryRunConfig {
            rules: r,
            allowlist,
            user_exclusions: &[],
            target_identity: id,
        }
    }

    fn target() -> ProcessIdentity {
        ProcessIdentity::new(42, 133_000_000_000_000_000)
    }

    /// `dry_run` over a fresh environment and the given trace.
    fn run(trace: &[TraceStep<'static>], allowlist: &Allowlist) -> DryRunLog {
        let r = PolicyRules::default_experiment();
        let id = target();
        dry_run(&env(&r, allowlist, &id), trace)
    }

    /// Steps at 0/1000/2000 ms, all signaling low memory: Elevated at
    /// index 2 (3 consecutive, 2 s span).
    fn elevated_trace() -> Vec<TraceStep<'static>> {
        vec![signaled(0), signaled(1_000), signaled(2_000)]
    }

    fn signaled(ms: u64) -> TraceStep<'static> {
        TraceStep {
            low_memory: Some(true),
            ..TraceStep::new(ms, IMAGE)
        }
    }

    fn no_action_reason(decision: &PolicyDecision) -> &NoActionReason {
        match decision {
            PolicyDecision::NoAction { reason } => reason,
            other => panic!("expected NoAction, got {other:?}"),
        }
    }

    #[test]
    fn rules_validate_ttl_range() {
        assert!(PolicyRules::new(MIN_TTL_MS - 1).is_err());
        assert!(PolicyRules::new(MIN_TTL_MS).is_ok());
        assert!(PolicyRules::new(MAX_TTL_MS).is_ok());
        assert!(PolicyRules::new(MAX_TTL_MS + 1).is_err());
        let d = PolicyRules::default_experiment();
        assert_eq!(d.ttl_ms, DEFAULT_TTL_MS);
        assert_eq!(d.rules_version, POLICY_RULES_VERSION);
    }

    #[test]
    fn quiet_trace_never_offers_anything() {
        let allowlist = tested_allowlist();
        let trace: Vec<TraceStep<'static>> =
            (0..10).map(|i| TraceStep::new(i * 1_000, IMAGE)).collect();
        let log = run(&trace, &allowlist);
        assert_eq!(
            log.summary,
            DryRunSummary {
                no_actions: 10,
                ..DryRunSummary::default()
            }
        );
        // First observations are Unknown; five clears over 4 s -> Normal.
        assert_eq!(
            no_action_reason(&log.steps[0].decision).code(),
            "pressure_unknown"
        );
        assert_eq!(
            no_action_reason(&log.steps[9].decision).code(),
            "pressure_not_elevated"
        );
    }

    #[test]
    fn elevated_pressure_with_eligible_target_offers_exactly_one_preview() {
        let allowlist = tested_allowlist();
        let log = run(&elevated_trace(), &allowlist);
        // Steps 0-1: pressure still Unknown (not enough consecutive).
        assert_eq!(
            no_action_reason(&log.steps[0].decision).code(),
            "pressure_unknown"
        );
        match &log.steps[2].decision {
            PolicyDecision::PreviewOffered {
                action_id,
                ttl_ms,
                rules_version,
            } => {
                assert_eq!(action_id, "dry-1");
                assert_eq!(*ttl_ms, DEFAULT_TTL_MS);
                assert_eq!(rules_version, POLICY_RULES_VERSION);
            }
            other => panic!("expected PreviewOffered, got {other:?}"),
        }
        assert_eq!(log.summary.previews_offered, 1);
    }

    #[test]
    fn dry_run_is_deterministic() {
        let allowlist = tested_allowlist();
        let a = run(&elevated_trace(), &allowlist);
        let b = run(&elevated_trace(), &allowlist);
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_string(&a).expect("serialize"),
            serde_json::to_string(&b).expect("serialize")
        );
    }

    #[test]
    fn ineligible_target_carries_the_eligibility_denies() {
        let empty = Allowlist::empty();
        let log = run(&elevated_trace(), &empty);
        match no_action_reason(&log.steps[2].decision) {
            NoActionReason::Ineligible { denies } => {
                let codes: Vec<&str> = denies.iter().map(|d| d.code()).collect();
                assert_eq!(codes, vec!["not_on_tested_allowlist"]);
            }
            reason => panic!("expected Ineligible, got {}", reason.code()),
        }
        // A foreground target denies through the eligibility path too.
        let mut fg = elevated_trace();
        fg[2].target.focus = FocusObservation::Observed {
            target_foreground: true,
            observed_age_ms: 50,
        };
        let log = run(&fg, &tested_allowlist());
        match no_action_reason(&log.steps[2].decision) {
            NoActionReason::Ineligible { denies } => {
                assert!(denies.iter().any(|d| d.code() == "target_foreground"));
            }
            reason => panic!("expected Ineligible, got {}", reason.code()),
        }
    }

    #[test]
    fn confirm_without_preview_is_no_action() {
        let allowlist = tested_allowlist();
        let mut trace = elevated_trace();
        trace[0].confirm = true;
        let log = run(&trace, &allowlist);
        assert_eq!(
            no_action_reason(&log.steps[0].decision).code(),
            "no_valid_preview"
        );
    }

    /// Offer at t=2_000, confirm at t=2_500, expire at t=122_500.
    fn lifecycle_trace() -> Vec<TraceStep<'static>> {
        let mut trace = elevated_trace();
        let mut confirm = signaled(2_500);
        confirm.confirm = true;
        trace.push(confirm);
        // Still signaled at the deadline boundary (level retained Elevated).
        trace.push(signaled(122_500));
        trace
    }

    #[test]
    fn full_lifecycle_confirm_then_ttl_expiry_at_deadline() {
        let allowlist = tested_allowlist();
        let log = run(&lifecycle_trace(), &allowlist);
        match &log.steps[3].decision {
            PolicyDecision::ApplySimulated {
                action_id,
                deadline_monotonic_ms,
            } => {
                assert_eq!(action_id, "dry-1");
                assert_eq!(*deadline_monotonic_ms, 2_500 + DEFAULT_TTL_MS);
            }
            other => panic!("expected ApplySimulated, got {other:?}"),
        }
        match &log.steps[4].decision {
            PolicyDecision::ActionResolved {
                action_id,
                cause,
                final_state,
            } => {
                assert_eq!(action_id, "dry-1");
                assert_eq!(*cause, ResolveCause::TtlExpired);
                assert_eq!(*final_state, ActionState::Restored);
            }
            other => panic!("expected ActionResolved, got {other:?}"),
        }
        assert_eq!(
            log.summary,
            DryRunSummary {
                previews_offered: 1,
                applies_simulated: 1,
                actions_resolved: 1,
                no_actions: 2,
                replays_ignored: 0,
            }
        );
    }

    #[test]
    fn resolve_triggers_map_to_contract_final_states() {
        type Mutation = Box<dyn Fn(&mut TraceStep<'static>)>;
        let cases: Vec<Mutation> = vec![
            Box::new(|s: &mut TraceStep<'static>| {
                s.target.focus = FocusObservation::Observed {
                    target_foreground: true,
                    observed_age_ms: 50,
                };
            }),
            Box::new(|s: &mut TraceStep<'static>| {
                s.target.identity_check = IdentityVerification::Unreachable {
                    reason: QueryFailure::Exited { code: 87 },
                };
            }),
            Box::new(|s: &mut TraceStep<'static>| {
                s.external_priority_change = true;
            }),
            Box::new(|s: &mut TraceStep<'static>| {
                s.target.children = ChildObservation::Observed { live_children: 1 };
            }),
            Box::new(|s: &mut TraceStep<'static>| {
                s.target.focus = FocusObservation::Observed {
                    target_foreground: false,
                    observed_age_ms: 3_000,
                };
            }),
            Box::new(|s: &mut TraceStep<'static>| {
                s.config_invalidated = true;
            }),
            Box::new(|s: &mut TraceStep<'static>| {
                s.disable = true;
            }),
        ];
        let expected = [
            (ResolveCause::ForegroundReturned, ActionState::Restored),
            (ResolveCause::TargetVanished, ActionState::TargetGone),
            (ResolveCause::ExternalConflict, ActionState::Conflict),
            (ResolveCause::ChildAppeared, ActionState::Restored),
            (ResolveCause::StaleSafetyInput, ActionState::Restored),
            (ResolveCause::ConfigInvalidated, ActionState::Restored),
            (ResolveCause::Disabled, ActionState::Restored),
        ];
        for (mutation, (cause, final_state)) in cases.into_iter().zip(expected) {
            let allowlist = tested_allowlist();
            let mut trace = lifecycle_trace();
            let len = trace.len();
            mutation(&mut trace[len - 1]);
            let log = run(&trace, &allowlist);
            match &log.steps[len - 1].decision {
                PolicyDecision::ActionResolved {
                    cause: got_cause,
                    final_state: got_state,
                    ..
                } => {
                    assert_eq!(*got_cause, cause, "cause mismatch for {cause:?}");
                    assert_eq!(*got_state, final_state, "state mismatch for {cause:?}");
                }
                other => panic!("expected ActionResolved for {cause:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn trigger_precedence_disable_wins() {
        let allowlist = tested_allowlist();
        let mut trace = lifecycle_trace();
        let last = trace.len() - 1;
        trace[last].disable = true;
        trace[last].external_priority_change = true;
        trace[last].target.focus = FocusObservation::Observed {
            target_foreground: true,
            observed_age_ms: 50,
        };
        let log = run(&trace, &allowlist);
        match &log.steps[last].decision {
            PolicyDecision::ActionResolved { cause, .. } => {
                assert_eq!(*cause, ResolveCause::Disabled);
            }
            other => panic!("expected ActionResolved, got {other:?}"),
        }
    }

    #[test]
    fn disable_blocks_future_offers() {
        let allowlist = tested_allowlist();
        let mut trace = lifecycle_trace();
        trace[4].disable = true; // resolves the active experiment AND disables
        trace.push(signaled(123_000)); // still Elevated (retained), disabled now
        let log = run(&trace, &allowlist);
        assert_eq!(no_action_reason(&log.steps[5].decision).code(), "disabled");
    }

    #[test]
    fn one_action_cap_blocks_new_previews_while_active() {
        let allowlist = tested_allowlist();
        let mut trace = lifecycle_trace();
        // Before the deadline: still active, still Elevated, still eligible.
        trace.insert(4, signaled(100_000));
        let log = run(&trace, &allowlist);
        match no_action_reason(&log.steps[4].decision) {
            NoActionReason::ActionActive { action_id } => assert_eq!(action_id, "dry-1"),
            reason => panic!("expected ActionActive, got {}", reason.code()),
        }
    }

    #[test]
    fn confirm_replay_is_idempotent() {
        let allowlist = tested_allowlist();
        let mut trace = lifecycle_trace();
        let mut replay = signaled(50_000);
        replay.confirm = true;
        trace.insert(4, replay);
        let log = run(&trace, &allowlist);
        match &log.steps[4].decision {
            PolicyDecision::ReplayIgnored { action_id } => assert_eq!(action_id, "dry-1"),
            other => panic!("expected ReplayIgnored, got {other:?}"),
        }
        assert_eq!(log.summary.applies_simulated, 1);
        assert_eq!(log.summary.replays_ignored, 1);
    }

    #[test]
    fn preview_expires_after_30_s() {
        let allowlist = tested_allowlist();
        let mut trace = elevated_trace();
        // Offered at 2_000 (last Elevated step); confirm 30_001 ms later.
        let mut late = signaled(32_002);
        late.confirm = true;
        trace.push(late);
        let log = run(&trace, &allowlist);
        match no_action_reason(&log.steps[3].decision) {
            NoActionReason::PreviewExpired {
                offered_at_ms,
                confirmed_at_ms,
            } => {
                assert_eq!(*offered_at_ms, 2_000);
                assert_eq!(*confirmed_at_ms, 32_002);
            }
            reason => panic!("expected PreviewExpired, got {}", reason.code()),
        }
    }

    #[test]
    fn preview_validity_boundary_is_inclusive() {
        let allowlist = tested_allowlist();
        let mut trace = elevated_trace();
        // Offered at 2_000; confirm exactly 30 s later is still valid.
        let mut boundary = signaled(32_000);
        boundary.confirm = true;
        trace.push(boundary);
        let log = run(&trace, &allowlist);
        assert!(matches!(
            &log.steps[3].decision,
            PolicyDecision::ApplySimulated { .. }
        ));
    }

    #[test]
    fn eligibility_change_between_offer_and_confirm_blocks_apply() {
        let allowlist = tested_allowlist();
        let mut trace = elevated_trace();
        // The preview existed; at confirmation the target is foreground.
        let mut fg = signaled(2_500);
        fg.confirm = true;
        fg.target.focus = FocusObservation::Observed {
            target_foreground: true,
            observed_age_ms: 50,
        };
        trace.push(fg);
        let log = run(&trace, &allowlist);
        match no_action_reason(&log.steps[3].decision) {
            NoActionReason::Ineligible { denies } => {
                assert!(denies.iter().any(|d| d.code() == "target_foreground"));
            }
            reason => panic!("expected Ineligible, got {}", reason.code()),
        }
        assert_eq!(log.summary.applies_simulated, 0);
    }

    #[test]
    fn journal_unavailable_blocks_offers_and_confirms_but_not_recovery() {
        let allowlist = tested_allowlist();
        // Block the offer.
        let mut trace = elevated_trace();
        trace[2].journal_available = false;
        let log = run(&trace, &allowlist);
        assert_eq!(
            no_action_reason(&log.steps[2].decision).code(),
            "journal_unavailable"
        );
        // Offer first, then confirm without a journal.
        let mut trace2 = elevated_trace();
        trace2.push(signaled(2_500));
        let mut confirm = signaled(2_600);
        confirm.confirm = true;
        confirm.journal_available = false;
        trace2.push(confirm);
        let log2 = run(&trace2, &allowlist);
        assert_eq!(
            no_action_reason(&log2.steps[4].decision).code(),
            "journal_unavailable"
        );
        assert_eq!(log2.summary.applies_simulated, 0);
        // Recovery ignores journal availability: the TTL resolve proceeds.
        let mut trace3 = lifecycle_trace();
        let last = trace3.len() - 1;
        trace3[last].journal_available = false;
        let log3 = run(&trace3, &allowlist);
        assert!(matches!(
            &log3.steps[last].decision,
            PolicyDecision::ActionResolved { .. }
        ));
    }

    #[test]
    fn preview_reoffer_keeps_action_id_until_expired() {
        let allowlist = tested_allowlist();
        let mut trace = elevated_trace();
        trace.push(signaled(3_000)); // still Elevated, preview fresh -> dry-1
        trace.push(signaled(34_000)); // 31 s after last offer -> dry-2
        let log = run(&trace, &allowlist);
        let ids: Vec<String> = log
            .steps
            .iter()
            .filter_map(|s| match &s.decision {
                PolicyDecision::PreviewOffered { action_id, .. } => Some(action_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec!["dry-1", "dry-1", "dry-2"]);
    }

    #[test]
    fn resolved_experiment_can_be_re_offered_without_cooldown() {
        // RS-018 owns the 60 s cooldown; this pins the current boundary
        // honestly: after a resolve, a fresh preview CAN be offered.
        let allowlist = tested_allowlist();
        let mut trace = lifecycle_trace();
        trace.push(signaled(123_000)); // post-restore, still Elevated
        let log = run(&trace, &allowlist);
        assert!(matches!(
            &log.steps[5].decision,
            PolicyDecision::PreviewOffered { action_id, .. } if action_id == "dry-2"
        ));
    }

    #[test]
    fn log_serializes_with_schema_and_summary() {
        let allowlist = tested_allowlist();
        let log = run(&lifecycle_trace(), &allowlist);
        let json = serde_json::to_string(&log).expect("serialize log");
        assert!(json.contains(DRY_RUN_SCHEMA));
        assert!(json.contains("\"previews_offered\":1"));
        assert!(json.contains("\"pressure_rule_version\":\"pressure_v1\""));
        assert!(json.contains("ttl_expired"));
    }

    #[test]
    fn no_action_reason_codes_are_stable() {
        let codes = [
            NoActionReason::PressureUnknown.code(),
            NoActionReason::PressureNotElevated.code(),
            NoActionReason::Disabled.code(),
            NoActionReason::ActionActive {
                action_id: "x".to_string(),
            }
            .code(),
            NoActionReason::Ineligible { denies: vec![] }.code(),
            NoActionReason::NoValidPreview.code(),
            NoActionReason::PreviewExpired {
                offered_at_ms: 0,
                confirmed_at_ms: 1,
            }
            .code(),
            NoActionReason::JournalUnavailable.code(),
        ];
        assert_eq!(
            codes,
            [
                "pressure_unknown",
                "pressure_not_elevated",
                "disabled",
                "action_active",
                "ineligible",
                "no_valid_preview",
                "preview_expired",
                "journal_unavailable",
            ]
        );
    }
}

