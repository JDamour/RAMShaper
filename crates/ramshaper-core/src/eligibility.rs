//! Eligibility matrix for the first policy action (RS-015, contract 14 §5).
//!
//! Pure decision logic, no OS calls: adapters observe reality and hand the
//! observations to [`evaluate`]. Every signal is an observation enum whose
//! `Unknown` variant **denies** (R-03: unknown targets cannot be changed;
//! an executable-name denylist must never replace this gate — doc 04).
//! The image name is a compatibility *label* only: it can match the tested
//! allowlist or a user exclusion, but never establishes identity or grants
//! authority. Authority comes from consent bound to the exact process
//! identity (RS-016); the allowlist only records *compatibility testing*.
//!
//! The initial allowlist is **empty** because no workload has passed
//! compatibility testing yet (RS-024 Stage B will add the first tested leaf
//! workload); with nothing tested, nothing is eligible — by design, not by
//! omission. Descendants and respawned processes are never enrolled
//! implicitly (R-10): a reused PID fails the identity check, and observed
//! children fail the leaf-workload check.

use serde::Serialize;

use crate::identity::{IdentityVerification, ProcessIdentity};

/// Maximum age of any observation used at apply time (contract 14 §3:
/// "apply telemetry freshness at most 2 s" — collect fresh input at
/// confirmation if background sampling is slower).
pub const APPLY_INPUT_FRESHNESS_MS: u64 = 2_000;

/// Version stamped into every report; consent records bind the rule version
/// they were evaluated under (contract 14 §5).
pub const ELIGIBILITY_RULES_VERSION: &str = "eligibility-v1";

/// Owner/session observation for the target (through the validated handle).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerSessionObservation {
    /// Runs as the current user in the current logon session.
    CurrentUserSession,
    /// Runs under a different user/session (evidence in `detail`).
    Different {
        /// What was observed (SID/session id detail), for audits.
        detail: String,
    },
    /// Owner or session could not be determined.
    Unknown {
        /// Why the observation failed.
        reason: String,
    },
}

/// Foreground observation. The adapter reports what it saw; freshness is
/// decided here against [`APPLY_INPUT_FRESHNESS_MS`] so the threshold logic
/// stays in one testable place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusObservation {
    /// A foreground-state observation of known age.
    Observed {
        /// True when the target itself is the foreground process.
        target_foreground: bool,
        /// Age of this observation, ms.
        observed_age_ms: u64,
    },
    /// No usable foreground observation.
    Unknown {
        /// Why the observation failed.
        reason: String,
    },
}

/// Categories that are always excluded from policy actions (contract 14 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedCategory {
    /// Audio paths (glitches are user-visible immediately).
    Audio,
    /// Security software (AV/EDR/identity protection).
    Security,
    /// Accessibility-critical assistive technology.
    AccessibilityCritical,
    /// Installers, updaters and package managers mid-operation.
    InstallUpdate,
    /// Games and anti-cheat (anti-cheat flags scheduling interference).
    GameAntiCheat,
    /// RAMShaper's own processes (self-exclusion).
    RamShaper,
    /// A known competing priority manager (doc 04).
    KnownIncompatiblePriorityManager,
}

impl ProtectedCategory {
    /// Stable machine code (IPC/audit).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Audio => "protected_audio",
            Self::Security => "protected_security",
            Self::AccessibilityCritical => "protected_accessibility",
            Self::InstallUpdate => "protected_install_update",
            Self::GameAntiCheat => "protected_game_anticheat",
            Self::RamShaper => "protected_ramshaper",
            Self::KnownIncompatiblePriorityManager => "protected_priority_manager",
        }
    }
}

// Serialized as the stable machine code, never the Rust variant name.
impl Serialize for ProtectedCategory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code())
    }
}

/// Safety classification from the (future, tested) signal matrix (R-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyClassification {
    /// Verified noncritical and nonprotected through tested signals.
    VerifiedNonCritical,
    /// Classified into an always-excluded category.
    Protected {
        /// Which category the signals matched.
        category: ProtectedCategory,
    },
    /// Signals unavailable or coverage absent for this target.
    Unknown {
        /// Why classification failed.
        reason: String,
    },
}

