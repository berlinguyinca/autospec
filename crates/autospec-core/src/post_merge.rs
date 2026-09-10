//! Merged and fixed are separate states (#4119): the observation that
//! confirms a change is recorded as part of the change, and the change is
//! not fixed until it is made.
//!
//! The incident: CI on `main` was cancelling every run before it finished.
//! The fix looked obvious — `cancel-in-progress: ${{ github.ref !=
//! 'refs/heads/main' }}` — and it was verified in every way available
//! *before* the merge: the YAML parsed, the expression was logically
//! correct, the intent matched the defect. It merged. Six consecutive runs
//! on `main` were still cancelled. The property the fix asserted — *runs on
//! `main` are not cancelled* — was never checked after the merge. The issue
//! was closed, the artefact got its section, the status report said "fixed"
//! — and the next person had no reason to check again.
//!
//! The checks available before a merge (it parses, the logic is right)
//! produce the same confidence as "it works", and for most changes they
//! coincide. For changes to a control plane — CI configuration, dispatch
//! policy, merge automation — they do not: the effect is observable only in
//! the next cycle. This module makes that distinction mechanical.
//!
//! Everything here is pure and testable: no I/O, no clock, no `gh`.
//!
//! 1. **A change whose effect is only observable after deployment declares
//!    the observation that will confirm it** — the specific query, log, or
//!    metric, and its expected value
//!    ([`ObservationDeclaration`], [`ObservationDeclaration::parse`]). The
//!    declaration is recorded as part of the change (the closeout report),
//!    so the confirmation cannot be lost with the session that made the
//!    change.
//! 2. **Merged and fixed are separate states** ([`FixState`],
//!    [`confirm_fixed`]). A change with a declared observation is not fixed
//!    until the observation has been made and matches the expected value.
//!    Merging converts an open problem into a *pending* one, not a closed
//!    one.
//! 3. **A control-plane change schedules a follow-up check**
//!    ([`is_control_plane`], [`ReviewFlag::ControlPlaneWithoutFollowUp`]).
//!    The failure mode in #4119 was not a wrong fix; it was nobody looking
//!    again.
//! 4. **A change that declares no observable effect is flagged at review**
//!    ([`ReviewFlag::NoDeclaredObservation`], [`review_flags`]). Flagged,
//!    not silently accepted: the absence is visible to the reviewer, who
//!    can waive it with a reason.
//! 5. **A closeout that declares an observation may not claim fixed**
//!    ([`validate_post_merge_closeout`]). The report grammar is
//!    `Post-merge observation: <query> — expected: <value>` plus an
//!    optional `Follow-up check: <what gets re-checked and when>`; while
//!    the observation is unrecorded the `Result:` line must not use the
//!    words *fixed* or *resolved*.

use std::fmt;

/// The line prefix the closeout report uses to declare a post-merge
/// observation.
pub const OBSERVATION_KEY: &str = "Post-merge observation:";

/// The line prefix the closeout report uses to schedule a follow-up check.
pub const FOLLOW_UP_KEY: &str = "Follow-up check:";

/// The marker that separates the observation from its expected value.
pub const EXPECTED_MARKER: &str = "expected:";

/// The specific observation that will confirm a change, recorded as part of
/// the change: the query, log, or metric, and the value that confirms it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationDeclaration {
    /// The specific query, log, or metric to run after the change is live.
    pub observation: String,
    /// The value (or property of the output) that confirms the change
    /// took effect.
    pub expected: String,
}

impl ObservationDeclaration {
    /// Build a declaration, rejecting either half when it is empty.
    pub fn new(
        observation: impl Into<String>,
        expected: impl Into<String>,
    ) -> Result<Self, DeclarationError> {
        let observation = observation.into();
        let expected = expected.into();
        if observation.trim().is_empty() {
            return Err(DeclarationError::EmptyObservation);
        }
        if expected.trim().is_empty() {
            return Err(DeclarationError::EmptyExpected);
        }
        Ok(Self {
            observation: observation.trim().to_string(),
            expected: expected.trim().to_string(),
        })
    }

