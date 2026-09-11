//! A spec states one outcome (issue #4333).
//!
//! Three incidents, one shape. A spec said `Fix the failing lints or file
//! issues for the ones that need redesign` — the agent filed the issues.
//! Another said `Add a baseline for the failing tests or fix the tests` —
//! the agent baselined. Both specs named a repair and, after an `or`, a
//! cheaper additive alternative, and the agent — which implements the
//! smallest thing the spec can be read as permitting — took the cheaper
//! branch every time. The `or` was a scope decision the spec was dodging
//! by deferring to the implementer.
//!
//! The one spec that behaved was the exception that explains the rule:
//! `Add a test file for the gate. Or, if the gate cannot distinguish a
//! file the gate runs from a file it does not, say so and name the commit`
//! — its fallback was guarded by a condition *and* required the
//! implementer to say so with evidence. The fallback was a justification,
//! not a substitute for the repair.
//!
//! And the closing half: an issue titled `rust-suites is 0/40 on main`
//! carried an acceptance criterion that checked the mechanism (`the test
//! file exists`) rather than the symptom. The patch merged, the gate was
//! still red, and nothing reopened anything — the closeout had asserted a
//! file, not the symptom the title reported.
//!
//! Five rules, each checkable:
//!
//! 1. **One outcome per spec.** Two acceptable paths are two issues: the
//!    repair as one, the workaround as a follow-up with its own
//!    acceptance criteria. The mechanical core of this rule is rule 2 —
//!    the `or` between a repair and a workaround is how a second outcome
//!    sneaks in ([`classify_alternative`]).
//! 2. **Never an `or` between a fix and a workaround.** `Fix the lints or
//!    file the issues` is a scope decision, not a spec. One branch
//!    classified as a repair and the other as an accept/record is an
//!    escape hatch, in either order ([`Alternative`],
//!    [`AlternativeVerdict::EscapeHatch`]).
//! 3. **A fallback is acceptable only with a condition and a
//!    justification.** `or, if the gate cannot distinguish X from Y, say
//!    so and name the commit` names the condition *and* requires evidence
//!    — the primary outcome is still the repair. A condition that names no
//!    way to say so is an escape hatch with a gate on it
//!    ([`AlternativeVerdict::GuardedFallback`]).
//! 4. **Acceptance criteria re-assert the title's symptom.** If the title
//!    carries a measurable symptom (`rust-suites is 0/40 on main`), at
//!    least one AC must check that the symptom is gone (`40/40`, `100%`,
//!    the subject passing) — not only the mechanism (`the test file
//!    exists`) ([`title_symptom`], [`ac_reasserts_symptom`]).
//! 5. **The post-merge check re-asserts the symptom, and a persisting
//!    symptom reopens the issue.** Merged is not fixed: the observation
//!    after merge is read against the title's measurement, and a symptom
//!    that is still present — or cannot be shown gone — is never read as
//!    resolved ([`recheck_symptom`], [`post_merge_action`]).

use std::fmt;

/// A repair verb: the branch does the work the issue exists for.
const FIX_VERBS: &[&str] = &[
    "fix", "fixed", "fixes", "fixing", "repair", "repairs", "repairing", "resolve", "resolves",
    "correct", "corrects", "correcting", "eliminate", "eliminates", "patch", "patches",
];

/// A workaround verb: the branch records, tolerates, or defers the defect
/// instead of removing it — the cheaper, additive path.
const WORKAROUND_VERBS: &[&str] = &[
    "record", "records", "recording", "baseline", "baselining", "waive", "waives", "waiving",
    "allow", "allows", "allowing", "skip", "skips", "skipping", "ignore", "ignores", "ignoring",
    "suppress", "suppresses", "tolerate", "tolerates", "document", "documents", "leave",
    "leaves", "accept", "accepts", "file", "files", "filing",
];

/// An evidence verb: the branch requires the implementer to say so —
/// report, name, state — which is what turns a guarded fallback into a
/// justification rather than a substitute.
const EVIDENCE_VERBS: &[&str] = &["say", "says", "state", "states", "report", "reports", "name", "names"];