/// Observed current CPU priority class of the target. Only `Normal` is
/// actionable; other originals are skipped, never "corrected" (contract 14 §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorityClassObservation {
    /// Priority process class `NORMAL_PRIORITY_CLASS`.
    Normal,
    /// Any other class (detail names it; e.g. `HIGH` or `IDLE`).
    Other {
        /// What was observed.
        detail: String,
    },
    /// Class could not be read.
    Unknown {
        /// Why the read failed.
        reason: String,
    },
}

/// Live-children observation (R-10: leaf workloads only; descendants never
/// inherit consent, and an observed new child invalidates the experiment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildObservation {
    /// Observation completed; `live_children` is the count of live direct
    /// children at observation time.
    Observed { live_children: u32 },
    /// The process-table evidence is unavailable.
    Unknown {
        /// Why the observation failed.
        reason: String,
    },
}

/// Whether the actuation API can change this target's priority class
/// ("unknown API capability means deny" — contract 14 §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorityCapability {
    /// Adapter reports the capability as available for this target.
    Supported,
    /// Adapter reports it as unavailable.
    Unsupported,
    /// Capability could not be determined.
    Unknown {
        /// Why the probe failed.
        reason: String,
    },
}

/// The tested leaf-workload compatibility allowlist. Entries are bare image
/// names of workloads that **passed compatibility testing** (RS-024 Stage B
/// evidence); membership is a compatibility classification, not authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowlist {
    rules_version: String,
    names: Vec<String>,
}

impl Allowlist {
    /// Build an allowlist (bare file names; callers own validation — user
    /// config can never populate this, only tested evidence can).
    #[must_use]
    pub fn new(rules_version: &str, names: &[&str]) -> Self {
        Self {
            rules_version: rules_version.to_string(),
            names: names.iter().map(|n| (*n).to_string()).collect(),
        }
    }

    /// The initial allowlist: versioned, empty — nothing has been tested.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(ELIGIBILITY_RULES_VERSION, &[])
    }

    /// Case-insensitive membership (Windows image names).
    #[must_use]
    pub fn contains(&self, image_name: &str) -> bool {
        self.names
            .iter()
            .any(|n| n.eq_ignore_ascii_case(image_name))
    }

    /// Rule version this allowlist was tested under.
    #[must_use]
    pub fn rules_version(&self) -> &str {
        &self.rules_version
    }

    /// Number of tested entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// True when no workload has been tested.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Everything [`evaluate`] needs, as observations. Adapters fill this from
/// OS queries at (re)evaluation time; tests fill it from fixtures.
#[derive(Debug, Clone)]
pub struct EligibilityInputs<'a> {
    /// The exact consent-bound identity being evaluated.
    pub identity: &'a ProcessIdentity,
    /// Target image file name (label only — never identity or authority).
    pub image_name: &'a str,
    /// Fresh re-check of [`Self::identity`] against current OS state.
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
    /// Actuation capability for this target.
    pub capability: PriorityCapability,
    /// The tested compatibility allowlist.
    pub allowlist: &'a Allowlist,
    /// User exclusion names from the loaded config (RS-012).
    pub user_exclusions: &'a [String],
    /// Count of outstanding (unresolved) actions/conflicts.
    pub outstanding_actions: usize,
    /// Whether the action journal is open and writable.
    pub journal_available: bool,
}

/// One named eligibility check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckId {
    /// Exact identity still matches (PID reuse denies).
    Identity,
    /// Current user/session.
    OwnerSession,
    /// Fresh foreground observation, target not foreground.
    Focus,
    /// Image on the tested compatibility allowlist.
    CompatibilityAllowlist,
    /// Verified noncritical/nonprotected classification.
    Classification,
    /// Not excluded by the user.
    UserExclusion,
    /// Original priority class is Normal.
    OriginalPriority,
    /// No live children (leaf workload; R-10).
    LeafWorkload,
    /// No outstanding action or conflict.
    NoOutstandingAction,
    /// Action journal available.
    JournalAvailable,
    /// Priority API capability known and supported.
    PriorityApiCapability,
}

impl CheckId {
    /// All checks, in the documented evaluation order (contract 14 §5).
    #[must_use]
    pub const fn all() -> [CheckId; 11] {
        [
            CheckId::Identity,
            CheckId::OwnerSession,
            CheckId::Focus,
            CheckId::CompatibilityAllowlist,
            CheckId::Classification,
            CheckId::UserExclusion,
            CheckId::OriginalPriority,
            CheckId::LeafWorkload,
            CheckId::NoOutstandingAction,
            CheckId::JournalAvailable,
            CheckId::PriorityApiCapability,
        ]
    }

