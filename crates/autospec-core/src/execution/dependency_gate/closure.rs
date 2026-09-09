//! Rule 3 — closing-keyword enforcement.
//!
//! An issue is closed by the change that resolves it, and the link between the
//! two is a closing keyword: `Closes #N`, `Fixes #N`, `Resolves #N` in the
//! merged change. Without that requirement an integration step that merely
//! *followed* an issue closes it, and the issue's own acceptance criteria are
//! then unverifiable from history — the ledger says done, the diff says nothing.
//!
//! This module is the authority for that decision, so that the close path can
//! ask one question and honour the answer:
//! [`authorize_closure`] returns either an [`Authorized`] carrying the
//! directive it found, or a [`RefusedClosure`] with code
//! `REFUSED-AUTO-CLOSE` — a refusal being *expected* whenever a PR is merged
//! that does not resolve the issue, and therefore never an error.
//!
//! The grammar is the existing one (`contains_issue_closing_directive` in the
//! executor bridge, which validates closeout *text*), pinned here with the
//! reference forms spelled out:
//!
//! 1. The keyword is a word on its own, case-insensitive: `closes`, `fixes`,
//!    `resolves`. `never closes the gap` contains the word but asserts nothing,
//!    so only the keyword's position relative to a reference is checked, not
//!    the sentiment around it.
//! 2. A reference is `#N`, `owner/repo#N`, or an issue URL ending
//!    `/issues/N` / `/issue/N`. A bare number is never a reference — the
//!    sentence "fixes 3 flaky specs" closes nothing.
//! 3. Qualifying phrases are not closing: `part of #42`, `follow-up to #42`,
//!    `blocks #42`, `see #42`. Only the three keywords bind.
//! 4. The reference must name the issue being closed; `Closes #7` does not
//!    authorize closing #42.

use std::fmt;

/// One of the three keywords that bind a change to an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClosingKeyword {
    Closes,
    Fixes,
    Resolves,
}

impl ClosingKeyword {
    /// Every closing keyword.
    pub const ALL: [Self; 3] = [Self::Closes, Self::Fixes, Self::Resolves];

    /// The canonical capitalized spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Closes => "Closes",
            Self::Fixes => "Fixes",
            Self::Resolves => "Resolves",
        }
    }

    /// The lowercase keyword as matched in text.
    fn token(self) -> &'static str {
        match self {
            Self::Closes => "closes",
            Self::Fixes => "fixes",
            Self::Resolves => "resolves",
        }
    }
}

impl fmt::Display for ClosingKeyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A closing keyword bound to an issue reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosingDirective {
    /// The keyword used.
    pub keyword: ClosingKeyword,
    /// The issue the directive names.
    pub issue: u64,
    /// The `owner/repo` for a cross-repository reference.
    pub repo: Option<String>,
    /// The matched source label, e.g. `commit` or `pull-request-body`.
    pub source: &'static str,
}

impl ClosingDirective {
    /// The directive rendered the way a change should write it.
    pub fn render(&self) -> String {
        let reference = match &self.repo {
            Some(repo) => format!("{repo}#{}", self.issue),
            None => format!("#{}", self.issue),
        };
        format!("{} {}", self.keyword.as_str(), reference)
    }
}

impl fmt::Display for ClosingDirective {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (in {})", self.render(), self.source)
    }
}

/// The text a merged change offers as evidence that it resolves an issue.
///
/// A merged change carries its message in more than one place — the commit
/// message, the squash message derived from the PR title, the PR body. A
/// directive found in any of them counts, because GitHub's own auto-close
/// reads the same set; requiring the commit message alone would refuse closes
/// that are genuinely linked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeEvidence {
    parts: Vec<(&'static str, String)>,
}

impl ChangeEvidence {
    /// Empty evidence: nothing has been offered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a commit message.
    pub fn commit(mut self, message: impl Into<String>) -> Self {
        self.parts.push(("commit", message.into()));
        self
    }

    /// Add a pull request body.
    pub fn pull_request(mut self, body: impl Into<String>) -> Self {
        self.parts.push(("pull-request-body", body.into()));
        self
    }

    /// Add a squash/merge message.
    pub fn merge_message(mut self, message: impl Into<String>) -> Self {
        self.parts.push(("merge-message", message.into()));
        self
    }

    /// The source labels offered, in insertion order.
    pub fn sources(&self) -> Vec<&'static str> {
        self.parts.iter().map(|(label, _)| *label).collect()
    }

    /// True when no text at all was offered.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

/// Whether an issue may be closed by a merged change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClosureAuthorization {
    /// A directive naming this issue was found; closing is authorized.
    Authorized {
        /// The issue that may be closed.
        issue: u64,
        /// The directive that authorizes it.
        directive: ClosingDirective,
    },
    /// No directive naming this issue was found; do not close.
    Refused(RefusedClosure),
}