    /// Parse a closeout report line:
    ///
    /// ```text
    /// Post-merge observation: <query, log, or metric> — expected: <value>
    /// ```
    ///
    /// The observation is the text before the first `expected:` marker; the
    /// expected value is the text after it. Both halves must be non-empty.
    pub fn parse(line: &str) -> Result<Self, DeclarationError> {
        let rest = line
            .trim()
            .strip_prefix(OBSERVATION_KEY)
            .ok_or(DeclarationError::MissingPrefix)?;
        let Some(marker) = rest.find(EXPECTED_MARKER) else {
            return Err(DeclarationError::MissingExpected);
        };
        let observation = rest[..marker]
            .trim()
            .trim_end_matches(['—', '-', ':', ' '])
            .trim();
        let expected = rest[marker + EXPECTED_MARKER.len()..].trim();
        Self::new(observation, expected)
    }
}

/// A [`ObservationDeclaration::parse`] or [`ObservationDeclaration::new`]
/// failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationError {
    /// The line does not start with [`OBSERVATION_KEY`].
    MissingPrefix,
    /// The line has no `expected:` marker, so there is no expected value to
    /// confirm against.
    MissingExpected,
    /// The query/log/metric half is empty.
    EmptyObservation,
    /// The expected-value half is empty.
    EmptyExpected,
}

impl fmt::Display for DeclarationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MissingPrefix => {
                format!("must start with `{OBSERVATION_KEY}`")
            }
            Self::MissingExpected => {
                format!("must carry the expected value after `{EXPECTED_MARKER}`")
            }
            Self::EmptyObservation => "must name the specific query, log, or metric".to_string(),
            Self::EmptyExpected => "must name the expected value".to_string(),
        };
        f.write_str(&message)
    }
}

/// A change under review: what it is, what it touches, and what it declared
/// about its own confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The change claims to fix a defect (as opposed to adding a feature
    /// whose effect is observable in the same cycle).
    pub is_fix: bool,
    /// The paths the change touches.
    pub paths: Vec<String>,
    /// The caller asserts the change targets a control plane the path list
    /// cannot recognise (dispatch policy, merge automation).
    pub control_plane: bool,
    /// The declared post-merge observation, if any.
    pub declaration: Option<ObservationDeclaration>,
    /// The scheduled follow-up check, if any.
    pub follow_up_check: Option<String>,
}

impl Change {
    /// Whether the change targets a control plane: a path the vocabulary
    /// recognises, or an explicit assertion by the caller.
    pub fn targets_control_plane(&self) -> bool {
        self.control_plane || self.paths.iter().any(|path| is_control_plane(path))
    }
}

/// Path prefixes whose effect is observable only in the next cycle: CI
/// configuration and the reusable build machinery it composes.
pub const CONTROL_PLANE_PREFIXES: &[&str] =
    &[".github/workflows/", ".github/actions/", ".github/scripts/"];

/// Whole-file control-plane surfaces.
pub const CONTROL_PLANE_FILES: &[&str] = &[".github/dependabot.yml", "Jenkinsfile"];

/// Whether a single path is a control-plane surface.
pub fn is_control_plane(path: &str) -> bool {
    let path = path.trim().trim_start_matches("./");
    CONTROL_PLANE_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
        || CONTROL_PLANE_FILES.iter().any(|file| path == *file)
}

/// The two states a merged change occupies. Merging lands the code; only a
/// recorded observation closes the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixState {
    /// The change is in the tree, but its declared observation has not been
    /// made (or contradicts the expected value).
    Merged,
    /// The declared observation has been made and matches the expected
    /// value.
    Fixed,
}

/// The change declared no observation, so nothing can ever confirm it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmError {
    /// No post-merge observation is declared; the change cannot be
    /// confirmed fixed, and a report claiming otherwise is the #4119
    /// signature.
    NoObservationDeclared,
}