    /// Stable machine code (IPC/audit).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::OwnerSession => "owner_session",
            Self::Focus => "focus",
            Self::CompatibilityAllowlist => "compatibility_allowlist",
            Self::Classification => "classification",
            Self::UserExclusion => "user_exclusion",
            Self::OriginalPriority => "original_priority",
            Self::LeafWorkload => "leaf_workload",
            Self::NoOutstandingAction => "no_outstanding_action",
            Self::JournalAvailable => "journal_available",
            Self::PriorityApiCapability => "priority_api_capability",
        }
    }
}

// Serialized as the stable machine code, never the Rust variant name.
impl Serialize for CheckId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code())
    }
}

/// Why one check denied, with the evidence that was seen. Machine codes are
/// stable strings for IPC/UI (RS-013 explanations, `Ineligible` details).
#[derive(Debug, Clone, PartialEq)]
pub enum DenyReason {
    /// PID now belongs to a different instance; the replacement never
    /// inherits history or authority.
    IdentityReused {
        /// What the PID currently resolves to.
        observed: ProcessIdentity,
    },
    /// Identity could not be re-queried (exited, denied, OS error).
    IdentityUnreachable {
        /// Machine-readable failure code.
        code: String,
    },
    /// Target runs under a different user/session.
    OwnerSessionDifferent { detail: String },
    /// Owner/session undetermined.
    OwnerSessionUnknown { reason: String },
    /// Target is the foreground process (fresh observation).
    TargetForeground { observed_age_ms: u64 },
    /// Foreground observation older than the apply-freshness window.
    FocusObservationStale {
        observed_age_ms: u64,
        max_age_ms: u64,
    },
    /// No usable foreground observation.
    FocusUnknown { reason: String },
    /// Classified into an always-excluded category.
    ProtectedCategory {
        /// Which category.
        category: ProtectedCategory,
    },
    /// Classification undetermined.
    ClassificationUnknown { reason: String },
    /// Image has not passed compatibility testing.
    NotOnTestedAllowlist,
    /// Image matches a user exclusion.
    ExcludedByUser { matched_name: String },
    /// Original priority class is not Normal (skipped, not corrected).
    OriginalPriorityNotNormal { detail: String },
    /// Original priority class unreadable.
    OriginalPriorityUnknown { reason: String },
    /// Live children observed; only leaf workloads are actionable (R-10).
    ChildrenObserved { count: u32 },
    /// Child state undetermined.
    ChildStateUnknown { reason: String },
    /// Another action/conflict is still outstanding.
    OutstandingActionExists { count: usize },
    /// Journal unavailable — no durable intent possible.
    JournalUnavailable,
    /// Adapter reports the priority API as unavailable.
    PriorityApiUnsupported,
    /// Priority capability undetermined.
    PriorityApiUnknown { reason: String },
}

impl DenyReason {
    /// Stable machine code (IPC/audit).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::IdentityReused { .. } => "identity_reused",
            Self::IdentityUnreachable { .. } => "identity_unreachable",
            Self::OwnerSessionDifferent { .. } => "owner_session_different",
            Self::OwnerSessionUnknown { .. } => "owner_session_unknown",
            Self::TargetForeground { .. } => "target_foreground",
            Self::FocusObservationStale { .. } => "focus_observation_stale",
            Self::FocusUnknown { .. } => "focus_unknown",
            Self::ProtectedCategory { .. } => "protected_category",
            Self::ClassificationUnknown { .. } => "classification_unknown",
            Self::NotOnTestedAllowlist => "not_on_tested_allowlist",
            Self::ExcludedByUser { .. } => "excluded_by_user",
            Self::OriginalPriorityNotNormal { .. } => "original_priority_not_normal",
            Self::OriginalPriorityUnknown { .. } => "original_priority_unknown",
            Self::ChildrenObserved { .. } => "children_observed",
            Self::ChildStateUnknown { .. } => "child_state_unknown",
            Self::OutstandingActionExists { .. } => "outstanding_action_exists",
            Self::JournalUnavailable => "journal_unavailable",
            Self::PriorityApiUnsupported => "priority_api_unsupported",
            Self::PriorityApiUnknown { .. } => "priority_api_unknown",
        }
    }
}

