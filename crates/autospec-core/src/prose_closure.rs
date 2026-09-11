//! Prose closure safety (issue #4305).
//!
//! A PR body's prose accidentally closed an issue on merge. The body said
//! `Refs #288 (does not close it)` — the converter's explicit, machine-readable
//! decision — and eleven lines later, in prose, `closed #288`, while
//! describing an earlier PR. GitHub closes on the closing keyword
//! adjacent to the reference, case-insensitively, anywhere in the body: the
//! prose won.
//!
//! The converter carries four invariants. The primitives here make each one
//! checkable:
//!
//! 1. **Never write `<keyword> #N` in prose.** `prose_violations` flags a
//!    live (bare `#N`) closing keyword adjacent to a reference for the issue
//!    anywhere except the closing trailer line — the trailer
//!    (`has_closing_trailer`) is the one place a live directive is
//!    intentional, and only when the change really closes the issue.
//! 2. **Escape or use a full URL when discussing closure.** `escaped_reference`
//!    and `url_reference` produce the two safe reference forms; `find_closure_directives`
//!    classifies every directive it finds — escaped (`&#35;N` / `&#x23;N`) and
//!    URL (`…/issues/N`) references are inert in GitHub's raw-text closing
//!    keyword detection.
//! 3. **Verify the issue state after merge.** `verify_after_merge`
//!    reconciles the converter's `ClosureDecision` against the observed
//!    `IssueState`; a converter that chose `Refs` must assert the issue is
//!    still open and fail loudly — `PostMergeAction::ReopenIssue` — if it is
//!    not.
//! 4. **Lint before publish.** `lint_refs_body` rejects a closing keyword in
//!    any PR body the converter marked `Refs` (the `has_partial_fix_marker`
//!    marker) — a machine-detectable contradiction, surfaced before merge
//!    rather than by the issue vanishing after it. `pre_publish_lint` runs
//!    both body checks for either decision.

use crate::evidence_fidelity::CLOSING_VERBS;
use crate::execution::closure::has_partial_fix_marker;

/// How a reference to the issue appears next to a closing verb.
///
/// Only a bare `#N` reference is *live*: GitHub's closing-keyword detection
/// runs on the raw body text, and a bare `#N` adjacent to a closing verb
/// closes the issue on merge. The escaped and URL forms do not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceForm {
    /// A bare `#N` reference — the one form that closes on merge.
    Bare,
    /// An HTML-escaped reference (`&#35;N` or `&#x23;N`) — inert.
    Escaped,
    /// A full issue URL (`…/issues/N`) — inert.
    Url,
}

impl ReferenceForm {
    /// Whether this form closes the issue when adjacent to a closing verb.
    pub fn is_live(self) -> bool {
        matches!(self, ReferenceForm::Bare)
    }
}

/// A closing keyword adjacent to a reference of one issue, found in a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosingDirective {
    /// 1-based line number in the body.
    pub line: usize,
    /// The matched closing verb, lower-cased.
    pub verb: &'static str,
    /// How the reference appears.
    pub form: ReferenceForm,
}

impl ClosingDirective {
    /// The report line for this directive.
    pub fn line_report(&self) -> String {
        format!(
            "closing keyword `{}` adjacent to issue reference on line {} ({} form)",
            self.verb,
            self.line,
            match self.form {
                ReferenceForm::Bare => "live",
                ReferenceForm::Escaped => "escaped",
                ReferenceForm::Url => "url",
            }
        )
    }
}

/// The converter's explicit closure decision for an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureDecision {
    /// The change claims to close the issue; a live closing trailer is
    /// expected in the body.
    Closes,
    /// The change references the issue without closing it (partial fix); the
    /// body carries the `Refs #N (does not close it)` marker.
    Refs,
}

/// The issue's observed state, read from the tracker after merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueState {
    Open,
    Closed,
}

/// What the post-merge verification tells the caller to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostMergeAction {
    /// Observed state matches the decision; nothing to do.
    Consistent,
    /// The converter chose `Refs` but the issue closed anyway (a stray
    /// live directive did the closing): reopen it.
    ReopenIssue,
    /// The converter chose `Closes` but the issue is still open: report the
    /// tracker lag and close the issue explicitly.
    ReportTrackerLag,
}

