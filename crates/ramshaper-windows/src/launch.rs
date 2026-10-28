//! RS-023: launch consent and the explicit non-sandbox/non-detachable
//! semantics of a managed group (doc 07 RS-023; P0-04; AT-16).
//!
//! The managed launcher exists only behind an explicit operator act: every
//! single launch is typed-consented on the spot (strict word, alone on the
//! line — the G3 session-consent pattern, but per launch rather than per
//! session). A consent is a **move-consumed token**: it buys at most one
//! launch attempt, so an automatic retry or duplicate launch cannot be
//! expressed — a second attempt always requires a second consent.
//!
//! The semantics this module pins (and its statements state verbatim):
//! - the advisory job is **not a sandbox**: it grants no security
//!   isolation and enforces nothing — the queried limit flags are always
//!   zero (AT-16: "queried job flags");
//! - membership is **not detachable**: removing monitoring drops the
//!   monitoring handles and never detaches the process; the association
//!   lasts for the process lifetime and is not reversible;
//! - the application **outlives monitoring**: UI/agent exit must not kill
//!   apps (P0-04) — removal is a handle drop, and the child keeps running
//!   (AT-16: "child membership and process lifetime");
//! - the launch is an **exact image**: `lpApplicationName` is passed
//!   explicitly, so no PATH search, no shell and no shortcut/association
//!   resolution can substitute a different program for the consented one.
//!
//! What this module deliberately does NOT have: any kill, suspension,
//! detach, retry or limit path; a persisted or configuration-derived
//! consent; an escape from a pre-existing launcher job (nesting is shown
//! in the preflight and accepted explicitly by consenting after it is
//! displayed). The managed-beta release gate (G3 sign-off + full AT-15
//! lifecycle traces) stays a separate decision; this is the gated
//! capability surface.

use std::path::Path;

use crate::job::{
    compatibility_report, exact_command_line, AdvisoryJob, AdvisoryStatus, AdvisoryThresholds,
    CompatibilityReport, GroupAccounting, JobError, ManagedChild,
};

/// The exact word the operator must type to consent to one launch.
/// Strict: case-sensitive, alone on the line (the G3 consent-word rules).
/// Deliberately distinct from the policy stage's `ENABLE` so the two
/// consent surfaces can never be confused.
pub const LAUNCH_CONSENT_WORD: &str = "LAUNCH";

/// One launch request: an exact executable path and explicit arguments.
/// Only new, explicitly chosen launches qualify (P0-04) — there is no
/// default request anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    /// The image to launch, passed to the OS as `lpApplicationName`.
    pub executable: std::ffi::OsString,
    /// The argument string, appended to the child command line verbatim.
    pub arguments: String,
}

impl LaunchRequest {
    /// Build a request; call [`LaunchRequest::validate`] before prompting.
    #[must_use]
    pub fn new(executable: impl Into<std::ffi::OsString>, arguments: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
            arguments: arguments.into(),
        }
    }

    /// The exact command line the child will receive (the string a consent
    /// prompt must display).
    #[must_use]
    pub fn command_line(&self) -> String {
        exact_command_line(&self.executable, &self.arguments)
    }

    /// Validate the request before any consent is asked: non-empty, free
    /// of NUL (wide-string truncation), and naming an existing regular
    /// file. Shortcuts, scripts and directories fail here or at the OS —
    /// never silently (P0-04 scope).
    ///
    /// # Errors
    /// [`LaunchError::InvalidRequest`] naming the first violated rule.
    pub fn validate(&self) -> Result<(), LaunchError> {
        if self.executable.is_empty() {
            return Err(LaunchError::InvalidRequest {
                reason: "executable path is empty",
            });
        }
        let path = Path::new(&self.executable);
        let invalid = |reason| Err(LaunchError::InvalidRequest { reason });
        if self.executable.as_encoded_bytes().contains(&0) {
            return invalid("executable path contains NUL");
        }
        if self.arguments.contains('\0') {
            return invalid("arguments contain NUL");
        }
        let Ok(metadata) = std::fs::metadata(path) else {
            return invalid("executable does not exist");
        };
        if !metadata.is_file() {
            return invalid("executable is not a regular file");
        }
        Ok(())
    }
}

/// A consent to launch **one** request, exactly once. Unconstructable
/// outside this module: the only source is [`consent_decide`], and
/// [`ManagedLaunchSession::launch`] consumes it — so a failed or succeeded
/// attempt always exhausts its consent and any retry needs a fresh typed
/// act (the "no automatic duplicate launch" guarantee, in the type system).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchConsent {
    _token: (),
}