impl std::fmt::Display for DenyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code())?;
        match self {
            Self::IdentityReused { observed } => write!(f, " ({observed})"),
            Self::IdentityUnreachable { code } => write!(f, " ({code})"),
            Self::OwnerSessionDifferent { detail }
            | Self::OwnerSessionUnknown { reason: detail, .. }
            | Self::ClassificationUnknown { reason: detail, .. }
            | Self::FocusUnknown { reason: detail, .. }
            | Self::OriginalPriorityNotNormal { detail, .. }
            | Self::OriginalPriorityUnknown { reason: detail, .. }
            | Self::ChildStateUnknown { reason: detail, .. }
            | Self::PriorityApiUnknown { reason: detail, .. } => write!(f, " ({detail})"),
            Self::TargetForeground { observed_age_ms } => {
                write!(
                    f,
                    " (target foreground, observation {observed_age_ms} ms old)"
                )
            }
            Self::FocusObservationStale {
                observed_age_ms,
                max_age_ms,
            } => write!(
                f,
                " (observation {observed_age_ms} ms old exceeds {max_age_ms} ms)"
            ),
            Self::ProtectedCategory { category } => write!(f, " ({})", category.code()),
            Self::NotOnTestedAllowlist => Ok(()),
            Self::ExcludedByUser { matched_name } => write!(f, " ({matched_name})"),
            Self::ChildrenObserved { count } => write!(f, " ({count} live children)"),
            Self::OutstandingActionExists { count } => write!(f, " ({count} outstanding)"),
            Self::JournalUnavailable | Self::PriorityApiUnsupported => Ok(()),
        }
    }
}

// Serialized as `{code, message}` with the stable machine code, never the
// Rust variant names — UI/IPC consumers branch on `code`.
impl Serialize for DenyReason {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("DenyReason", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

/// Outcome of one check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Outcome {
    /// Check passed.
    Pass,
    /// Check denied, with evidence.
    Deny {
        /// Why.
        reason: DenyReason,
    },
}

/// One evaluated check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckResult {
    /// Which check.
    pub id: CheckId,
    /// Its outcome.
    pub outcome: Outcome,
}

/// Full eligibility evaluation: every check is evaluated and reported (the
/// UI explains all failures, not just the first — RS-011/RS-013).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EligibilityReport {
    /// Rules version this evaluation ran under.
    pub rules_version: String,
    /// Every check, in documented order.
    pub checks: Vec<CheckResult>,
    /// True only when every check passed.
    pub eligible: bool,
}

impl EligibilityReport {
    /// First deny in documented order, if any.
    #[must_use]
    pub fn first_deny(&self) -> Option<&DenyReason> {
        self.checks.iter().find_map(|c| match &c.outcome {
            Outcome::Pass => None,
            Outcome::Deny { reason } => Some(reason),
        })
    }

    /// All deny reasons.
    #[must_use]
    pub fn deny_reasons(&self) -> Vec<&DenyReason> {
        self.checks
            .iter()
            .filter_map(|c| match &c.outcome {
                Outcome::Pass => None,
                Outcome::Deny { reason } => Some(reason),
            })
            .collect()
    }
}

impl std::fmt::Display for EligibilityReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let denies = self.deny_reasons();
        if denies.is_empty() {
            write!(
                f,
                "eligible (all {} checks passed, rules {})",
                self.checks.len(),
                self.rules_version
            )
        } else {
            write!(f, "not eligible ({}):", denies.len())?;
            for reason in denies {
                write!(f, " {reason};")?;
            }
            Ok(())
        }
    }
}

fn identity_outcome(check: &IdentityVerification) -> Outcome {
    match check {
        IdentityVerification::Matches => Outcome::Pass,
        IdentityVerification::Mismatch { observed } => Outcome::Deny {
            reason: DenyReason::IdentityReused {
                observed: *observed,
            },
        },
        IdentityVerification::Unreachable { reason } => Outcome::Deny {
            reason: DenyReason::IdentityUnreachable {
                code: reason.error_code(),
            },
        },
    }
}

fn owner_session_outcome(observation: &OwnerSessionObservation) -> Outcome {
    match observation {
        OwnerSessionObservation::CurrentUserSession => Outcome::Pass,
        OwnerSessionObservation::Different { detail } => Outcome::Deny {
            reason: DenyReason::OwnerSessionDifferent {
                detail: detail.clone(),
            },
        },
        OwnerSessionObservation::Unknown { reason } => Outcome::Deny {
            reason: DenyReason::OwnerSessionUnknown {
                reason: reason.clone(),
            },
        },
    }
}