/// The result of reconciling a closure decision against the observed issue
/// state after merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostMergeCheck {
    /// The issue the decision was about.
    pub issue: u64,
    /// The converter's decision.
    pub decision: ClosureDecision,
    /// The state the issue was observed in after merge.
    pub observed: IssueState,
    /// What the caller must do.
    pub action: PostMergeAction,
}

impl PostMergeCheck {
    /// Whether the observed state matches the decision.
    pub fn consistent(&self) -> bool {
        self.action == PostMergeAction::Consistent
    }

    /// The report line.
    pub fn line(&self) -> String {
        match self.action {
            PostMergeAction::Consistent => format!(
                "issue #{} {} after merge, as the {:?} decision intended",
                self.issue,
                match self.observed {
                    IssueState::Open => "is open",
                    IssueState::Closed => "closed",
                },
                self.decision
            ),
            PostMergeAction::ReopenIssue => format!(
                "FAIL: issue #{} closed after merge but the converter chose Refs (does not close \
                 it) — fail loudly and reopen #{}",
                self.issue, self.issue
            ),
            PostMergeAction::ReportTrackerLag => format!(
                "FAIL: issue #{} is still open after merge but the converter chose Closes — \
                 report the tracker lag and close #{} explicitly",
                self.issue, self.issue
            ),
        }
    }
}

/// A closing keyword found in a body the converter marked `Refs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefsBodyViolation {
    /// Every closing directive found in the body, with line numbers.
    pub directives: Vec<ClosingDirective>,
}

/// Findings from a pre-publish lint of a PR body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrePublishViolation {
    /// Each finding, one per rule broken.
    pub findings: Vec<String>,
}

/// The two reference forms that are safe to use in prose while discussing
/// closure: the HTML-escaped reference and the full issue URL.
///
/// GitHub's closing-keyword detection runs on the raw body text; neither
/// form matches a closing verb as a live reference.
pub fn escaped_reference(issue_number: u64) -> String {
    format!("&#35;{issue_number}")
}

pub fn url_reference(owner: &str, repo: &str, issue_number: u64) -> String {
    format!("https://github.com/{owner}/{repo}/issues/{issue_number}")
}

/// Whether the body's final non-empty line is exactly a closing trailer for
/// this issue — `<verb> #N`, any closing verb, case-insensitive, nothing
/// else on the line.
///
/// The trailer is the one place a live closing directive is intentional.
/// Prose on the same line ("… and this closes #288.") is not a trailer,
/// because GitHub would close on the keyword anyway — the body is broken
/// either way and invariant 1 flags the prose directive.
pub fn has_closing_trailer(body: &str, issue_number: u64) -> bool {
    body.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .is_some_and(|last| is_trailer_line(last, issue_number))
}

/// Whether a line is exactly a closing trailer for this issue.
pub fn is_trailer_line(line: &str, issue_number: u64) -> bool {
    let trimmed = line.trim();
    let Some((verb, rest)) = trimmed.split_once(char::is_whitespace) else {
        return false;
    };
    rest.trim_start() == format!("#{issue_number}")
        && CLOSING_VERBS.iter().any(|v| verb.eq_ignore_ascii_case(v))
}

/// Every closing directive in the body for this issue — live (bare `#N`),
/// escaped, and URL forms alike — each with its line number and reference
/// form.
///
/// Matching mirrors GitHub's closing-keyword detection: a closing verb as a
/// whole word, then optional whitespace, then the reference. Case-
/// insensitive on the verb.
pub fn find_closure_directives(body: &str, issue_number: u64) -> Vec<ClosingDirective> {
    let mut found = Vec::new();
    for (idx, line) in body.lines().enumerate() {
        found.extend(directives_in_line(line, idx + 1, issue_number));
    }
    found
}