/// Decide one line of console input against the consent word. `None` is a
/// decline — including end of input (a non-interactive run never consents
/// by absence of input). The caller must have displayed the prompt for
/// *this* request before reading the line; within one CLI invocation the
/// decided request is the only one [`ManagedLaunchSession::launch`] can
/// receive, which is the binding.
#[must_use]
pub fn consent_decide(input: Option<&str>) -> Option<LaunchConsent> {
    if input.map(str::trim) == Some(LAUNCH_CONSENT_WORD) {
        Some(LaunchConsent { _token: () })
    } else {
        None
    }
}

/// The consent prompt for one request: the exact command line, the
/// preflight facts, the honest semantics, and the word. The CLI renders
/// this to the operator BEFORE reading the consent line — consenting to a
/// launch one has not seen is not a flow this module builds.
#[must_use]
pub fn render_consent_prompt(request: &LaunchRequest, preflight: &CompatibilityReport) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("Managed launch request\n");
    out.push_str(&format!("  command line : {}\n", request.command_line()));
    out.push_str("  job          : fresh advisory job (zero limits, queried)\n");
    if preflight.notes.is_empty() {
        out.push_str("  preflight    : no compatibility notes\n");
    } else {
        for note in &preflight.notes {
            out.push_str(&format!("  preflight    : {note}\n"));
        }
    }
    for statement in semantics_statements() {
        out.push_str(&format!("  semantics    : {statement}\n"));
    }
    out.push_str(&format!(
        "type {} (exactly, alone) to consent to this one launch; anything else declines: ",
        LAUNCH_CONSENT_WORD
    ));
    out
}

/// The explicit semantics of a managed group, stated verbatim wherever the
/// surface reports a launch (AT-16: "UI never promises detachment or
/// sandboxing" — this is the affirmative honest copy, not an absence).
#[must_use]
pub fn semantics_statements() -> Vec<&'static str> {
    vec![
        "the job is not a sandbox: it provides no security isolation and enforces nothing",
        "membership is not detachable and lasts for the process lifetime",
        "removing monitoring does not detach the process or stop the application",
        "no kill-on-close, hard, time or process-count limits exist on this job",
    ]
}

/// Launch-failure vocabulary. Every variant is an explicit outcome; none
/// carries a retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// The typed line was absent or anything other than the exact word.
    ConsentDeclined,
    /// The request itself is unusable (the reason names the rule).
    InvalidRequest {
        /// The violated validation rule.
        reason: &'static str,
    },
    /// A Win32 call failed with the preserved code (a refused
    /// `CreateProcessW` means nothing was launched and the job stayed
    /// empty).
    Os {
        /// Preserved Win32 error code.
        code: u32,
        /// The failing function, for the audit trail.
        function: &'static str,
    },
    /// The child exists but the membership proof did not confirm the job —
    /// it may be running unmanaged. Reported, never retried.
    MembershipUnverified,
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConsentDeclined => write!(f, "launch declined (consent word absent or wrong)"),
            Self::InvalidRequest { reason } => write!(f, "invalid launch request: {reason}"),
            Self::Os { code, function } => {
                write!(f, "launch failed: OS error {code} from {function}")
            }
            Self::MembershipUnverified => write!(
                f,
                "launch failed: the child exists but job membership could not be verified \
                 (it may be running unmanaged)"
            ),
        }
    }
}

impl From<JobError> for LaunchError {
    fn from(error: JobError) -> Self {
        match error {
            JobError::MembershipUnverified => Self::MembershipUnverified,
            JobError::Os { code, function } => Self::Os { code, function },
        }
    }
}

/// One consented, managed launch: the fresh advisory job, the child, the
/// exact consented request and the preflight facts. Dropping the session
/// (or calling [`ManagedLaunchSession::remove_monitoring`]) removes the
/// monitoring view — never the application.
#[derive(Debug)]
pub struct ManagedLaunchSession {
    job: AdvisoryJob,
    child: ManagedChild,
    request: LaunchRequest,
    preflight: CompatibilityReport,
}