impl ClosureAuthorization {
    /// True when the close may proceed.
    pub fn authorized(&self) -> bool {
        matches!(self, Self::Authorized { .. })
    }

    /// The directive, when authorized.
    pub fn directive(&self) -> Option<&ClosingDirective> {
        match self {
            Self::Authorized { directive, .. } => Some(directive),
            Self::Refused(_) => None,
        }
    }

    /// The refusal, when refused.
    pub fn refusal(&self) -> Option<&RefusedClosure> {
        match self {
            Self::Authorized { .. } => None,
            Self::Refused(refused) => Some(refused),
        }
    }

    /// The wire code: none when authorized.
    pub fn code(&self) -> Option<&'static str> {
        self.refusal().map(RefusedClosure::code)
    }

    /// One-line report, so the close path logs the reason either way.
    pub fn report(&self) -> String {
        match self {
            Self::Authorized { directive, .. } => {
                format!("auto-close=authorized {directive}")
            }
            Self::Refused(refused) => refused.report(),
        }
    }
}

/// A close that is not authorized: the merged change says nothing about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedClosure {
    issue: u64,
    offered: Vec<&'static str>,
    named: Vec<u64>,
}

impl RefusedClosure {
    /// The wire code, carried in the monitor log and on the issue.
    pub const CODE: &'static str = "REFUSED-AUTO-CLOSE";

    /// The issue that stayed open.
    pub fn issue(&self) -> u64 {
        self.issue
    }

    /// Which texts were examined.
    pub fn offered(&self) -> &[&'static str] {
        &self.offered
    }

    /// Other issues the change *does* declare it closes. A non-empty list here
    /// is the interesting case: the directive is present but points elsewhere.
    pub fn named_issues(&self) -> &[u64] {
        &self.named
    }

    /// The code string, for callers holding a reference.
    pub fn code(&self) -> &'static str {
        Self::CODE
    }

    /// One-line report naming the code and what was looked at.
    pub fn report(&self) -> String {
        let issue = self.issue;
        let offered = if self.offered.is_empty() {
            "none".to_string()
        } else {
            self.offered.join(",")
        };
        let named = if self.named.is_empty() {
            "none".to_string()
        } else {
            self.named
                .iter()
                .map(|issue| issue.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "{code} issue={issue} looked-in={offered} closes-other-than={named} issue-stays-open",
            code = self.code()
        )
    }
}

/// Decide whether `change` may close `issue`.
pub fn authorize_closure(issue: u64, change: &ChangeEvidence) -> ClosureAuthorization {
    let mut named = Vec::new();
    for (label, text) in &change.parts {
        for directive in find_closing_directives(text) {
            if directive.issue == issue {
                return ClosureAuthorization::Authorized {
                    issue,
                    directive: ClosingDirective {
                        source: label,
                        ..directive
                    },
                };
            }
            named.push(directive.issue);
        }
    }
    named.sort_unstable();
    named.dedup();
    ClosureAuthorization::Refused(RefusedClosure {
        issue,
        offered: change.sources(),
        named,
    })
}

/// True when `change` carries a directive naming `issue`.
pub fn merged_change_closes(issue: u64, change: &ChangeEvidence) -> bool {
    authorize_closure(issue, change).authorized()
}

/// Every closing directive in `text`, in order of appearance.
pub fn find_closing_directives(text: &str) -> Vec<ClosingDirective> {
    let mut found = Vec::new();
    for line in text.lines() {
        let line = strip_code_span(line);
        let lower = line.to_ascii_lowercase();
        for keyword in ClosingKeyword::ALL {
            for (offset, _) in lower.match_indices(keyword.token()) {
                if !word_bound(&lower, offset, keyword.token().len()) {
                    continue;
                }
                let after = keyword.token().len();
                let Some((issue, _)) = parse_reference(&lower[offset + after..]) else {
                    continue;
                };
                // Re-read the reference from the original-cased text so owner
                // names keep their spelling. `to_ascii_lowercase` only alters
                // ASCII bytes, so both slices share byte offsets.
                let repo = parse_reference(&line[offset + after..]).and_then(|(_, r)| r);
                found.push(ClosingDirective {
                    keyword,
                    issue,
                    repo,
                    source: "text",
                });
            }
        }
    }
    found
}

/// True when `keyword` at `offset` stands alone.
fn word_bound(haystack: &str, offset: usize, len: usize) -> bool {
    let before = haystack[..offset].chars().next_back();
    let after = haystack[offset + len..].chars().next();
    let boundary = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
    boundary(before) && boundary(after)
}