/// The kind of work a clause of an `or` commits to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    /// The clause does the repair.
    Fix,
    /// The clause records, tolerates, or defers the defect.
    Workaround,
    /// The clause contains neither a repair verb nor a workaround verb:
    /// this cannot classify it, so it does not guess.
    Neutral,
}

/// Classify one clause by its verbs. A clause carrying a workaround verb
/// is a workaround even if it also carries a repair verb — the cheaper
/// path is present, and the conservative reading flags it.
pub fn classify_branch(clause: &str) -> BranchKind {
    let lower = clause.to_lowercase();
    if WORKAROUND_VERBS.iter().any(|v| contains_word(&lower, v)) {
        return BranchKind::Workaround;
    }
    if FIX_VERBS.iter().any(|v| contains_word(&lower, v)) {
        return BranchKind::Fix;
    }
    BranchKind::Neutral
}

/// True when `word` occurs in `lower` (already lowercased) as a whole
/// word: not part of a longer token, so `error` does not match `or`.
fn contains_word(lower: &str, word: &str) -> bool {
    lower.match_indices(word).any(|(i, _)| {
        let before_ok = i == 0 || !lower.as_bytes()[i - 1].is_ascii_alphanumeric();
        let after = i + word.len();
        let after_ok = after >= lower.len() || !lower.as_bytes()[after].is_ascii_alphanumeric();
        before_ok && after_ok
    })
}

/// One `or` in a spec's text: the left branch, the right branch, and —
/// when the right branch is guarded — the condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alternative {
    /// The left branch, trimmed of sentence punctuation. A leading
    /// `either` is stripped.
    pub left: String,
    /// The right branch. For a guarded fallback this is the action after
    /// the condition (`say so and name the commit`), not the condition.
    pub right: String,
    /// The guard, present when the right branch reads `if <condition>,
    /// <action>`.
    pub condition: Option<String>,
}

/// The characters that bound a branch: an `or` joins clauses inside a
/// sentence, and a new sentence is a new statement.
const SENTENCE_ENDS: &[char] = &['.', '!', '?', ';', '\n'];

/// Find every `or` in the text and split it into branches. Each `or` is
/// scanned independently, so `A or B or C` yields both alternatives. The
/// word `or` inside a longer token (`error`, `orphan`) is not an
/// alternative.
pub fn find_alternatives(text: &str) -> Vec<Alternative> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    for (s, e) in word_spans(&lower, "or") {
        let left = left_branch(text, s);
        if left.is_empty() {
            continue;
        }
        let right_end = text[e..]
            .find(SENTENCE_ENDS)
            .map(|i| e + i)
            .unwrap_or(text.len());
        let mut right = text[e..right_end].trim().trim_start_matches(',').trim();

        // `or, if <condition>, <action>` — the guard belongs to the
        // fallback, and only the action is what the implementer would do.
        let condition = if let Some(rest) = right.strip_prefix("if ") {
            match rest.find(',') {
                Some(c) => {
                    let cond = rest[..c].trim();
                    right = rest[c + 1..].trim();
                    Some(cond.to_string())
                }
                None => {
                    // `or if <condition>` with no action: the condition is
                    // the whole right branch and there is no fallback to
                    // classify.
                    right = "";
                    Some(rest.trim().to_string())
                }
            }
        } else {
            None
        };

        out.push(Alternative {
            left: strip_either(&left),
            right: right.to_string(),
            condition,
        });
    }
    out
}

/// The byte spans of whole-word occurrences of `word` in `lower`.
fn word_spans(lower: &str, word: &str) -> Vec<(usize, usize)> {
    lower
        .match_indices(word)
        .filter(|(i, _)| {
            let before_ok = *i == 0 || !lower.as_bytes()[*i - 1].is_ascii_alphanumeric();
            let after = *i + word.len();
            let after_ok = after >= lower.len() || !lower.as_bytes()[after].is_ascii_alphanumeric();
            before_ok && after_ok
        })
        .map(|(i, _)| (i, i + word.len()))
        .collect()
}