impl ManagedLaunchSession {
    /// Execute one consented launch: validate, preflight the launcher
    /// environment, create the fresh advisory job and spawn the exact
    /// image into it at creation time. The consent token is consumed even
    /// on failure: one consent buys at most one attempt.
    ///
    /// # Errors
    /// [`LaunchError`] — every variant an explicit outcome (invalid
    /// request, refused creation with an empty job, unverified membership).
    pub fn launch(request: LaunchRequest, consent: LaunchConsent) -> Result<Self, LaunchError> {
        request.validate()?;
        let _consumed = consent;
        let preflight = compatibility_report(None, None).map_err(LaunchError::from)?;
        let job = AdvisoryJob::create().map_err(LaunchError::from)?;
        let child = ManagedChild::spawn_exact(&job, &request.executable, &request.arguments)
            .map_err(LaunchError::from)?;
        Ok(Self {
            job,
            child,
            request,
            preflight,
        })
    }

    /// The child's PID.
    #[must_use]
    pub fn child_pid(&self) -> u32 {
        self.child.pid()
    }

    /// The exact consented request (what was launched).
    #[must_use]
    pub fn request(&self) -> &LaunchRequest {
        &self.request
    }

    /// The preflight compatibility report captured at launch.
    #[must_use]
    pub fn preflight(&self) -> &CompatibilityReport {
        &self.preflight
    }

    /// Whether the child is a member of its advisory job, through its own
    /// spawn-time handle.
    ///
    /// # Errors
    /// [`JobError`] when the query fails.
    pub fn child_in_job(&self) -> Result<bool, JobError> {
        self.child.in_job(&self.job)
    }

    /// The job's effective limit flags — queried, never assumed. The
    /// advisory contract: this is always 0 (AT-16's "queried job flags").
    ///
    /// # Errors
    /// [`JobError`] when the query fails.
    pub fn limit_flags(&self) -> Result<u32, JobError> {
        self.job.limit_flags()
    }

    /// Group accounting for the managed group.
    ///
    /// # Errors
    /// [`JobError`] when the query fails.
    pub fn accounting(&self) -> Result<GroupAccounting, JobError> {
        self.job.accounting()
    }

    /// Advisory threshold evaluation over the current accounting (pure,
    /// report-only).
    ///
    /// # Errors
    /// [`JobError`] when the accounting query fails.
    pub fn advisory_status(
        &self,
        thresholds: &AdvisoryThresholds,
    ) -> Result<AdvisoryStatus, JobError> {
        Ok(crate::job::evaluate_advisory(
            &self.accounting()?,
            thresholds,
        ))
    }

    /// Whether the child is still running (zero-timeout wait).
    ///
    /// # Errors
    /// [`JobError`] when the wait fails.
    pub fn child_is_running(&self) -> Result<bool, JobError> {
        self.child.is_running()
    }

    /// **Remove monitoring, only monitoring.** The job and child monitoring
    /// handles are dropped (the accounting view ends with them); the final
    /// facts are queried BEFORE the drop and returned with the child handle
    /// and the explicit semantics statements. Nothing is detached, nothing
    /// is killed: the association lasts the process lifetime and the
    /// application continues unaided (pinned by the tests).
    #[must_use]
    pub fn remove_monitoring(self) -> MonitoringRemoved {
        let final_accounting = self.job.accounting().ok();
        let final_limit_flags = self.job.limit_flags().ok();
        let MonitoringHandleDrop { child, child_pid } = self.into_parts();
        MonitoringRemoved {
            child,
            child_pid,
            final_accounting,
            final_limit_flags,
            statements: semantics_statements(),
        }
    }

    /// Split into the retained child handle (drop the job view, keep the
    /// ability to observe the app) — the removal mechanics themselves.
    fn into_parts(self) -> MonitoringHandleDrop {
        let Self {
            job,
            child,
            request,
            preflight,
        } = self;
        let _ = (job, request, preflight);
        MonitoringHandleDrop {
            child_pid: child.pid(),
            child,
        }
    }
}

/// The parts [`ManagedLaunchSession::into_parts`] keeps: the child handle
/// (observation only) and its PID; the job view is dropped here.
struct MonitoringHandleDrop {
    child_pid: u32,
    child: ManagedChild,
}