/// The live (bare `#N`) closing directives in the body that are NOT on the
/// closing trailer line.
///
/// This is the machine-detectable form of invariant 1: a `<keyword> #N` in
/// prose. An empty result means the body either carries no live directive
/// or carries one only where the trailer belongs.
pub fn prose_violations(body: &str, issue_number: u64) -> Vec<ClosingDirective> {
    let lines: Vec<&str> = body.lines().collect();
    let trailer_line = lines
        .iter()
        .enumerate()
        .rev()
        .find(|(_, l)| !l.trim().is_empty() && is_trailer_line(l, issue_number))
        .map(|(i, _)| i + 1);
    find_closure_directives(body, issue_number)
        .into_iter()
        .filter(|d| d.form.is_live() && Some(d.line) != trailer_line)
        .collect()
}

/// Invariant 4: reject a closing keyword in any PR body the converter
/// marked `Refs`.
///
/// The `Refs` marker (`has_partial_fix_marker`) is the converter's explicit
/// decision that the change does not close the issue. A closing keyword
/// adjacent to the issue reference anywhere in that body is a
/// machine-detectable contradiction: it closes the issue on merge, and the
/// only correct response is to fix the body before publish. A body the
/// converter did not mark `Refs` is not this check's concern.
pub fn lint_refs_body(body: &str, issue_number: u64) -> Result<(), RefsBodyViolation> {
    if !has_partial_fix_marker(body, issue_number) {
        return Ok(());
    }
    // Only live (bare) directives close the issue; escaped and URL forms are
    // the safe ways to discuss closure and are not contradictions.
    let directives = find_closure_directives(body, issue_number)
        .into_iter()
        .filter(|d| d.form.is_live())
        .collect::<Vec<_>>();
    if directives.is_empty() {
        return Ok(());
    }
    Err(RefsBodyViolation { directives })
}

/// The pre-publish gate: the body checks a converter must pass before a PR
/// goes out, for either decision.
///
/// - `Closes`: the body must carry a closing trailer
///   (`has_closing_trailer`), and no live closing directive may sit in
///   prose (`prose_violations`).
/// - `Refs`: every closing keyword in the body is a contradiction
///   (`lint_refs_body`).
pub fn pre_publish_lint(
    decision: ClosureDecision,
    body: &str,
    issue_number: u64,
) -> Result<(), PrePublishViolation> {
    match decision {
        ClosureDecision::Refs => {
            if let Err(v) = lint_refs_body(body, issue_number) {
                return Err(PrePublishViolation {
                    findings: v
                        .directives
                        .into_iter()
                        .map(|d| format!("Refs-marked body: {}", d.line_report()))
                        .collect(),
                });
            }
            Ok(())
        }
        ClosureDecision::Closes => {
            let mut findings = Vec::new();
            if !has_closing_trailer(body, issue_number) {
                findings.push(format!(
                    "Closes-decision body has no closing trailer (a final `{verb} #{n}` line) \
                     for issue #{n}; the issue will not close on merge",
                    verb = "close|closes|closed|fix|fixes|fixed|resolve|resolves|resolved",
                    n = issue_number
                ));
            }
            for d in prose_violations(body, issue_number) {
                findings.push(format!("closing keyword in prose: {}", d.line_report()));
            }
            if findings.is_empty() {
                Ok(())
            } else {
                Err(PrePublishViolation { findings })
            }
        }
    }
}

/// Invariant 3: after merge, the converter verifies the issue state and
/// fails loudly on a mismatch with its decision.
///
/// - `Closes` + `Closed` → `Consistent`.
/// - `Closes` + `Open` → `ReportTrackerLag`: the merge did not close the
///   issue (the trailer was missing, or the tracker lagged); name it.
/// - `Refs` + `Open` → `Consistent`.
/// - `Refs` + `Closed` → `ReopenIssue`: a stray live directive closed the
///   issue the converter decided not to close. Fail loudly and reopen.
pub fn verify_after_merge(
    decision: ClosureDecision,
    observed: IssueState,
    issue_number: u64,
) -> PostMergeCheck {
    let action = match (decision, observed) {
        (ClosureDecision::Closes, IssueState::Closed) => PostMergeAction::Consistent,
        (ClosureDecision::Refs, IssueState::Open) => PostMergeAction::Consistent,
        (ClosureDecision::Refs, IssueState::Closed) => PostMergeAction::ReopenIssue,
        (ClosureDecision::Closes, IssueState::Open) => PostMergeAction::ReportTrackerLag,
    };
    PostMergeCheck {
        issue: issue_number,
        decision,
        observed,
        action,
    }
}