/// The left branch of the `or` at byte offset `s`: the text since the
/// previous sentence boundary. When the `or` opens a new sentence (`Add a
/// test file. Or, if …`), the branch is the *preceding* sentence — the
/// alternative is between that statement and the fallback.
fn left_branch(text: &str, s: usize) -> String {
    let before = &text[..s];
    let ends: Vec<usize> = before.match_indices(SENTENCE_ENDS).map(|(i, _)| i + 1).collect();
    let last = ends.last().copied().unwrap_or(0);
    let candidate = before
        .trim_end_matches(['.', '!', '?', ',', ':'])
        .trim();
    let since_last = before[last..].trim().trim_end_matches(['.', '!', '?', ',', ':']).trim();
    if !since_last.is_empty() {
        return since_last.to_string();
    }
    // The `or` sits at the start of a sentence: the branch is the whole
    // preceding sentence.
    if candidate.is_empty() {
        return String::new();
    }
    if ends.len() >= 2 {
        let start = ends[ends.len() - 2];
        return before[start..last - 1]
            .trim()
            .trim_end_matches(['.', '!', '?', ',', ':'])
            .trim()
            .to_string();
    }
    candidate.to_string()
}

/// Strip a leading `either` — `either fix it or record it` is the same
/// alternative with a stricter frame.
fn strip_either(left: &str) -> String {
    let lower = left.to_lowercase();
    if lower.starts_with("either ") {
        return left["either ".len()..].trim().to_string();
    }
    left.to_string()
}

/// What an alternative does to the spec's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlternativeVerdict {
    /// An `or` between a repair and a workaround, in either order, with
    /// no guard — or a guarded fallback that never requires the
    /// implementer to say so. The implementer may choose the cheaper,
    /// additive outcome; the spec deferred a scope decision to it.
    EscapeHatch,
    /// The fallback is guarded by a condition *and* requires evidence
    /// (`say so and name the commit`). The primary outcome is still the
    /// repair; the fallback is a justification, not a substitute.
    GuardedFallback,
    /// At least one branch is unclassifiable, or both branches are
    /// repairs: this does not flag what it cannot classify.
    NotAnHatch,
}

/// Rules 1–3: classify an alternative. A conditional branch is a
/// `GuardedFallback` only when its action carries an evidence verb; a
/// condition that names no way to say so is still an escape hatch.
pub fn classify_alternative(alt: &Alternative) -> AlternativeVerdict {
    match &alt.condition {
        Some(_) => {
            if EVIDENCE_VERBS
                .iter()
                .any(|v| contains_word(&alt.right.to_lowercase(), v))
            {
                AlternativeVerdict::GuardedFallback
            } else {
                AlternativeVerdict::EscapeHatch
            }
        }
        None => match (classify_branch(&alt.left), classify_branch(&alt.right)) {
            (BranchKind::Fix, BranchKind::Workaround) | (BranchKind::Workaround, BranchKind::Fix) => {
                AlternativeVerdict::EscapeHatch
            }
            _ => AlternativeVerdict::NotAnHatch,
        },
    }
}

/// The rendered line for a verdict: a hatch says why it is one and what
/// the remedy is; a guarded fallback says what makes it acceptable.
pub fn alternative_line(alt: &Alternative, verdict: AlternativeVerdict) -> String {
    match verdict {
        AlternativeVerdict::EscapeHatch => match &alt.condition {
            Some(cond) => format!(
                "'or, if {cond}, {}': a guarded fallback that never requires the implementer \
                 to say so is an escape hatch with a gate on it — name the evidence the \
                 fallback must produce, or drop it (one outcome per spec, #4333)",
                alt.right
            ),
            None => format!(
                "'{} or {}': an unconditional or between a repair and a workaround — an agent \
                 implements the smallest thing the spec can be read as permitting, and the \
                 workaround is the smaller thing. One outcome per spec: ship the repair, and \
                 file the workaround as its own issue (#4333)",
                alt.left, alt.right
            ),
        },
        AlternativeVerdict::GuardedFallback => format!(
            "'or, if {}, {}': the fallback names its condition and requires the implementer \
             to say so — a justification, not a substitute for the repair",
            alt.condition.as_deref().unwrap_or(""),
            alt.right
        ),
        AlternativeVerdict::NotAnHatch => format!(
            "'{} or {}': no repair/workaround pair detected — not flagged",
            alt.left, alt.right
        ),
    }
}