fn focus_outcome(observation: &FocusObservation) -> Outcome {
    match observation {
        FocusObservation::Observed {
            target_foreground: false,
            observed_age_ms,
        } if *observed_age_ms <= APPLY_INPUT_FRESHNESS_MS => Outcome::Pass,
        FocusObservation::Observed {
            target_foreground: false,
            observed_age_ms,
        } => Outcome::Deny {
            reason: DenyReason::FocusObservationStale {
                observed_age_ms: *observed_age_ms,
                max_age_ms: APPLY_INPUT_FRESHNESS_MS,
            },
        },
        FocusObservation::Observed {
            target_foreground: true,
            observed_age_ms,
        } => Outcome::Deny {
            reason: DenyReason::TargetForeground {
                observed_age_ms: *observed_age_ms,
            },
        },
        FocusObservation::Unknown { reason } => Outcome::Deny {
            reason: DenyReason::FocusUnknown {
                reason: reason.clone(),
            },
        },
    }
}

fn classification_outcome(classification: &SafetyClassification) -> Outcome {
    match classification {
        SafetyClassification::VerifiedNonCritical => Outcome::Pass,
        SafetyClassification::Protected { category } => Outcome::Deny {
            reason: DenyReason::ProtectedCategory {
                category: *category,
            },
        },
        SafetyClassification::Unknown { reason } => Outcome::Deny {
            reason: DenyReason::ClassificationUnknown {
                reason: reason.clone(),
            },
        },
    }
}

fn priority_outcome(priority: &PriorityClassObservation) -> Outcome {
    match priority {
        PriorityClassObservation::Normal => Outcome::Pass,
        PriorityClassObservation::Other { detail } => Outcome::Deny {
            reason: DenyReason::OriginalPriorityNotNormal {
                detail: detail.clone(),
            },
        },
        PriorityClassObservation::Unknown { reason } => Outcome::Deny {
            reason: DenyReason::OriginalPriorityUnknown {
                reason: reason.clone(),
            },
        },
    }
}

fn children_outcome(children: &ChildObservation) -> Outcome {
    match children {
        ChildObservation::Observed { live_children: 0 } => Outcome::Pass,
        ChildObservation::Observed { live_children } => Outcome::Deny {
            reason: DenyReason::ChildrenObserved {
                count: *live_children,
            },
        },
        ChildObservation::Unknown { reason } => Outcome::Deny {
            reason: DenyReason::ChildStateUnknown {
                reason: reason.clone(),
            },
        },
    }
}

fn capability_outcome(capability: &PriorityCapability) -> Outcome {
    match capability {
        PriorityCapability::Supported => Outcome::Pass,
        PriorityCapability::Unsupported => Outcome::Deny {
            reason: DenyReason::PriorityApiUnsupported,
        },
        PriorityCapability::Unknown { reason } => Outcome::Deny {
            reason: DenyReason::PriorityApiUnknown {
                reason: reason.clone(),
            },
        },
    }
}