/// Parse the reference that follows a keyword.
///
/// Accepted: ` #123`, `: #123`, ` owner/repo#123`, `: https://host/o/r/issues/123`.
/// Rejected: a bare number, a `#` with no digits, anything not separated from
/// the keyword by whitespace.
fn parse_reference(rest: &str) -> Option<(u64, Option<String>)> {
    // A reference is separated from its keyword by whitespace or a colon.
    // "closes42" and "closesonly #42" are not directives.
    if !rest.is_empty() && !rest.starts_with([' ', '\t', ':', ',']) {
        return None;
    }
    let rest = rest.trim_start_matches([' ', '\t', ':', ',']);
    let token = rest
        .split([' ', '\t', ',', ';', ')', ']', '<', '>'])
        .next()
        .filter(|token| !token.is_empty())?
        .trim_end_matches(['.', ':', '!', '?', '\'', '"', '`']);

    if token.contains("://") {
        let number = trailing_issue_number(token)?;
        return Some((number, repo_from_url(token)));
    }

    // `#N` or `owner/repo#N`. A bare number never reaches here: no `#`, no reference.
    let (prefix, digits) = token.split_once('#')?;
    let repo = if prefix.is_empty() {
        None
    } else if is_repo_slug(prefix) {
        Some(prefix.to_string())
    } else {
        return None;
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().map(|number| (number, repo))
}

fn is_repo_slug(prefix: &str) -> bool {
    let Some((owner, name)) = prefix.rsplit_once('/') else {
        return false;
    };
    !owner.is_empty()
        && !name.is_empty()
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The `owner/repo` an issue URL points at.
fn repo_from_url(url: &str) -> Option<String> {
    let after_host = url.split_once("://")?.1.split_once('/')?.1;
    let mut segments = after_host.split('/');
    let owner = segments.next()?;
    let repo = segments.next()?.trim_end_matches(".git");
    is_repo_slug(&format!("{owner}/{repo}")).then(|| format!("{owner}/{repo}"))
}

/// The number at the end of `/issues/N` or `/issue/N`.
fn trailing_issue_number(text: &str) -> Option<u64> {
    let tail = text
        .trim_end_matches(['.', ')', ']', ','])
        .rsplit('/')
        .next()?;
    if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    tail.parse().ok()
}

/// Drop a trailing inline-code span, so `` `Closes #1` `` in a template is not
/// read as a directive from prose about the directive.
fn strip_code_span(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_span = false;
    for ch in line.chars() {
        if ch == '`' {
            in_span = !in_span;
            continue;
        }
        if !in_span {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directive(text: &str, issue: u64) -> ClosureAuthorization {
        authorize_closure(issue, &ChangeEvidence::new().commit(text))
    }

    #[test]
    fn each_keyword_authorizes_the_issue_it_names() {
        for keyword in ClosingKeyword::ALL {
            let line = format!("{keyword} #42 in the merged change");
            let authorization = directive(&line, 42);
            assert!(authorization.authorized(), "{line}");
            assert_eq!(authorization.code(), None);
            let found = authorization.directive().expect("directive");
            assert_eq!(found.keyword, keyword);
            assert_eq!(found.issue, 42);
            assert_eq!(found.repo, None);
            assert_eq!(found.render(), format!("{} #42", keyword.as_str()));
            assert_eq!(found.source, "commit");
        }
    }

    #[test]
    fn a_change_without_a_keyword_is_refused() {
        // The shape of the incident: an integration PR merged on top of an
        // issue, with no statement that it resolves it.
        let authorization = directive("feat: implement the compose probe", 42);
        assert!(!authorization.authorized());
        assert_eq!(authorization.code(), Some(RefusedClosure::CODE));
        let refused = authorization.refusal().expect("refusal");
        assert_eq!(refused.issue(), 42);
        assert_eq!(refused.offered(), &["commit"]);
        assert!(refused.named_issues().is_empty());
        assert_eq!(
            refused.report(),
            "REFUSED-AUTO-CLOSE issue=42 looked-in=commit closes-other-than=none issue-stays-open"
        );
    }

    #[test]
    fn a_directive_for_another_issue_does_not_authorize_this_one() {
        let authorization = directive("Fixes #7 while wiring the probe", 42);
        assert!(!authorization.authorized());
        let refused = authorization.refusal().expect("refusal");
        assert_eq!(refused.named_issues(), &[7]);
        assert!(refused.report().contains("closes-other-than=7"));
        assert!(refused.report().contains("issue-stays-open"));
    }

    #[test]
    fn a_bare_number_is_not_a_reference() {
        assert!(!directive("fixes 3 flaky specs", 3).authorized());
        assert!(!directive("resolves 42 flakes per run", 42).authorized());
    }

    #[test]
    fn qualifying_phrases_are_not_closing() {
        for line in [
            "part of #42",
            "follow-up to #42",
            "blocks #42",
            "see #42",
            "related to #42",
            "supersedes #42",
        ] {
            assert!(
                !directive(line, 42).authorized(),
                "{line} must not authorize a close"
            );
        }
    }

    #[test]
    fn keyword_position_is_matched_not_sentiment() {
        // "never closes the gap" has no reference, so nothing binds. But the
        // matcher is positional: a sentence that denies the close still binds
        // if it carries the reference, exactly as GitHub's does.
        assert!(!directive("this never closes the gap", 42).authorized());
        assert!(directive("note: this does not close, Closes #42", 42).authorized());
    }

    #[test]
    fn keyword_must_be_a_whole_word() {
        assert!(!directive("closeset #42", 42).authorized());
        assert!(!directive("recloses #42", 42).authorized());
        assert!(directive("CLOSES #42", 42).authorized());
        assert!(directive("fixes:#42", 42).authorized());
        assert!(directive("Resolves #42.", 42).authorized());
    }

    #[test]
    fn cross_repository_and_url_references_are_parsed() {
        let authorization = directive("Fixes berlinguyinca/autospec#42", 42);
        let found = authorization.directive().expect("authorized");
        assert_eq!(found.repo.as_deref(), Some("berlinguyinca/autospec"));
        assert_eq!(found.render(), "Fixes berlinguyinca/autospec#42");

        let authorization = directive(
            "Closes https://github.com/berlinguyinca/autospec/issues/42",
            42,
        );
        let found = authorization.directive().expect("authorized");
        assert_eq!(found.issue, 42);
        assert_eq!(found.repo.as_deref(), Some("berlinguyinca/autospec"));

        assert!(directive("Resolves https://github.com/o/r/issue/42.", 42).authorized());
    }

    #[test]
    fn a_reference_must_be_separated_from_its_keyword() {
        assert!(!directive("closes42", 42).authorized());
        assert!(!directive("closesonly #42", 42).authorized());
        assert!(directive("Closes: #42", 42).authorized());
        assert!(directive("fixes #42, #7", 42).authorized());
        assert!(directive("Resolves #42.", 42).authorized());
    }

    #[test]
    fn a_reference_without_an_issue_shape_is_not_a_reference() {
        assert!(!directive("Closes https://github.com/berlinguyinca/autospec", 42).authorized());
        assert!(!directive("Fixes #", 42).authorized());
        assert!(!directive("Fixes #abc", 42).authorized());
    }

    #[test]
    fn every_offered_text_is_searched_and_the_hit_labels_its_source() {
        let change = ChangeEvidence::new()
            .commit("feat: add the compose probe")
            .merge_message("feat: implement fixture (#42)")
            .pull_request("Runs the real engine on a runtime host.\n\nCloses #42\n");
        let authorization = authorize_closure(42, &change);
        assert!(authorization.authorized());
        assert_eq!(
            authorization.directive().expect("authorized").source,
            "pull-request-body"
        );
        assert_eq!(
            change.sources(),
            vec!["commit", "merge-message", "pull-request-body"]
        );
        assert!(merged_change_closes(42, &change));
        assert!(authorize_closure(99, &change)
            .report()
            .starts_with(RefusedClosure::CODE));
    }

    #[test]
    fn empty_evidence_is_refused_with_nothing_examined() {
        let authorization = authorize_closure(42, &ChangeEvidence::new());
        assert!(!authorization.authorized());
        assert!(authorization
            .refusal()
            .expect("refusal")
            .offered()
            .is_empty());
        assert!(authorization.report().contains("looked-in=none"));
    }

    #[test]
    fn directives_are_found_in_order_and_code_spans_are_ignored() {
        let text = "Closes #1 and Fixes #2\nThe template says `Closes #9`.\n";
        let found = find_closing_directives(text);
        assert_eq!(
            found
                .iter()
                .map(|d| (d.keyword, d.issue))
                .collect::<Vec<_>>(),
            vec![(ClosingKeyword::Closes, 1), (ClosingKeyword::Fixes, 2),]
        );
    }

    #[test]
    fn a_refusal_is_reported_not_raised() {
        // A merged PR that resolves nothing is a normal outcome, so the close
        // path gets a value, not an error to swallow.
        let authorization = authorize_closure(42, &ChangeEvidence::new().commit("chore: tidy"));
        assert!(matches!(authorization, ClosureAuthorization::Refused(_)));
        assert!(!authorization.authorized());
        assert_eq!(authorization.code(), Some(RefusedClosure::CODE));
    }
}
