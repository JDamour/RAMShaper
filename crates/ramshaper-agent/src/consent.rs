//! The G3 session-consent path (contract 14 §5; AGENTS.md rule 13).
//!
//! Startup is always Observe Only. Starting the agent with
//! `--policy-journal <path>` brings the policy gate **present but
//! disabled**: the restore-direction loop runs (startup reconciliation is
//! the contract-mandated safety direction), previews answer the honest
//! "policy is disabled for this session" refusal, and nothing can write
//! until the operator types the exact consent word on the agent console —
//! a per-session act that is never persisted, never derivable from
//! configuration, and never grantable over IPC (a reconnect does not
//! enable policy mode). A profile/import is not consent; neither is a
//! command-line flag: the flag only decides whether the gate exists at
//! all, and the typed word decides whether this session may apply.

use std::path::Path;

use ramshaper_core::eligibility::{Allowlist, ELIGIBILITY_RULES_VERSION};
use ramshaper_journal::JournalError;

use crate::policy::PolicyGate;

/// The exact word the operator must type to enable the policy session.
/// Strict: case-sensitive, alone on the line. Anything else — including a
/// lowercase variant, extra words, or end of input — leaves the session in
/// Observe Only.
pub const CONSENT_WORD: &str = "ENABLE";

/// Decide one line of console input. `None` is end of input (a
/// non-interactive or closed stdin): declined, never guessed.
#[must_use]
pub fn decide(input: Option<&str>) -> bool {
    input.map(str::trim) == Some(CONSENT_WORD)
}

/// Open the session's policy gate the way the shipped surface always does:
/// the compatibility allowlist is **empty** (nothing has passed RS-024
/// Stage B testing, so no target is eligible for applies until evidence
/// exists), and the user's configured exclusions can only reduce
/// eligibility further.
///
/// # Errors
/// [`JournalError`] when the journal cannot be opened — an unusable
/// journal blocks the policy stage (not the observe-only agent).
pub fn open_session_gate(
    journal_path: &Path,
    user_exclusions: Vec<String>,
) -> Result<PolicyGate, JournalError> {
    PolicyGate::open(
        journal_path,
        Allowlist::new(ELIGIBILITY_RULES_VERSION, &[]),
        user_exclusions,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_word_consents() {
        assert!(decide(Some("ENABLE")));
        assert!(
            decide(Some("  ENABLE  ")),
            "surrounding whitespace is input noise"
        );
    }

    #[test]
    fn everything_else_stays_observe_only() {
        assert!(
            !decide(Some("enable")),
            "the word is strict, not case-insensitive"
        );
        assert!(!decide(Some("ENABLE now")));
        assert!(!decide(Some("")));
        assert!(!decide(Some("YES")));
        assert!(!decide(None), "end of input is a decline, never a guess");
    }
}