impl fmt::Display for ConfirmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoObservationDeclared => f.write_str(
                "no post-merge observation declared; the change cannot be confirmed fixed",
            ),
        }
    }
}

/// Decide the state of a merged change from its recorded observation, if
/// any.
///
/// - No declaration: [`ConfirmError::NoObservationDeclared`] — there is no
///   observation that could ever confirm the change.
/// - Declaration, no recorded observation: [`FixState::Merged`] (AC: a
///   change that declares a post-merge observation is not marked resolved
///   until the observation is recorded).
/// - Recorded observation matching the expected value: [`FixState::Fixed`].
/// - Recorded observation contradicting the expected value:
///   [`FixState::Merged`] — the change is demonstrably *not* working, which
///   is further from fixed, not closer.
pub fn confirm_fixed(change: &Change, observed: Option<&str>) -> Result<FixState, ConfirmError> {
    let Some(declaration) = &change.declaration else {
        return Err(ConfirmError::NoObservationDeclared);
    };
    match observed {
        None => Ok(FixState::Merged),
        Some(value) if value.trim() == declaration.expected.trim() => Ok(FixState::Fixed),
        Some(_) => Ok(FixState::Merged),
    }
}

/// A review-time flag: visible to the reviewer, waivable with a reason,
/// never silently accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewFlag {
    /// A fix that declares no observable effect. Flagged at review — the
    /// reviewer must see that nothing will ever confirm the fix.
    NoDeclaredObservation,
    /// A control-plane change that declares no observation: its effect is
    /// only observable after deployment, so the confirmation must be
    /// recorded as part of the change.
    ControlPlaneWithoutObservation,
    /// A control-plane change with no scheduled follow-up check: the
    /// re-look is left to chance, which is exactly how #4119 aged from
    /// incident to "fixed".
    ControlPlaneWithoutFollowUp,
}

/// The review flags a change carries. Deterministic order: declaration
/// flags first, then the control-plane flags.
pub fn review_flags(change: &Change) -> Vec<ReviewFlag> {
    let mut flags = Vec::new();
    if change.is_fix && change.declaration.is_none() {
        flags.push(ReviewFlag::NoDeclaredObservation);
    }
    if change.targets_control_plane() {
        if change.declaration.is_none() {
            flags.push(ReviewFlag::ControlPlaneWithoutObservation);
        }
        if change
            .follow_up_check
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            flags.push(ReviewFlag::ControlPlaneWithoutFollowUp);
        }
    }
    flags
}

/// Whether text claims a defect is fixed or resolved: the words *fixed* and
/// *resolved*, case-insensitive, at word boundaries (so `prefixed`,
/// `unresolved`, and `fix` do not match).
pub fn claims_fixed(text: &str) -> bool {
    ["fixed", "resolved"]
        .iter()
        .any(|word| contains_word(text, word))
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    let lower = haystack.to_lowercase();
    let hay = lower.as_str();
    let n = needle.len();
    let mut start = 0;
    while let Some(rel) = hay[start..].find(needle) {
        let idx = start + rel;
        let before_ok = idx == 0 || !hay.as_bytes()[idx - 1].is_ascii_alphanumeric();
        let after = idx + n;
        let after_ok = after >= hay.len() || !hay.as_bytes()[after].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = idx + 1;
    }
    false
}

/// A closeout-report body that fails the post-merge observation rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseoutBodyError {
    /// A `Post-merge observation:` line that does not carry both halves.
    MalformedDeclaration(DeclarationError),
    /// More than one `Post-merge observation:` line.
    DuplicateObservation,
    /// More than one `Follow-up check:` line.
    DuplicateFollowUp,
    /// A `Follow-up check:` line with no content.
    EmptyFollowUp,
    /// The body declares an observation but the `Result:` line claims the
    /// defect fixed or resolved. Merged and fixed are separate states: the
    /// change stays merged until the observation is recorded.
    FixedClaimedBeforeObservation,
}