/// One spec section and its text, as an audit input. `section` is the
/// section name the finding must carry (`Goal`, `Acceptance criteria`,
/// `Implementation outline`).
pub type Section = (&'static str, String);

/// One audit finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecFinding {
    /// The rule id: `SPEC_ESCAPE_HATCH`.
    pub rule: &'static str,
    /// The section the hatch was found in.
    pub section: &'static str,
    /// The rendered [`alternative_line`].
    pub detail: String,
}

/// Audit the outcome-bearing sections of a spec body. Clean sections —
/// no `or` at all, or an `or` that joins two repairs — produce no
/// findings.
pub fn audit_spec(sections: &[Section]) -> Vec<SpecFinding> {
    let mut findings = Vec::new();
    for (section, text) in sections {
        for alt in find_alternatives(text) {
            if classify_alternative(&alt) == AlternativeVerdict::EscapeHatch {
                findings.push(SpecFinding {
                    rule: "SPEC_ESCAPE_HATCH",
                    section: *section,
                    detail: alternative_line(&alt, AlternativeVerdict::EscapeHatch),
                });
            }
        }
    }
    findings
}

/// A measurable symptom in an issue title: a `reported/total` fraction
/// (`rust-suites is 0/40 on main` → `0` of `40`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symptom {
    /// The subject the measurement hangs off — the title text before the
    /// fraction, trimmed of a trailing copula (`rust-suites`).
    pub subject: String,
    /// The numerator as reported in the title.
    pub reported: u32,
    /// The denominator: the population the numerator is out of.
    pub total: u32,
}

impl std::fmt::Display for Symptom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.subject.is_empty() {
            write!(f, "{}/{}", self.reported, self.total)
        } else {
            write!(f, "{} {}/{}", self.subject, self.reported, self.total)
        }
    }
}

/// All `N/M` fractions in `text`, with the byte offset of each match. A
/// fraction is two digit runs over a single `/`; a URL path or a second
/// `/` closes the match.
fn find_fractions(text: &str) -> Vec<(u32, u32, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'/' {
                let j = i + 1;
                if j < bytes.len() && bytes[j].is_ascii_digit() {
                    let dstart = j;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                    let trailing_ok = j >= bytes.len() || (!bytes[j].is_ascii_digit() && bytes[j] != b'/');
                    if trailing_ok {
                        let n: u32 = text[start..i].parse().unwrap_or(0);
                        let m: u32 = text[dstart..j].parse().unwrap_or(0);
                        out.push((n, m, start));
                    }
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Extract the title's measurable symptom, if it has one: the first
/// `N/M` fraction with `M > 0` and `N <= M`. A title without a fraction
/// has no measurable symptom and is not checkable against its ACs —
/// `None`, never a guess.
pub fn title_symptom(title: &str) -> Option<Symptom> {
    for (n, m, at) in find_fractions(title) {
        if m > 0 && n <= m {
            let before = title[..at].trim().trim_end_matches(['.', '!', '?', ',', ':']).trim();
            let subject = before
                .strip_suffix(" is")
                .or_else(|| before.strip_suffix(" was"))
                .map(str::trim)
                .unwrap_or(before)
                .to_string();
            return Some(Symptom {
                subject: subject.to_string(),
                reported: n,
                total: m,
            });
        }
    }
    None
}

/// Rule 4's outcome: does the AC list re-assert the title's symptom?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reassert {
    /// At least one AC checks that the symptom is gone: the full
    /// fraction (`40/40`), `100%`, or the subject's words together with
    /// a resolution word (`rust-suites passes`).
    Reasserted,
    /// No AC re-asserts the symptom — every AC checks only the mechanism
    /// (`the test file exists`). The patch can merge and the symptom can
    /// persist, and nothing would say so.
    NotReasserted,
}

/// Words that say the symptom is gone, when they sit next to the
/// subject's own words.
const RESOLUTION_WORDS: &[&str] = &["passes", "passed", "green", "fixed", "resolved"];

/// Rule 4: at least one acceptance criterion must check the resolved
/// symptom, not only the mechanism. A title with no measurable symptom
/// is the caller's business (`title_symptom` returned `None`).
pub fn ac_reasserts_symptom(symptom: &Symptom, acs: &[&str]) -> Reassert {
    let full = format!("{}/{}", symptom.total, symptom.total);
    let subject_words: Vec<&str> = symptom
        .subject
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 2)
        .collect();
    for ac in acs {
        let lower = ac.to_lowercase();
        if lower.contains(&full) || lower.contains("100%") {
            return Reassert::Reasserted;
        }
        if !subject_words.is_empty()
            && subject_words.iter().all(|w| contains_word(&lower, w))
            && RESOLUTION_WORDS
                .iter()
                .any(|r| contains_word(&lower, r))
        {
            return Reassert::Reasserted;
        }
    }
    Reassert::NotReasserted
}

/// The rendered line for rule 4.
pub fn reassert_line(symptom: &Symptom, verdict: Reassert) -> String {
    match verdict {
        Reassert::Reasserted => format!(
            "the acceptance criteria re-assert the title's symptom '{symptom}': a merged patch \
             that leaves it in place would fail its own AC"
        ),
        Reassert::NotReasserted => format!(
            "no acceptance criterion re-asserts the title's symptom '{symptom}': the ACs check \
             the mechanism, not the symptom — an AC must assert the symptom is gone \
             (e.g. '{}/{}'), or the patch can merge and the issue's own failure can persist \
             silently (#4333)",
            symptom.total, symptom.total
        ),
    }
}

/// Rule 5's outcome: the post-merge observation read against the
/// title's symptom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recheck {
    /// The observation shows the symptom cleared: a full fraction
    /// (`40/40`) or `100%`.
    Resolved,
    /// The observation shows the symptom still present: a fraction of
    /// the same total below full (`0/40`, `18/40`).
    Persists,
    /// The observation contains no measurement of this total — this
    /// cannot tell whether the symptom cleared.
    Unverifiable,
}