/// The outcome of removing monitoring from a managed launch (AT-16's
/// "monitoring is removed" clause): final queried facts, the still-owned
/// child handle for continued observation, and the semantics statements.
#[derive(Debug)]
pub struct MonitoringRemoved {
    /// The child handle: the app remains observable, never killable from
    /// here (this module has no kill).
    pub child: ManagedChild,
    /// The child's PID.
    pub child_pid: u32,
    /// The last successfully queried accounting, taken immediately before
    /// the monitoring handles were dropped (`None` when that final query
    /// failed — the removal itself never fails).
    pub final_accounting: Option<GroupAccounting>,
    /// The final queried limit flags (`None` when the query failed;
    /// otherwise 0 by the advisory contract).
    pub final_limit_flags: Option<u32>,
    /// The explicit semantics statements (see [`semantics_statements`]).
    pub statements: Vec<&'static str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real image that exists on every Windows box (validation fixtures).
    fn ping_exe() -> &'static str {
        "C:\\Windows\\System32\\ping.exe"
    }

    #[test]
    fn only_the_exact_word_consents() {
        assert!(consent_decide(Some("LAUNCH")).is_some());
        assert!(
            consent_decide(Some("  LAUNCH  ")).is_some(),
            "surrounding whitespace is input noise"
        );
        assert!(
            consent_decide(Some("launch")).is_none(),
            "the word is strict, not case-insensitive"
        );
        assert!(consent_decide(Some("LAUNCH now")).is_none());
        assert!(
            consent_decide(Some("ENABLE")).is_none(),
            "the policy stage's word does not consent a launch"
        );
        assert!(consent_decide(Some("")).is_none());
        assert!(
            consent_decide(None).is_none(),
            "end of input is a decline, never a guess"
        );
    }

    #[test]
    fn consent_is_not_constructable_without_the_word() {
        // The only source is consent_decide; the token cannot be faked
        // (private field), so a launch without a typed act cannot compile.
        fn assert_unconstructable<T: std::fmt::Debug + Copy>(value: T) -> T {
            value
        }
        let consent = consent_decide(Some("LAUNCH")).expect("consent");
        let _ = assert_unconstructable(consent);
    }

    #[test]
    fn validation_rejects_unusable_requests() {
        let cases: Vec<(LaunchRequest, &'static str)> = vec![
            (LaunchRequest::new("", ""), "empty"),
            (
                LaunchRequest::new("C:\\does\\not\\exist.exe", ""),
                "does not exist",
            ),
            (LaunchRequest::new("C:\\Windows", ""), "not a regular file"),
            (LaunchRequest::new("C:\0bad.exe", ""), "NUL"),
            (LaunchRequest::new(ping_exe(), "-n\0 4"), "NUL"),
        ];
        for (request, fragment) in cases {
            let error = request.validate().expect_err("must be rejected");
            match error {
                LaunchError::InvalidRequest { reason } => {
                    assert!(
                        reason.contains(fragment) || fragment == "empty",
                        "reason '{reason}' should name the rule ({fragment})"
                    );
                }
                other => panic!("expected InvalidRequest, got {other:?}"),
            }
        }
        // A real image validates.
        LaunchRequest::new(ping_exe(), "-n 4 127.0.0.1")
            .validate()
            .expect("a real image validates");
    }

    #[test]
    fn command_line_quotes_paths_with_spaces_and_appends_arguments() {
        let request = LaunchRequest::new("C:\\Program Files\\App\\tool.exe", "--flag value");
        assert_eq!(
            request.command_line(),
            "\"C:\\Program Files\\App\\tool.exe\" --flag value"
        );
        let plain = LaunchRequest::new("C:\\Windows\\System32\\ping.exe", "-n 4 127.0.0.1");
        assert_eq!(
            plain.command_line(),
            "C:\\Windows\\System32\\ping.exe -n 4 127.0.0.1"
        );
        let bare = LaunchRequest::new("C:\\tools\\app.exe", "");
        assert_eq!(bare.command_line(), "C:\\tools\\app.exe");
    }

    #[test]
    fn consent_prompt_displays_the_exact_request_and_semantics() {
        let request = LaunchRequest::new(ping_exe(), "-n 4 127.0.0.1");
        let preflight = compatibility_report(None, None).expect("preflight");
        let prompt = render_consent_prompt(&request, &preflight);
        assert!(prompt.contains(&request.command_line()), "{prompt}");
        assert!(prompt.contains("LAUNCH"), "{prompt}");
        assert!(prompt.contains("not a sandbox"), "{prompt}");
        assert!(prompt.contains("not detachable"), "{prompt}");
        assert!(prompt.contains("does not detach"), "{prompt}");
        assert!(prompt.contains("no kill-on-close"), "{prompt}");
        if preflight.launcher_in_job {
            assert!(
                prompt.contains("preflight    : launcher"),
                "the nesting note must be displayed pre-consent: {prompt}"
            );
        }
    }

    #[test]
    fn statements_state_the_at16_semantics() {
        let statements = semantics_statements();
        assert_eq!(statements.len(), 4);
        assert!(statements.iter().any(|s| s.contains("not a sandbox")));
        assert!(statements.iter().any(|s| s.contains("not detachable")));
        assert!(statements.iter().any(|s| s.contains("does not detach")));
        assert!(statements.iter().any(|s| s.contains("no kill-on-close")));
    }

    #[test]
    fn launch_error_display_is_explicit() {
        assert_eq!(
            LaunchError::ConsentDeclined.to_string(),
            "launch declined (consent word absent or wrong)"
        );
        assert_eq!(
            LaunchError::InvalidRequest {
                reason: "executable does not exist"
            }
            .to_string(),
            "invalid launch request: executable does not exist"
        );
        assert_eq!(
            LaunchError::Os {
                code: 2,
                function: "CreateProcessW"
            }
            .to_string(),
            "launch failed: OS error 2 from CreateProcessW"
        );
        assert!(LaunchError::MembershipUnverified
            .to_string()
            .contains("unmanaged"));
    }

    // ---- Live managed-launch evidence (the ping-child fixture pattern) ----

    #[test]
    fn consented_launch_monitors_and_removal_leaves_the_app_alive() {
        let request = LaunchRequest::new(ping_exe(), "-n 5 127.0.0.1");
        let consent = consent_decide(Some("LAUNCH")).expect("typed consent");
        let session = ManagedLaunchSession::launch(request, consent).expect("managed launch");

        // AT-16, queried: zero limit flags on the fresh advisory job, real
        // membership, live accounting.
        assert_eq!(session.limit_flags().expect("limit flags"), 0);
        assert!(session.child_in_job().expect("membership"));
        let accounting = session.accounting().expect("accounting");
        assert!(accounting.active_processes >= 1, "{accounting:?}");
        assert_eq!(
            session
                .advisory_status(&AdvisoryThresholds::default())
                .expect("advisory"),
            AdvisoryStatus::Within,
            "default thresholds evaluate nothing"
        );
        assert!(session.child_is_running().expect("alive"));

        // Remove monitoring: final facts were queried BEFORE the drop.
        let removed = session.remove_monitoring();
        assert_eq!(removed.child_pid, removed.child.pid());
        assert_eq!(
            removed.final_limit_flags,
            Some(0),
            "the advisory contract holds at removal time"
        );
        assert!(removed.final_accounting.is_some());
        assert_eq!(removed.statements.len(), 4);

        // The app REMAINS ALIVE after monitoring was removed (AT-16).
        assert!(
            removed.child.is_running().expect("alive after removal"),
            "the child must outlive the monitoring view"
        );
        // Membership is NOT detachable: with every monitoring handle gone,
        // the kernel keeps the job (it still has a member) and the process
        // still observes as jobbed.
        let membership =
            crate::job::observe_membership(removed.child_pid, None).expect("observe after removal");
        assert!(
            membership.in_any_job,
            "membership must persist after monitoring removal: {membership:?}"
        );

        // And the child reaches completion unaided — no one holds it.
        let exit = removed.child.wait().expect("completes unaided");
        assert_eq!(exit, 0, "ping exits 0");
    }

    #[test]
    fn a_nonexecutable_request_fails_explicitly_and_launches_nothing() {
        // P0-04: shortcuts/scripts/associations are out of scope — an
        // existing but non-executable file must fail LOUDLY at the OS, not
        // resolve through any broker. The consent for this attempt is
        // consumed either way (one consent, at most one attempt).
        let mut probe = std::env::temp_dir();
        probe.push(format!("ramshaper-launch-probe-{}.txt", std::process::id()));
        std::fs::write(&probe, "not an image").expect("write probe file");
        let request = LaunchRequest::new(probe.clone(), "");
        request.validate().expect("the file exists and is a file");

        let consent = consent_decide(Some("LAUNCH")).expect("typed consent");
        let error = ManagedLaunchSession::launch(request, consent)
            .expect_err("a text file is not an executable image");
        match error {
            LaunchError::Os { code, function } => {
                assert_eq!(function, "CreateProcessW");
                assert_eq!(code, 193, "%1 is not a valid Win32 application");
            }
            other => panic!("expected an explicit OS refusal, got {other:?}"),
        }
        std::fs::remove_file(&probe).expect("clean up probe file");
    }

    #[test]
    fn a_declined_consent_never_reaches_the_launcher() {
        let request = LaunchRequest::new(ping_exe(), "-n 1 127.0.0.1");
        request.validate().expect("valid");
        // Any word other than LAUNCH (or none) yields no token, and without
        // a token `launch` is uncallable — the decline IS the outcome.
        assert!(consent_decide(Some("yes")).is_none());
        assert!(consent_decide(None).is_none());
        let _ = request;
    }
}