impl fmt::Display for CloseoutBodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedDeclaration(inner) => {
                write!(f, "{OBSERVATION_KEY} {inner}")
            }
            Self::DuplicateObservation => {
                write!(f, "{OBSERVATION_KEY} must appear exactly once")
            }
            Self::DuplicateFollowUp => {
                write!(f, "{FOLLOW_UP_KEY} must appear exactly once")
            }
            Self::EmptyFollowUp => {
                write!(f, "{FOLLOW_UP_KEY} must not be empty")
            }
            Self::FixedClaimedBeforeObservation => f.write_str(
                "merged and fixed are separate states: a closeout that declares a post-merge \
                 observation must not claim the defect fixed or resolved until the observation \
                 is recorded",
            ),
        }
    }
}

/// Validate the post-merge observation section of a closeout report body.
///
/// Rules (#4119):
///
/// - `Post-merge observation:` appears at most once; when present it must
///   carry both the observation and the expected value
///   ([`ObservationDeclaration::parse`]).
/// - `Follow-up check:` appears at most once; when present it must not be
///   empty.
/// - A body that declares an observation must not claim the change fixed
///   or resolved in its `Result:` line: the declared observation is, by
///   definition of a closeout (written before the merge), unrecorded, so
///   the change is merged, not fixed.
///
/// Bodies without either line are unchanged: the declaration is mandatory
/// for control-plane changes at *review* time ([`review_flags`]), not at
/// parse time.
pub fn validate_post_merge_closeout(body: &str) -> Result<(), CloseoutBodyError> {
    let mut observation_lines = 0usize;
    let mut follow_up_lines = 0usize;
    let mut result_claim = String::new();

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with(OBSERVATION_KEY) {
            observation_lines += 1;
            if observation_lines > 1 {
                return Err(CloseoutBodyError::DuplicateObservation);
            }
            ObservationDeclaration::parse(line).map_err(CloseoutBodyError::MalformedDeclaration)?;
            continue;
        }
        if let Some(rest) = line.strip_prefix(FOLLOW_UP_KEY) {
            follow_up_lines += 1;
            if follow_up_lines > 1 {
                return Err(CloseoutBodyError::DuplicateFollowUp);
            }
            if rest.trim().is_empty() {
                return Err(CloseoutBodyError::EmptyFollowUp);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("Result:") {
            result_claim = rest.trim().to_string();
        }
    }

    if observation_lines == 1 && claims_fixed(&result_claim) {
        return Err(CloseoutBodyError::FixedClaimedBeforeObservation);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration() -> ObservationDeclaration {
        ObservationDeclaration::new(
            "gh run list --branch main --limit 5",
            "no cancelled runs on main",
        )
        .expect("declaration parses")
    }

    fn change(paths: &[&str]) -> Change {
        Change {
            is_fix: true,
            paths: paths.iter().map(|s| s.to_string()).collect(),
            control_plane: false,
            declaration: Some(declaration()),
            follow_up_check: Some("next main run is not cancelled".to_string()),
        }
    }

    fn body(result: &str, observation: Option<&str>, follow_up: Option<&str>) -> String {
        let mut out = String::from("## Closeout report\nResult: ");
        out.push_str(result);
        out.push('\n');
        if let Some(observation) = observation {
            out.push_str(observation);
            out.push('\n');
        }
        if let Some(follow_up) = follow_up {
            out.push_str(follow_up);
            out.push('\n');
        }
        out
    }

    // ── AC: a declared observation does not resolve the change ──

    #[test]
    fn declared_observation_without_record_stays_merged() {
        let change = change(&[]);
        assert_eq!(
            confirm_fixed(&change, None).expect("pending, not an error"),
            FixState::Merged,
            "a change that declares a post-merge observation is not marked resolved \
             until the observation is recorded"
        );
    }

    #[test]
    fn recorded_matching_observation_confirms_fixed() {
        let change = change(&[]);
        assert_eq!(
            confirm_fixed(&change, Some("no cancelled runs on main")).expect("recorded"),
            FixState::Fixed
        );
    }

    #[test]
    fn recorded_contradicting_observation_stays_merged() {
        let change = change(&[]);
        assert_eq!(
            confirm_fixed(&change, Some("runs on main still cancelled"))
                .expect("a contradiction is not an error"),
            FixState::Merged,
            "an observation contradicting the expected value moves the change \
             further from fixed, not closer"
        );
    }

    #[test]
    fn no_declaration_cannot_confirm() {
        let mut change = change(&[]);
        change.declaration = None;
        assert_eq!(
            confirm_fixed(&change, Some("whatever")).expect_err("nothing to confirm"),
            ConfirmError::NoObservationDeclared
        );
    }

    // ── AC: a change with no declared observable effect is flagged ──

    #[test]
    fn fix_without_declared_effect_is_flagged_at_review() {
        let mut change = change(&["src/parser.rs"]);
        change.declaration = None;
        change.follow_up_check = None;
        assert_eq!(
            review_flags(&change),
            vec![ReviewFlag::NoDeclaredObservation],
            "a fix that declares no observable effect is flagged at review"
        );
    }

    #[test]
    fn declared_fix_is_not_flagged() {
        let change = change(&["src/parser.rs"]);
        assert_eq!(review_flags(&change), Vec::<ReviewFlag>::new());
    }

    #[test]
    fn non_fix_without_declaration_is_not_flagged() {
        let mut change = change(&["src/parser.rs"]);
        change.is_fix = false;
        change.declaration = None;
        change.follow_up_check = None;
        assert_eq!(review_flags(&change), Vec::<ReviewFlag>::new());
    }

    // ── control plane: observation and follow-up check ──

    #[test]
    fn ci_config_is_control_plane_and_requires_observation_and_follow_up() {
        let mut change = Change {
            is_fix: true,
            paths: vec![".github/workflows/ci.yml".to_string()],
            control_plane: false,
            declaration: None,
            follow_up_check: None,
        };
        assert!(change.targets_control_plane());
        assert_eq!(
            review_flags(&change),
            vec![
                ReviewFlag::NoDeclaredObservation,
                ReviewFlag::ControlPlaneWithoutObservation,
                ReviewFlag::ControlPlaneWithoutFollowUp,
            ]
        );

        change.declaration = Some(declaration());
        change.follow_up_check = Some("next main run is not cancelled".to_string());
        assert_eq!(review_flags(&change), Vec::<ReviewFlag>::new());
    }

    #[test]
    fn follow_up_check_is_scheduled_not_left_to_chance() {
        let mut change = change(&[".github/workflows/ci.yml"]);
        change.declaration = Some(declaration());
        change.follow_up_check = None;
        assert_eq!(
            review_flags(&change),
            vec![ReviewFlag::ControlPlaneWithoutFollowUp]
        );
        change.follow_up_check = Some("cron sweep re-runs the query daily".to_string());
        assert_eq!(review_flags(&change), Vec::<ReviewFlag>::new());
    }

    #[test]
    fn control_plane_vocabulary_covers_ci_surfaces() {
        assert!(is_control_plane(".github/workflows/ci.yml"));
        assert!(is_control_plane("./.github/actions/cache/action.yml"));
        assert!(is_control_plane(".github/dependabot.yml"));
        assert!(is_control_plane("Jenkinsfile"));
        assert!(!is_control_plane("src/main.rs"));
        assert!(!is_control_plane("docs/ci.md"));
        assert!(!is_control_plane(".github/workflows"));
    }

    #[test]
    fn explicit_control_plane_assertion_counts() {
        let change = Change {
            is_fix: true,
            paths: vec!["scripts/dispatch-policy.toml".to_string()],
            control_plane: true,
            declaration: Some(declaration()),
            follow_up_check: Some("next dispatch batch routes correctly".to_string()),
        };
        assert!(change.targets_control_plane());
        assert_eq!(review_flags(&change), Vec::<ReviewFlag>::new());
    }

    // ── declaration grammar ──

    #[test]
    fn parse_declaration_full_line() {
        let declaration = ObservationDeclaration::parse(
            "Post-merge observation: gh run list --limit 5 — expected: no cancelled runs",
        )
        .expect("well-formed line");
        assert_eq!(declaration.observation, "gh run list --limit 5");
        assert_eq!(declaration.expected, "no cancelled runs");
    }

    #[test]
    fn parse_declaration_rejects_missing_halves() {
        assert_eq!(
            ObservationDeclaration::parse("Post-merge observation: gh run list"),
            Err(DeclarationError::MissingExpected)
        );
        assert_eq!(
            ObservationDeclaration::parse("Post-merge observation: — expected:"),
            Err(DeclarationError::EmptyObservation)
        );
        assert_eq!(
            ObservationDeclaration::parse("Post-merge observation: gh run list — expected:   "),
            Err(DeclarationError::EmptyExpected)
        );
        assert_eq!(
            ObservationDeclaration::parse("Result: fixed it"),
            Err(DeclarationError::MissingPrefix)
        );
    }

    // ── closeout body rules ──

    #[test]
    fn closeout_with_declaration_must_not_claim_fixed() {
        let body = body(
            "Fixed the main-branch cancellation; runs now survive",
            Some("Post-merge observation: gh run list --limit 5 — expected: no cancelled runs"),
            Some("Follow-up check: re-run the query after the next main push"),
        );
        let error = validate_post_merge_closeout(&body)
            .expect_err("fixed claim with a pending observation");
        assert_eq!(error, CloseoutBodyError::FixedClaimedBeforeObservation);
    }

    #[test]
    fn closeout_with_declaration_stays_merged_in_result() {
        let body = body(
            "Landed the concurrency-group fix; merged, pending observation on the next main runs",
            Some("Post-merge observation: gh run list --limit 5 — expected: no cancelled runs"),
            Some("Follow-up check: re-run the query after the next main push"),
        );
        validate_post_merge_closeout(&body).expect("merged, not fixed, is the honest state");
    }

    #[test]
    fn closeout_without_declaration_is_untouched() {
        let body = body("Fixed the parser; 14 new tests green", None, None);
        validate_post_merge_closeout(&body).expect("no declaration, nothing to enforce");
    }

    #[test]
    fn closeout_rejects_duplicate_and_empty_lines() {
        let mut out = body(
            "Merged, pending observation",
            Some("Post-merge observation: gh run list — expected: green"),
            None,
        );
        out.push_str("Post-merge observation: other query — expected: other\n");
        assert_eq!(
            validate_post_merge_closeout(&out),
            Err(CloseoutBodyError::DuplicateObservation)
        );

        let mut out = body(
            "Merged, pending observation",
            None,
            Some("Follow-up check: check A"),
        );
        out.push_str("Follow-up check:\n");
        assert_eq!(
            validate_post_merge_closeout(&out),
            Err(CloseoutBodyError::DuplicateFollowUp)
        );

        let out = body(
            "Merged, pending observation",
            None,
            Some("Follow-up check:   "),
        );
        assert_eq!(
            validate_post_merge_closeout(&out),
            Err(CloseoutBodyError::EmptyFollowUp)
        );
    }

    #[test]
    fn closeout_rejects_malformed_declaration() {
        let body = body(
            "Merged, pending observation",
            Some("Post-merge observation: gh run list"),
            None,
        );
        assert_eq!(
            validate_post_merge_closeout(&body),
            Err(CloseoutBodyError::MalformedDeclaration(
                DeclarationError::MissingExpected
            ))
        );
    }

    // ── claim detection ──

    #[test]
    fn claims_fixed_matches_only_whole_words() {
        assert!(claims_fixed("Fixed the cancellation"));
        assert!(claims_fixed("the issue is resolved"));
        assert!(claims_fixed("RESOLVED upstream"));
        assert!(!claims_fixed("the prefix is unchanged"));
        assert!(!claims_fixed("still unresolved"));
        assert!(!claims_fixed("the fix landed"));
        assert!(!claims_fixed("n/a — control-plane change"));
    }
}