/// Rule 5: read the post-merge observation against the title's symptom.
/// A fraction with a *different* denominator is not evidence about this
/// symptom; absence of a fraction is `Unverifiable`, never `Resolved`.
pub fn recheck_symptom(symptom: &Symptom, observation: &str) -> Recheck {
    let mut saw_partial = false;
    for (n, m, _) in find_fractions(observation) {
        if m == symptom.total {
            if n == m {
                return Recheck::Resolved;
            }
            if n < m {
                saw_partial = true;
            }
        }
    }
    if observation.to_lowercase().contains("100%") {
        return Recheck::Resolved;
    }
    if saw_partial {
        Recheck::Persists
    } else {
        Recheck::Unverifiable
    }
}

/// What a closeout does with the issue, given the recheck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostMergeAction {
    /// The symptom is shown gone: the issue may close.
    Close,
    /// The symptom persists after merge: the issue reopens — the patch
    /// fixed the mechanism, not the issue.
    Reopen,
    /// The observation cannot verify the symptom: fail-closed, the issue
    /// holds — never read as resolved.
    Hold,
}

/// Rule 5: merged is not fixed. Only a `Resolved` recheck closes; a
/// persisting symptom reopens; an unverifiable one holds.
pub fn post_merge_action(recheck: Recheck) -> PostMergeAction {
    match recheck {
        Recheck::Resolved => PostMergeAction::Close,
        Recheck::Persists => PostMergeAction::Reopen,
        Recheck::Unverifiable => PostMergeAction::Hold,
    }
}

/// The rendered line for rule 5: it says what the observation showed and
/// what the issue does next.
pub fn recheck_line(symptom: &Symptom, recheck: Recheck, observation: &str) -> String {
    match recheck {
        Recheck::Resolved => format!(
            "symptom '{symptom}' cleared after merge ({observation}): the issue may close"
        ),
        Recheck::Persists => format!(
            "symptom '{symptom}' persists after merge ({observation}): the patch fixed the \
             mechanism, not the issue — reopen (#4333)"
        ),
        Recheck::Unverifiable => format!(
            "symptom '{symptom}' unverifiable from '{observation}': fail-closed — the issue \
             holds, it is not read as resolved (#4333)"
        ),
    }
}