/// Evaluate every eligibility check (contract 14 §5 apply-time list plus the
/// identity and leaf-workload gates). Pure; unknown always denies.
#[must_use]
pub fn evaluate(inputs: &EligibilityInputs<'_>) -> EligibilityReport {
    let checks = vec![
        CheckResult {
            id: CheckId::Identity,
            outcome: identity_outcome(&inputs.identity_check),
        },
        CheckResult {
            id: CheckId::OwnerSession,
            outcome: owner_session_outcome(&inputs.owner_session),
        },
        CheckResult {
            id: CheckId::Focus,
            outcome: focus_outcome(&inputs.focus),
        },
        CheckResult {
            id: CheckId::CompatibilityAllowlist,
            outcome: if inputs.allowlist.contains(inputs.image_name) {
                Outcome::Pass
            } else {
                Outcome::Deny {
                    reason: DenyReason::NotOnTestedAllowlist,
                }
            },
        },
        CheckResult {
            id: CheckId::Classification,
            outcome: classification_outcome(&inputs.classification),
        },
        CheckResult {
            id: CheckId::UserExclusion,
            outcome: {
                let matched = inputs
                    .user_exclusions
                    .iter()
                    .find(|name| name.eq_ignore_ascii_case(inputs.image_name));
                match matched {
                    None => Outcome::Pass,
                    Some(name) => Outcome::Deny {
                        reason: DenyReason::ExcludedByUser {
                            matched_name: name.clone(),
                        },
                    },
                }
            },
        },
        CheckResult {
            id: CheckId::OriginalPriority,
            outcome: priority_outcome(&inputs.priority_class),
        },
        CheckResult {
            id: CheckId::LeafWorkload,
            outcome: children_outcome(&inputs.children),
        },
        CheckResult {
            id: CheckId::NoOutstandingAction,
            outcome: if inputs.outstanding_actions == 0 {
                Outcome::Pass
            } else {
                Outcome::Deny {
                    reason: DenyReason::OutstandingActionExists {
                        count: inputs.outstanding_actions,
                    },
                }
            },
        },
        CheckResult {
            id: CheckId::JournalAvailable,
            outcome: if inputs.journal_available {
                Outcome::Pass
            } else {
                Outcome::Deny {
                    reason: DenyReason::JournalUnavailable,
                }
            },
        },
        CheckResult {
            id: CheckId::PriorityApiCapability,
            outcome: capability_outcome(&inputs.capability),
        },
    ];
    let eligible = checks.iter().all(|c| matches!(c.outcome, Outcome::Pass));
    EligibilityReport {
        rules_version: inputs.allowlist.rules_version().to_string(),
        eligible,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::QueryFailure;

    fn target() -> ProcessIdentity {
        ProcessIdentity::new(1234, 133_000_000_000_000_000)
    }

    fn tested_allowlist() -> Allowlist {
        Allowlist::new(ELIGIBILITY_RULES_VERSION, &["leafapp.exe"])
    }

    /// Inputs where every check passes (target on the tested allowlist).
    fn passing_inputs<'a>(
        identity: &'a ProcessIdentity,
        allowlist: &'a Allowlist,
        exclusions: &'a [String],
    ) -> EligibilityInputs<'a> {
        EligibilityInputs {
            identity,
            image_name: "leafapp.exe",
            identity_check: IdentityVerification::Matches,
            owner_session: OwnerSessionObservation::CurrentUserSession,
            focus: FocusObservation::Observed {
                target_foreground: false,
                observed_age_ms: 500,
            },
            classification: SafetyClassification::VerifiedNonCritical,
            priority_class: PriorityClassObservation::Normal,
            children: ChildObservation::Observed { live_children: 0 },
            capability: PriorityCapability::Supported,
            allowlist,
            user_exclusions: exclusions,
            outstanding_actions: 0,
            journal_available: true,
        }
    }

    #[test]
    fn all_checks_pass_yields_eligible() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let report = evaluate(&passing_inputs(&id, &allowlist, &exclusions));
        assert!(report.eligible);
        assert_eq!(report.first_deny(), None);
        // Every documented check is reported exactly once, in order.
        let ids: Vec<CheckId> = report.checks.iter().map(|c| c.id).collect();
        assert_eq!(ids, CheckId::all().to_vec());
        assert_eq!(report.rules_version, ELIGIBILITY_RULES_VERSION);
    }

    #[test]
    fn empty_allowlist_denies_everything_by_design() {
        // R-03: initial state — no workload has passed compatibility testing,
        // so nothing is eligible even when every other check passes.
        let id = target();
        let allowlist = Allowlist::empty();
        assert!(allowlist.is_empty());
        let exclusions: Vec<String> = vec![];
        let report = evaluate(&passing_inputs(&id, &allowlist, &exclusions));
        assert!(!report.eligible);
        let reasons = report.deny_reasons();
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0].code(), "not_on_tested_allowlist");
    }

    #[test]
    fn pid_reuse_denies_and_never_inherits() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.identity_check = IdentityVerification::Mismatch {
            observed: ProcessIdentity::new(1234, 133_000_000_000_000_999),
        };
        let report = evaluate(&inputs);
        let reason = report.first_deny().expect("must deny");
        assert_eq!(reason.code(), "identity_reused");
        assert!(
            format!("{reason}").contains("999"),
            "audit text carries the replacement identity"
        );
    }

    #[test]
    fn identity_unreachable_denies_with_machine_code() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.identity_check = IdentityVerification::Unreachable {
            reason: QueryFailure::Exited { code: 87 },
        };
        let report = evaluate(&inputs);
        let reason = report.first_deny().expect("must deny");
        assert_eq!(reason.code(), "identity_unreachable");
        assert!(format!("{reason}").contains("exited_87"));
    }

    #[test]
    fn different_owner_session_denies() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.owner_session = OwnerSessionObservation::Different {
            detail: "session 2 (SERVICE)".to_string(),
        };
        let report = evaluate(&inputs);
        assert_eq!(
            report.first_deny().expect("deny").code(),
            "owner_session_different"
        );
    }

    #[test]
    fn unknown_owner_session_denies() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.owner_session = OwnerSessionObservation::Unknown {
            reason: "token query denied".to_string(),
        };
        let report = evaluate(&inputs);
        assert_eq!(
            report.first_deny().expect("deny").code(),
            "owner_session_unknown"
        );
    }

    #[test]
    fn foreground_target_denies_even_when_fresh() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.focus = FocusObservation::Observed {
            target_foreground: true,
            observed_age_ms: 100,
        };
        let report = evaluate(&inputs);
        assert_eq!(
            report.first_deny().expect("deny").code(),
            "target_foreground"
        );
    }

    #[test]
    fn focus_freshness_boundary_is_inclusive() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.focus = FocusObservation::Observed {
            target_foreground: false,
            observed_age_ms: APPLY_INPUT_FRESHNESS_MS,
        };
        assert!(evaluate(&inputs).eligible, "age == bound is fresh");
        inputs.focus = FocusObservation::Observed {
            target_foreground: false,
            observed_age_ms: APPLY_INPUT_FRESHNESS_MS + 1,
        };
        let report = evaluate(&inputs);
        let reason = report.first_deny().expect("must deny");
        assert_eq!(reason.code(), "focus_observation_stale");
        assert!(format!("{reason}").contains("2001 ms"));
    }

    #[test]
    fn unknown_focus_denies() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.focus = FocusObservation::Unknown {
            reason: "no foreground window".to_string(),
        };
        let report = evaluate(&inputs);
        assert_eq!(report.first_deny().expect("deny").code(), "focus_unknown");
    }

    #[test]
    fn every_protected_category_denies() {
        let categories = [
            ProtectedCategory::Audio,
            ProtectedCategory::Security,
            ProtectedCategory::AccessibilityCritical,
            ProtectedCategory::InstallUpdate,
            ProtectedCategory::GameAntiCheat,
            ProtectedCategory::RamShaper,
            ProtectedCategory::KnownIncompatiblePriorityManager,
        ];
        for category in categories {
            let id = target();
            let allowlist = tested_allowlist();
            let exclusions: Vec<String> = vec![];
            let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
            inputs.classification = SafetyClassification::Protected { category };
            let report = evaluate(&inputs);
            assert!(!report.eligible, "{category:?} must deny");
            let reason = report.first_deny().expect("deny");
            assert_eq!(reason.code(), "protected_category");
            assert!(format!("{reason}").contains(category.code()));
        }
    }

    #[test]
    fn unknown_classification_denies() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.classification = SafetyClassification::Unknown {
            reason: "signal coverage absent".to_string(),
        };
        let report = evaluate(&inputs);
        assert_eq!(
            report.first_deny().expect("deny").code(),
            "classification_unknown"
        );
    }

    #[test]
    fn user_exclusion_matches_case_insensitively() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions = vec!["LeafApp.EXE".to_string()];
        let inputs = passing_inputs(&id, &allowlist, &exclusions);
        let report = evaluate(&inputs);
        let reason = report.first_deny().expect("must deny");
        assert_eq!(reason.code(), "excluded_by_user");
        assert!(format!("{reason}").contains("LeafApp.EXE"));
    }

    #[test]
    fn non_normal_original_priority_denies() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.priority_class = PriorityClassObservation::Other {
            detail: "HIGH_PRIORITY_CLASS".to_string(),
        };
        let report = evaluate(&inputs);
        let reason = report.first_deny().expect("deny");
        assert_eq!(reason.code(), "original_priority_not_normal");
        assert!(format!("{reason}").contains("HIGH_PRIORITY_CLASS"));
    }

    #[test]
    fn unknown_original_priority_denies() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.priority_class = PriorityClassObservation::Unknown {
            reason: "read denied".to_string(),
        };
        let report = evaluate(&inputs);
        assert_eq!(
            report.first_deny().expect("deny").code(),
            "original_priority_unknown"
        );
    }

    #[test]
    fn observed_children_deny_leaf_gate() {
        // R-10: descendants never inherit consent; a spawning parent is not
        // a leaf workload at apply time.
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.children = ChildObservation::Observed { live_children: 2 };
        let report = evaluate(&inputs);
        let reason = report.first_deny().expect("deny");
        assert_eq!(reason.code(), "children_observed");
        assert!(format!("{reason}").contains("2 live children"));
    }

    #[test]
    fn unknown_child_state_denies() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.children = ChildObservation::Unknown {
            reason: "table scan failed".to_string(),
        };
        let report = evaluate(&inputs);
        assert_eq!(
            report.first_deny().expect("deny").code(),
            "child_state_unknown"
        );
    }

    #[test]
    fn outstanding_action_denies() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.outstanding_actions = 1;
        let report = evaluate(&inputs);
        let reason = report.first_deny().expect("deny");
        assert_eq!(reason.code(), "outstanding_action_exists");
        assert!(format!("{reason}").contains("1 outstanding"));
    }

    #[test]
    fn unavailable_journal_denies() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.journal_available = false;
        let report = evaluate(&inputs);
        assert_eq!(
            report.first_deny().expect("deny").code(),
            "journal_unavailable"
        );
    }

    #[test]
    fn unsupported_and_unknown_capability_deny() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.capability = PriorityCapability::Unsupported;
        let report = evaluate(&inputs);
        assert_eq!(
            report.first_deny().expect("deny").code(),
            "priority_api_unsupported"
        );
        inputs.capability = PriorityCapability::Unknown {
            reason: "probe timed out".to_string(),
        };
        let report = evaluate(&inputs);
        assert_eq!(
            report.first_deny().expect("deny").code(),
            "priority_api_unknown"
        );
    }

    #[test]
    fn multiple_failures_are_all_reported_in_order() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions = vec!["leafapp.exe".to_string()];
        let mut inputs = passing_inputs(&id, &allowlist, &exclusions);
        inputs.journal_available = false;
        inputs.children = ChildObservation::Observed { live_children: 1 };
        let report = evaluate(&inputs);
        assert!(!report.eligible);
        let codes: Vec<&str> = report.deny_reasons().iter().map(|r| r.code()).collect();
        // Documented order: user exclusion, leaf workload, journal.
        assert_eq!(
            codes,
            vec![
                "excluded_by_user",
                "children_observed",
                "journal_unavailable"
            ]
        );
        // first_deny follows the same documented order.
        assert_eq!(
            report.first_deny().expect("deny").code(),
            "excluded_by_user"
        );
        let text = format!("{report}");
        assert!(text.starts_with("not eligible (3):"));
        assert!(text.contains("excluded_by_user"));
    }

    #[test]
    fn allowlist_membership_is_case_insensitive_and_versioned() {
        let allowlist = Allowlist::new("eligibility-v1-fixture", &["LeafApp.exe"]);
        assert!(allowlist.contains("leafapp.exe"));
        assert!(allowlist.contains("LEAFAPP.EXE"));
        assert!(!allowlist.contains("other.exe"));
        assert_eq!(allowlist.len(), 1);
        assert_eq!(allowlist.rules_version(), "eligibility-v1-fixture");
        // The report carries the allowlist's own version, not the constant.
        let id = target();
        let exclusions: Vec<String> = vec![];
        let report = evaluate(&passing_inputs(&id, &allowlist, &exclusions));
        assert!(report.eligible);
        assert_eq!(report.rules_version, "eligibility-v1-fixture");
    }

    #[test]
    fn report_serializes_for_ipc_and_ui() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions = vec!["leafapp.exe".to_string()];
        let report = evaluate(&passing_inputs(&id, &allowlist, &exclusions));
        let json = serde_json::to_string(&report).expect("serialize report");
        assert!(json.contains("\"eligible\":false"));
        assert!(
            json.contains("\"compatibility_allowlist\"") || json.contains("CompatibilityAllowlist")
        );
        assert!(json.contains("excluded_by_user"));
    }

    #[test]
    fn display_of_eligible_report_names_all_checks() {
        let id = target();
        let allowlist = tested_allowlist();
        let exclusions: Vec<String> = vec![];
        let report = evaluate(&passing_inputs(&id, &allowlist, &exclusions));
        let text = format!("{report}");
        assert!(text.starts_with("eligible (all 11 checks passed"));
        assert!(text.contains(ELIGIBILITY_RULES_VERSION));
    }
}