fn directives_in_line(line: &str, line_no: usize, issue_number: u64) -> Vec<ClosingDirective> {
    // GitHub's closing-keyword detection is case-insensitive on the verb;
    // search a lower-cased copy so `CLOSED #288` matches. The matched verb
    // name comes from the (already lower-case) CLOSING_VERBS table.
    let line = line.to_ascii_lowercase();
    let mut found = Vec::new();
    for verb in CLOSING_VERBS {
        let mut search_from = 0;
        while let Some(rel) = line[search_from..].find(verb) {
            let start = search_from + rel;
            if !is_word_start(&line[..start]) {
                search_from = start + verb.len();
                continue;
            }
            let after_verb = start + verb.len();
            if let Some(form) = reference_after(&line, after_verb, issue_number) {
                found.push(ClosingDirective {
                    line: line_no,
                    verb,
                    form,
                });
            }
            search_from = after_verb;
        }
    }
    found
}

/// Whether the character immediately before the verb is a word character.
/// `unclosed #288` must not match on `closed`; a line start, a space, a
/// punctuation mark, or a backtick all do.
fn is_word_start(prefix: &str) -> bool {
    !prefix
        .chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The reference form, if a reference to `issue_number` follows the verb.
///
/// Zero or more whitespace characters may separate the verb and the
/// reference (`closes#288` is a directive). For the URL form the whole URL
/// token is consumed, so `issues/288` and `issues/2883` are told apart by
/// the digit guard.
fn reference_after(line: &str, from: usize, issue_number: u64) -> Option<ReferenceForm> {
    let rest = &line[from..];
    let n_ws = rest.len() - rest.trim_start_matches(|c: char| c.is_whitespace()).len();
    let after_ws = &rest[n_ws..];

    // Bare: #N
    if let Some(tail) = after_ws.strip_prefix('#') {
        if let Some((digits, tail_after)) = split_number(tail) {
            if digits.parse::<u64>().is_ok_and(|n| n == issue_number)
                && !tail_after.starts_with(|c: char| c.is_ascii_digit())
            {
                return Some(ReferenceForm::Bare);
            }
        }
    }

    // Escaped: &#35;N, &#23;N (both are decimal '#'), or &#x23;N (hex '#').
    // The entity number is terminated by ';' — the tail after it is what the
    // digit guard must check.
    if let Some(entity) = after_ws.strip_prefix("&#") {
        let tail = entity
            .strip_prefix("35;")
            .or_else(|| entity.strip_prefix("23;"))
            .or_else(|| entity.strip_prefix("x23;"))
            .or_else(|| entity.strip_prefix("X23;"));
        if let Some(tail) = tail {
            if let Some((digits, tail_after)) = split_number(tail) {
                if digits.parse::<u64>().is_ok_and(|n| n == issue_number)
                    && !tail_after.starts_with(|c: char| c.is_ascii_digit())
                {
                    return Some(ReferenceForm::Escaped);
                }
            }
        }
    }

    // URL: an http(s) token containing /issues/N. The scheme must follow
    // the verb directly (whitespace allowed); "closed https://…" must not
    // match on the shorter verb "close" (its remainder is "d https://…").
    let lower = after_ws.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let token = after_ws
            .split(|c: char| c.is_whitespace() || c == ')')
            .next()
            .unwrap_or("");
        let tl = token.to_ascii_lowercase();
        if let Some(pos) = tl.find("/issues/") {
            let after = &tl[pos + "/issues/".len()..];
            if let Some((digits, tail_after)) = split_number(after) {
                if digits.parse::<u64>().is_ok_and(|n| n == issue_number)
                    && !tail_after.starts_with(|c: char| c.is_ascii_digit())
                {
                    return Some(ReferenceForm::Url);
                }
            }
        }
    }

    None
}

/// Split a leading run of digits from the remainder.
fn split_number(s: &str) -> Option<(&str, &str)> {
    let n = s
        .len()
        .min(s.bytes().take_while(|b| b.is_ascii_digit()).count());
    (n > 0).then(|| (&s[..n], &s[n..]))
}
