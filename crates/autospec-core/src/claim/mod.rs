#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimState {
    Claimed,
    Merged,
}

impl ClaimState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ClaimState::Claimed => "claimed",
            ClaimState::Merged => "merged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimLease {
    pub issue_number: u64,
    pub worker_id: String,
    pub branch: String,
    pub claimed_at: u64,
    pub updated_at: u64,
    pub ttl_seconds: u64,
    pub state: ClaimState,
}

impl ClaimLease {
    pub fn new(
        issue_number: u64,
        worker_id: impl Into<String>,
        branch: impl Into<String>,
        claimed_at: u64,
        updated_at: u64,
        ttl_seconds: u64,
    ) -> Self {
        Self {
            issue_number,
            worker_id: worker_id.into(),
            branch: branch.into(),
            claimed_at,
            updated_at,
            ttl_seconds,
            state: ClaimState::Claimed,
        }
    }

    pub fn terminal_merged(
        issue_number: u64,
        worker_id: impl Into<String>,
        branch: impl Into<String>,
        merged_at: u64,
    ) -> Self {
        Self {
            issue_number,
            worker_id: worker_id.into(),
            branch: branch.into(),
            claimed_at: merged_at,
            updated_at: merged_at,
            ttl_seconds: 0,
            state: ClaimState::Merged,
        }
    }

    pub fn is_stale_at(&self, server_timestamp: u64) -> bool {
        if self.state == ClaimState::Merged {
            return false;
        }
        server_timestamp.saturating_sub(self.updated_at) > self.ttl_seconds
    }

    pub fn is_terminal(&self) -> bool {
        self.state == ClaimState::Merged
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaimRunState {
    pub lease: Option<ClaimLease>,
}

impl ClaimRunState {
    pub fn new(lease: Option<ClaimLease>) -> Self {
        Self { lease }
    }

    pub fn terminal_merged(
        issue_number: u64,
        worker_id: impl Into<String>,
        branch: impl Into<String>,
        merged_at: u64,
    ) -> Self {
        Self::new(Some(ClaimLease::terminal_merged(
            issue_number,
            worker_id,
            branch,
            merged_at,
        )))
    }

    pub fn accepts_claim(&self, incoming: &ClaimLease, server_timestamp: u64) -> bool {
        match &self.lease {
            None => true,
            Some(current) if current.is_terminal() => false,
            Some(current) if current.worker_id == incoming.worker_id => true,
            Some(current) => current.is_stale_at(server_timestamp),
        }
    }
}
use std::{collections::BTreeMap, fmt, ops::Range};

use crate::state::json::{JsonParser, JsonValue};

pub const RUN_STATE_BEGIN_MARKER: &str = "<!-- autospec-run-state:begin -->";
pub const RUN_STATE_END_MARKER: &str = "<!-- autospec-run-state:end -->";
pub const RUN_TERMINAL_BEGIN_MARKER: &str = "<!-- autospec-run-terminal:begin -->";
pub const RUN_TERMINAL_END_MARKER: &str = "<!-- autospec-run-terminal:end -->";
const SAFETY_BEGIN_MARKER: &str = "<!-- autospec-safety:begin -->";
const SAFETY_END_MARKER: &str = "<!-- autospec-safety:end -->";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimSafetyInput {
    pub labels: Vec<String>,
    pub title: String,
    pub body: String,
    pub author: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimIssueSnapshot {
    pub labels: Vec<String>,
    pub title: String,
    pub body: String,
    pub author: String,
}

impl ClaimIssueSnapshot {
    pub fn safety_input(&self) -> ClaimSafetyInput {
        ClaimSafetyInput::new(
            self.labels.clone(),
            self.title.clone(),
            self.body.clone(),
            self.author.clone(),
        )
    }
}

pub fn parse_claim_issue_json(input: &str) -> Result<ClaimIssueSnapshot, String> {
    let mut object = JsonParser::new(input)
        .parse()?
        .into_object("GitHub claim issue")?;
    require_only_keys(&object, &["labels", "title", "body", "author"])?;
    let labels = take_required(&mut object, "labels")?
        .into_array("GitHub claim issue labels")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| value.into_string(&format!("GitHub claim issue labels[{index}]")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ClaimIssueSnapshot {
        labels,
        title: take_optional_string(&mut object, "title", "GitHub claim issue")?
            .unwrap_or_default(),
        body: take_optional_string(&mut object, "body", "GitHub claim issue")?.unwrap_or_default(),
        author: take_optional_string(&mut object, "author", "GitHub claim issue")?
            .unwrap_or_default(),
    })
}

impl ClaimSafetyInput {
    pub fn new(
        labels: Vec<String>,
        title: impl Into<String>,
        body: impl Into<String>,
        author: impl Into<String>,
    ) -> Self {
        Self {
            labels,
            title: title.into(),
            body: body.into(),
            author: author.into(),
        }
    }
}

/// The only decision tokens that may be persisted in a safety-review marker.
///
/// The token is deliberately separate from queue eligibility: a pass may be
/// marked as reviewed, while ambiguous and blocked reviews require their own
/// human or quarantine handling in the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyReviewDecision {
    Pass,
    Ambiguous,
    Block,
}

impl SafetyReviewDecision {
    pub fn token(self) -> &'static str {
        match self {
            Self::Pass => "SAFETY_PASS",
            Self::Ambiguous => "SAFETY_AMBIGUOUS",
            Self::Block => "SAFETY_BLOCK",
        }
    }
}

/// A typed result of applying the current Rust issue-intent policy.
///
/// Findings are retained so the GitHub boundary can report the exact policy
/// evidence without parsing rendered Markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyReviewVerdict {
    pub decision: SafetyReviewDecision,
    pub findings: Vec<IssueIntentFinding>,
}

/// Errors returned instead of overwriting pre-existing safety-review evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyReviewSectionError {
    MalformedExistingSection,
    DuplicateExistingSection,
}

impl SafetyReviewSectionError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MalformedExistingSection => "malformed_existing_safety_review",
            Self::DuplicateExistingSection => "duplicate_existing_safety_review",
        }
    }
}

impl fmt::Display for SafetyReviewSectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for SafetyReviewSectionError {}

/// Evaluate an issue under the default trusted-actor policy and return the
/// strictest typed safety decision.
pub fn review_issue_safety(input: &ClaimSafetyInput) -> SafetyReviewVerdict {
    review_issue_safety_with_trusted_actors(input, &["berlinguyinca"])
}

/// Evaluate an issue under explicitly configured trusted actors.
///
/// A blocking finding always wins over ambiguous findings. The narrow trusted
/// test-reset exception remains owned by `lint_issue_intent_with_trusted_actors`.
pub fn review_issue_safety_with_trusted_actors(
    input: &ClaimSafetyInput,
    trusted_actors: &[&str],
) -> SafetyReviewVerdict {
    let lint = lint_issue_intent_with_trusted_actors(
        &input.title,
        &input.body,
        &input.author,
        trusted_actors,
    );
    let decision = if lint.blocking {
        SafetyReviewDecision::Block
    } else if lint.ambiguous {
        SafetyReviewDecision::Ambiguous
    } else {
        SafetyReviewDecision::Pass
    };
    SafetyReviewVerdict {
        decision,
        findings: lint.findings,
    }
}

/// Render exactly the Markdown section accepted by the Rust claim-safety
/// evaluator, using the supplied typed decision token.
pub fn render_safety_review_section(decision: SafetyReviewDecision) -> String {
    format!(
        "## Safety review\n\n{SAFETY_BEGIN_MARKER}\n- **decision:** `{}`\n{SAFETY_END_MARKER}",
        decision.token()
    )
}

/// Append a canonical safety review when no review evidence exists, or replace
/// the one existing canonical section. Any malformed or duplicate prior review
/// is an error: callers must not silently overwrite potentially conflicting
/// audit evidence.
pub fn replace_safety_review_section(
    body: &str,
    decision: SafetyReviewDecision,
) -> Result<String, SafetyReviewSectionError> {
    let replacement = render_safety_review_section(decision);
    match canonical_safety_review_bounds(body)? {
        Some(bounds) => Ok(format!(
            "{}{}{}",
            &body[..bounds.start],
            replacement,
            &body[bounds.end..]
        )),
        None if body.trim().is_empty() => Ok(format!("{replacement}\n")),
        None => Ok(format!("{}\n\n{replacement}\n", body.trim_end())),
    }
}

fn canonical_safety_review_bounds(
    body: &str,
) -> Result<Option<Range<usize>>, SafetyReviewSectionError> {
    let heading_positions = safety_review_heading_positions(body);
    let begin_count = body.matches(SAFETY_BEGIN_MARKER).count();
    let end_count = body.matches(SAFETY_END_MARKER).count();

    if heading_positions.is_empty() && begin_count == 0 && end_count == 0 {
        return Ok(None);
    }
    if heading_positions.len() > 1 || begin_count > 1 || end_count > 1 {
        return Err(SafetyReviewSectionError::DuplicateExistingSection);
    }
    if heading_positions.len() != 1 || begin_count != 1 || end_count != 1 {
        return Err(SafetyReviewSectionError::MalformedExistingSection);
    }

    let heading_start = heading_positions[0];
    let begin = body
        .find(SAFETY_BEGIN_MARKER)
        .expect("counted one safety begin marker");
    let end = body
        .find(SAFETY_END_MARKER)
        .expect("counted one safety end marker");
    if begin <= heading_start || end <= begin {
        return Err(SafetyReviewSectionError::MalformedExistingSection);
    }

    let section_end = end + SAFETY_END_MARKER.len();
    let section = &body[heading_start..section_end];
    let is_canonical = [
        SafetyReviewDecision::Pass,
        SafetyReviewDecision::Ambiguous,
        SafetyReviewDecision::Block,
    ]
    .into_iter()
    .any(|decision| section == render_safety_review_section(decision));
    if !is_canonical {
        return Err(SafetyReviewSectionError::MalformedExistingSection);
    }
    let post_section_end = section_end
        + next_markdown_heading_offset(&body[section_end..]).unwrap_or(body.len() - section_end);
    let post_section = &body[section_end..post_section_end];
    if !post_section.trim().is_empty() {
        return Err(SafetyReviewSectionError::MalformedExistingSection);
    }
    Ok(Some(heading_start..section_end))
}

fn safety_review_heading_positions(body: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let text = line.trim_end_matches('\n');
        if text.trim() == "## Safety review" {
            positions.push(offset);
        }
        offset += line.len();
    }
    positions
}

fn next_markdown_heading_offset(body: &str) -> Option<usize> {
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if line.starts_with("## ") {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimSafetyDecision {
    pub allowed: bool,
    pub reason: &'static str,
}

impl ClaimSafetyDecision {
    fn pass() -> Self {
        Self {
            allowed: true,
            reason: "pass",
        }
    }

    fn reject(reason: &'static str) -> Self {
        Self {
            allowed: false,
            reason,
        }
    }
}

/// Evaluate the fail-closed claim safety contract without executing a script or
/// trusting generated metadata. This deliberately checks the current issue
/// title/body after validating the exact reviewed marker block.
pub fn evaluate_claim_safety(input: &ClaimSafetyInput) -> ClaimSafetyDecision {
    evaluate_claim_safety_with_trusted_actors(input, &["berlinguyinca"])
}

/// Evaluate the claim safety contract with configured trusted actors. This is
/// intentionally limited to the scoped test-reset exception; custom policy
/// regexes are handled fail-closed by the CLI before a queue is planned.
pub fn evaluate_claim_safety_with_trusted_actors(
    input: &ClaimSafetyInput,
    trusted_actors: &[&str],
) -> ClaimSafetyDecision {
    let labels = input
        .labels
        .iter()
        .map(|label| label.as_str())
        .collect::<Vec<_>>();
    if labels.contains(&"security:quarantined") {
        return ClaimSafetyDecision::reject("security_quarantined");
    }
    if !labels.contains(&"safety:reviewed") {
        return ClaimSafetyDecision::reject("missing_safety_reviewed");
    }
    if input.body.matches(SAFETY_BEGIN_MARKER).count() != 1
        || input.body.matches(SAFETY_END_MARKER).count() != 1
    {
        return ClaimSafetyDecision::reject("invalid_safety_markers");
    }
    let Some(begin) = input.body.find(SAFETY_BEGIN_MARKER) else {
        return ClaimSafetyDecision::reject("invalid_safety_markers");
    };
    let Some(end) = input.body.find(SAFETY_END_MARKER) else {
        return ClaimSafetyDecision::reject("invalid_safety_markers");
    };
    if begin >= end {
        return ClaimSafetyDecision::reject("invalid_safety_markers");
    }

    let prefix = &input.body[..begin];
    let Some((heading_start, heading_end)) = last_safety_heading(prefix) else {
        return ClaimSafetyDecision::reject("missing_safety_review_heading");
    };
    if prefix[heading_end..]
        .lines()
        .any(|line| !line.trim().is_empty())
    {
        return ClaimSafetyDecision::reject("unexpected_safety_review_preamble");
    }
    let block_start = begin + SAFETY_BEGIN_MARKER.len();
    let block = &input.body[block_start..end];
    let lines = block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines
        .iter()
        .any(|line| !line.starts_with("- **decision:**"))
    {
        return ClaimSafetyDecision::reject("unexpected_safety_block_content");
    }
    if lines.len() != 1 {
        return ClaimSafetyDecision::reject("missing_safety_pass");
    }
    if lines[0] != "- **decision:** `SAFETY_PASS`" {
        return ClaimSafetyDecision::reject("non_pass_safety_decision");
    }

    let after_end = end + SAFETY_END_MARKER.len();
    let body_without_review = format!(
        "{}{}",
        &input.body[..heading_start],
        &input.body[after_end..]
    );
    let scan = format!(
        "{}\n{}",
        input.title,
        strip_guardian_skips(&body_without_review)
    );
    let intent = evaluate_issue_intent_with_trusted_actors(&scan, &input.author, trusted_actors);
    if intent.blocking {
        return ClaimSafetyDecision::reject("current_body_safety_block");
    }
    if intent.ambiguous {
        return ClaimSafetyDecision::reject("current_body_safety_ambiguous");
    }
    ClaimSafetyDecision::pass()
}

#[derive(Debug, Default)]
struct IssueIntent {
    blocking: bool,
    ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueIntentFinding {
    pub severity: &'static str,
    pub rule_id: &'static str,
    pub pattern: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueIntentLint {
    pub findings: Vec<IssueIntentFinding>,
    pub blocking: bool,
    pub ambiguous: bool,
    pub trusted: bool,
}

/// Evaluate a draft or persisted issue against the built-in intent policy.
/// The returned rule IDs are stable CLI data; callers must not infer safety
/// from prose or reimplement these predicates.
pub fn lint_issue_intent(title: &str, body: &str, actor: &str) -> IssueIntentLint {
    lint_issue_intent_with_trusted_actors(title, body, actor, &["berlinguyinca"])
}

/// Evaluate issue intent with the built-in policy and explicitly configured
/// trusted actors. Trust can only enable the narrow scoped-test-reset exception;
/// it never bypasses a blocking rule.
pub fn lint_issue_intent_with_trusted_actors(
    title: &str,
    body: &str,
    actor: &str,
    trusted_actors: &[&str],
) -> IssueIntentLint {
    let lower = format!("{title}\n{body}").to_ascii_lowercase();
    let mut findings = Vec::new();
    let mut add = |severity, rule_id, pattern| {
        findings.push(IssueIntentFinding {
            severity,
            rule_id,
            pattern,
        });
    };

    if contains_production_destruction(&lower) {
        add("block", "production-data-destruction", "delete production");
    }
    if contains_secret_exfiltration(&lower) {
        add("block", "secret-exfiltration", "secret disclosure");
    }
    if contains_credential_printing(&lower) {
        add("block", "credential-printing", "credential disclosure");
    }
    if contains_instruction_bypass(&lower) {
        add(
            "block",
            "instruction-bypass",
            "instruction or policy bypass",
        );
    }
    if contains_ci_or_review_bypass(&lower) {
        add("block", "ci-or-review-bypass", "CI or review bypass");
    }
    if contains_auth_backdoor(&lower) {
        add(
            "block",
            "auth-backdoor",
            "authentication backdoor or bypass",
        );
    }
    if lower.contains("rm -rf /")
        || (lower.contains("curl") && (lower.contains("| sh") || lower.contains("| bash")))
    {
        add("block", "destructive-shell", "destructive shell execution");
    }
    if contains_vague_data_cleanup(&lower) {
        add("ambiguous", "vague-data-cleanup", "ambiguous data cleanup");
    }
    if contains_weakened_security(&lower) {
        add(
            "ambiguous",
            "weaken-security-control",
            "weakened security control",
        );
    }
    if contains_any(
        &lower,
        &[
            "production",
            "prod",
            "billing",
            "payments",
            "migration",
            "terraform",
            "iam",
            "kms",
        ],
    ) {
        add(
            "ambiguous",
            "production-or-infra-touch",
            "production or infrastructure touch",
        );
    }

    let trusted_reset = is_trusted_test_reset(&lower, actor, trusted_actors);
    let blocking = findings.iter().any(|finding| finding.severity == "block");
    if trusted_reset && !blocking {
        findings.retain(|finding| {
            !matches!(
                finding.rule_id,
                "vague-data-cleanup" | "production-or-infra-touch"
            )
        });
        findings.push(IssueIntentFinding {
            severity: "info",
            rule_id: "trusted:test_data_reset",
            pattern: "configured trusted actor",
        });
    }
    IssueIntentLint {
        blocking: findings.iter().any(|finding| finding.severity == "block"),
        ambiguous: findings
            .iter()
            .any(|finding| finding.severity == "ambiguous"),
        trusted: trusted_reset
            && findings
                .iter()
                .any(|finding| finding.rule_id == "trusted:test_data_reset"),
        findings,
    }
}

/// The Rust authority uses bounded, line-local checks where the shell
/// implementation used bounded regular expressions: a CI noun followed by a
/// present-tense "skips" is descriptive prose, not a request to bypass CI.
fn evaluate_issue_intent_with_trusted_actors(
    text: &str,
    actor: &str,
    trusted_actors: &[&str],
) -> IssueIntent {
    let lint = lint_issue_intent_with_trusted_actors("", text, actor, trusted_actors);
    IssueIntent {
        blocking: lint.blocking,
        ambiguous: lint.ambiguous,
    }
}

fn last_safety_heading(prefix: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    let mut heading = None;
    for line in prefix.split_inclusive('\n') {
        let text = line.trim_end_matches(['\r', '\n']);
        if text.starts_with("## ") {
            heading = (text.trim() == "## Safety review").then_some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    if !prefix.ends_with('\n') {
        let last_start = prefix.rfind('\n').map_or(0, |index| index + 1);
        let last = &prefix[last_start..];
        if last.starts_with("## ") {
            heading = (last.trim() == "## Safety review").then_some((last_start, prefix.len()));
        }
    }
    heading
}

fn strip_guardian_skips(body: &str) -> String {
    body.lines()
        .filter(|line| !line.trim_start().starts_with("Guardian: skip-"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn contains_production_destruction(text: &str) -> bool {
    text.lines().any(|line| {
        let delete_production = ordered_contains(line, "delete", "production");
        let drop_production_database =
            ordered_contains(line, "drop", "prod") && ordered_contains(line, "prod", "database");
        delete_production || drop_production_database
    })
}

fn contains_secret_exfiltration(text: &str) -> bool {
    (contains_any(text, &["dump", "print", "exfiltrate", "send"]) && text.contains("secret"))
        || contains_any(text, &["aws token", "github token", "stripe token"])
}

fn contains_credential_printing(text: &str) -> bool {
    let subject = [
        "credential",
        "password",
        "api key",
        "api-key",
        "private key",
        "private-key",
        "token",
    ];
    let action = ["print", "dump", "log", "write", "show", "expose", "send"];
    (contains_any(text, &subject) && contains_any(text, &action))
        || (contains_any(text, &subject)
            && contains_any(
                text,
                &["to the log", "to log", "to console", "to stdout", "in logs"],
            ))
}

fn contains_instruction_bypass(text: &str) -> bool {
    [
        "ignore previous instructions",
        "ignore all previous instructions",
        "ignore system instructions",
        "ignore all system instructions",
        "ignore developer instructions",
        "ignore all developer instructions",
        "ignore agent instructions",
        "ignore all agent instructions",
        "bypass ci",
        "bypass tests",
        "bypass hooks",
        "bypass review",
        "bypass guardian",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

fn contains_ci_or_review_bypass(text: &str) -> bool {
    const NOUNS: &[&str] = &["ci", "check", "test", "hook", "review", "guardian"];
    const VERBS: &[&str] = &["disable", "turn off", "remove", "skip"];
    const NOUN_FIRST_VERBS: &[&str] = &[
        "disable",
        "disabled",
        "turn off",
        "turned off",
        "remove",
        "removed",
        "skip",
        "skipped",
        "bypass",
        "bypassed",
        "ignore",
        "ignored",
        "suppress",
        "suppressed",
    ];
    text.lines().any(|line| {
        // Corresponds to the verb→noun regex arm.
        VERBS.iter().any(|verb| {
            NOUNS
                .iter()
                .any(|noun| ordered_contains_word(line, verb, noun))
        })
            // Corresponds to the noun→verb arm. Do not include present-tense
            // "skips"/"disables"/"removes": that is the #1799 false-positive
            // shape the bounded legacy rule deliberately excluded.
            || NOUNS.iter().any(|noun| {
                NOUN_FIRST_VERBS
                    .iter()
                    .any(|verb| ordered_contains_word(line, noun, verb))
            })
    })
}

fn contains_auth_backdoor(text: &str) -> bool {
    let subject = ["auth", "login", "password", "token", "admin"];
    (contains_any(text, &subject) && contains_any(text, &["backdoor", "bypass"]))
        || contains_any(text, &["magic token", "magic password", "magic login"])
}

fn contains_vague_data_cleanup(text: &str) -> bool {
    [
        "clean data",
        "clean old data",
        "clean bad data",
        "clean stale data",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

fn contains_weakened_security(text: &str) -> bool {
    text.lines().any(|line| {
        ["relax", "disable", "remove"].iter().any(|verb| {
            ["security", "auth", "audit", "logging"]
                .iter()
                .any(|noun| ordered_contains(line, verb, noun))
        })
    })
}

fn is_trusted_test_reset(text: &str, actor: &str, trusted_actors: &[&str]) -> bool {
    if !trusted_actors.contains(&actor) {
        return false;
    }
    let reset = ["delete", "reset", "repopulate"].iter().any(|verb| {
        (ordered_contains(text, verb, "test") && ordered_contains(text, "test", "database"))
            || (ordered_contains(text, "test database", verb))
    });
    let production_out_of_scope = text
        .lines()
        .any(|line| ordered_contains(line, "production", "out of scope"));
    let scoped = contains_any(text, &["test", "local", "fixture", "dev"])
        && (production_out_of_scope || text.contains("production, staging"));
    reset && scoped
}

fn ordered_contains(text: &str, first: &str, second: &str) -> bool {
    text.find(first)
        .and_then(|index| text[index + first.len()..].find(second))
        .is_some()
}

fn ordered_contains_word(text: &str, first: &str, second: &str) -> bool {
    word_positions(text, first).into_iter().any(|first_index| {
        word_positions(&text[first_index + first.len()..], second)
            .into_iter()
            .next()
            .is_some()
    })
}

fn word_positions(text: &str, word: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut offset = 0;
    while let Some(found) = text[offset..].find(word) {
        let index = offset + found;
        let before = text[..index].chars().next_back();
        let after = text[index + word.len()..].chars().next();
        if before.is_none_or(|character| !character.is_ascii_alphanumeric() && character != '_')
            && after.is_none_or(|character| !character.is_ascii_alphanumeric() && character != '_')
        {
            positions.push(index);
        }
        offset = index + word.len();
    }
    positions
}

fn contains_any(text: &str, values: &[&str]) -> bool {
    values.iter().any(|value| text.contains(value))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteComment {
    pub id: u64,
    pub body: String,
    pub updated_at: String,
}

impl RemoteComment {
    pub fn new(id: u64, body: impl Into<String>, updated_at: impl Into<String>) -> Self {
        Self {
            id,
            body: body.into(),
            updated_at: updated_at.into(),
        }
    }
}

/// Parse the deliberately projected `gh api` comment payload.
///
/// Callers request only `id`, `body`, and `updated_at`, so accepting any other
/// key would make the remote-input boundary depend on an undocumented GitHub
/// shape. Null body/timestamp values preserve the shell protocol's empty-value
/// behavior and remain non-authoritative when the marked state is parsed.
pub fn parse_remote_comments_json(input: &str) -> Result<Vec<RemoteComment>, String> {
    JsonParser::new(input)
        .parse()?
        .into_array("GitHub issue comments")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("GitHub issue comments[{index}]");
            let mut object = value.into_object(&context)?;
            require_only_keys(&object, &["id", "body", "updated_at"])?;
            let id = take_required(&mut object, "id")?.into_number(&format!("{context} id"))?;
            let body = take_optional_string(&mut object, "body", &context)?.unwrap_or_default();
            let updated_at =
                take_optional_string(&mut object, "updated_at", &context)?.unwrap_or_default();
            Ok(RemoteComment::new(id, body, updated_at))
        })
        .collect()
}

/// Normalize the legacy `--paths` argument without depending on a shell JSON
/// parser. JSON arrays preserve paths verbatim; CSV input is trimmed and drops
/// empty entries, matching the former run-state helper.
pub fn parse_paths_argument(input: &str) -> Result<Vec<String>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Vec::new());
    }
    if input.starts_with('[') {
        return JsonParser::new(input)
            .parse()?
            .into_array("run-state paths")?
            .into_iter()
            .enumerate()
            .map(|(index, value)| value.into_string(&format!("run-state paths[{index}]")))
            .collect();
    }
    Ok(input
        .split(',')
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPullRequest {
    pub number: u64,
    pub body: String,
}

pub fn parse_open_pull_requests_json(input: &str) -> Result<Vec<OpenPullRequest>, String> {
    JsonParser::new(input)
        .parse()?
        .into_array("GitHub open pull requests")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("GitHub open pull requests[{index}]");
            let mut object = value.into_object(&context)?;
            require_only_keys(&object, &["number", "body"])?;
            Ok(OpenPullRequest {
                number: take_required(&mut object, "number")?
                    .into_number(&format!("{context} number"))?,
                body: take_optional_string(&mut object, "body", &context)?.unwrap_or_default(),
            })
        })
        .collect()
}

pub fn find_reconcilable_pull_request(
    pull_requests: &[OpenPullRequest],
    issue: u64,
) -> Option<&OpenPullRequest> {
    pull_requests
        .iter()
        .filter(|pull_request| {
            closes_issue(&pull_request.body, issue) && closeout_count(&pull_request.body) == 1
        })
        .min_by_key(|pull_request| pull_request.number)
}

fn closes_issue(body: &str, issue: u64) -> bool {
    let body = body.to_ascii_lowercase();
    let issue = format!("#{issue}");
    [
        "close", "closed", "closes", "fix", "fixed", "fixes", "resolve", "resolved", "resolves",
    ]
    .iter()
    .any(|verb| contains_closing_reference(&body, verb, &issue))
}

fn contains_closing_reference(body: &str, verb: &str, issue: &str) -> bool {
    let mut start = 0;
    while let Some(found) = body[start..].find(verb) {
        let index = start + found;
        let before = body[..index].chars().next_back();
        let after_verb = index + verb.len();
        if before.is_none_or(|character| !character.is_ascii_alphanumeric()) {
            let suffix = &body[after_verb..];
            let whitespace = suffix.len() - suffix.trim_start().len();
            let reference = &suffix[whitespace..];
            if let Some(after_issue) = reference.strip_prefix(issue) {
                if after_issue
                    .chars()
                    .next()
                    .is_none_or(|character| !character.is_ascii_digit())
                {
                    return true;
                }
            }
        }
        start = after_verb;
    }
    false
}

fn closeout_count(body: &str) -> usize {
    body.lines()
        .filter(|line| line.trim().eq_ignore_ascii_case("## Closeout report"))
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStateRecord {
    pub repo: String,
    pub issue: u64,
    pub worker_id: String,
    pub state: String,
    pub branch: String,
    pub pr: String,
    pub step: String,
    pub paths: Vec<String>,
    pub claimed_at: String,
    pub updated_at: String,
    pub ttl_seconds: u64,
}

impl RunStateRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: impl Into<String>,
        issue: u64,
        worker_id: impl Into<String>,
        state: impl Into<String>,
        branch: impl Into<String>,
        pr: impl Into<String>,
        step: impl Into<String>,
        paths: Vec<String>,
        claimed_at: impl Into<String>,
        updated_at: impl Into<String>,
        ttl_seconds: u64,
    ) -> Self {
        Self {
            repo: repo.into(),
            issue,
            worker_id: worker_id.into(),
            state: state.into(),
            branch: branch.into(),
            pr: pr.into(),
            step: step.into(),
            paths,
            claimed_at: claimed_at.into(),
            updated_at: updated_at.into(),
            ttl_seconds,
        }
    }

    pub fn parse_json(input: &str) -> Result<Self, String> {
        let value = JsonParser::new(input).parse()?;
        let mut object = value.into_object("run-state record")?;
        require_only_keys(
            &object,
            &[
                "schema",
                "repo",
                "issue",
                "worker_id",
                "state",
                "branch",
                "pr",
                "step",
                "paths",
                "claimed_at",
                "updated_at",
                "ttl_seconds",
            ],
        )?;
        let schema = take_required(&mut object, "schema")?.into_number("run-state schema")?;
        if schema != 1 {
            return Err(format!("unsupported run-state schema: {schema}"));
        }
        let paths = match object.remove("paths") {
            None | Some(JsonValue::Null) => Vec::new(),
            Some(value) => value
                .into_array("run-state paths")?
                .into_iter()
                .enumerate()
                .map(|(index, value)| value.into_string(&format!("run-state paths[{index}]")))
                .collect::<Result<Vec<_>, _>>()?,
        };
        let state = take_required(&mut object, "state")?.into_string("run-state state")?;
        let claimed_at =
            take_required(&mut object, "claimed_at")?.into_string("run-state claimed_at")?;
        let record = Self {
            repo: take_required(&mut object, "repo")?.into_string("run-state repo")?,
            issue: take_required(&mut object, "issue")?.into_number("run-state issue")?,
            worker_id: take_required(&mut object, "worker_id")?
                .into_string("run-state worker_id")?,
            step: take_optional_string(&mut object, "step", "run-state record")?
                .unwrap_or_else(|| state.clone()),
            branch: take_optional_string(&mut object, "branch", "run-state record")?
                .unwrap_or_default(),
            pr: take_optional_string(&mut object, "pr", "run-state record")?.unwrap_or_default(),
            state,
            paths,
            updated_at: take_optional_string(&mut object, "updated_at", "run-state record")?
                .unwrap_or_else(|| claimed_at.clone()),
            claimed_at,
            ttl_seconds: match object.remove("ttl_seconds") {
                None | Some(JsonValue::Null) => 10_800,
                Some(value) => value.into_number("run-state ttl_seconds")?,
            },
        };
        record.validate()?;
        Ok(record)
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema\":1,\"repo\":\"{}\",\"issue\":{},\"worker_id\":\"{}\",\"state\":\"{}\",\"branch\":\"{}\",\"pr\":\"{}\",\"step\":\"{}\",\"paths\":[{}],\"claimed_at\":\"{}\",\"updated_at\":\"{}\",\"ttl_seconds\":{}}}",
            escape_json(&self.repo),
            self.issue,
            escape_json(&self.worker_id),
            escape_json(&self.state),
            escape_json(&self.branch),
            escape_json(&self.pr),
            escape_json(&self.step),
            self.paths
                .iter()
                .map(|path| format!("\"{}\"", escape_json(path)))
                .collect::<Vec<_>>()
                .join(","),
            escape_json(&self.claimed_at),
            escape_json(&self.updated_at),
            self.ttl_seconds,
        )
    }

    pub fn to_marked_comment(&self) -> String {
        format!(
            "{RUN_STATE_BEGIN_MARKER}\n{}\n{RUN_STATE_END_MARKER}",
            self.to_json()
        )
    }

    fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("repo", &self.repo),
            ("worker_id", &self.worker_id),
            ("state", &self.state),
            ("claimed_at", &self.claimed_at),
            ("updated_at", &self.updated_at),
        ] {
            if value.trim().is_empty() {
                return Err(format!("run-state {name} must not be empty"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedRunState {
    pub comment_id: u64,
    pub server_updated_at: String,
    pub record: RunStateRecord,
}

/// Select the deterministic CAS owner from untrusted GitHub comments.
///
/// The lowest numeric marked-comment ID is authoritative even when it is
/// malformed or bound to another issue. In those cases this returns `None`
/// rather than selecting a higher comment and silently stealing a lease.
pub fn select_run_state(
    comments: &[RemoteComment],
    repo: &str,
    issue: u64,
) -> Option<SelectedRunState> {
    let comment = lowest_marked_comment(comments)?;
    let record = parse_marked_record(&comment.body).ok()?;
    if record.repo != repo || record.issue != issue {
        return None;
    }
    Some(SelectedRunState {
        comment_id: comment.id,
        server_updated_at: comment.updated_at.clone(),
        record,
    })
}

/// Return the sole CAS linearization point even if its embedded record is
/// malformed. Upsert must patch that comment rather than create a higher-ID
/// competitor, while reads fail closed through `select_run_state`.
pub fn lowest_marked_comment(comments: &[RemoteComment]) -> Option<&RemoteComment> {
    comments
        .iter()
        .filter(|comment| {
            comment.body.contains(RUN_STATE_BEGIN_MARKER)
                && comment.body.contains(RUN_STATE_END_MARKER)
        })
        .min_by_key(|comment| comment.id)
}

fn parse_marked_record(body: &str) -> Result<RunStateRecord, String> {
    let (_, after_begin) = body
        .split_once(RUN_STATE_BEGIN_MARKER)
        .ok_or_else(|| "missing run-state begin marker".to_string())?;
    let (record, _) = after_begin
        .split_once(RUN_STATE_END_MARKER)
        .ok_or_else(|| "missing run-state end marker".to_string())?;
    RunStateRecord::parse_json(record.trim())
}

pub fn parse_run_state_comment(body: &str) -> Result<RunStateRecord, String> {
    parse_marked_record(body)
}

/// Return true only for a syntactically valid terminal record that explicitly
/// records a merged state. This avoids letting whitespace, prose, or a forged
/// JSON fragment bypass the terminal-claim protection.
pub fn terminal_merged_comment_exists(comments: &[RemoteComment]) -> bool {
    comments.iter().any(|comment| {
        let Some((_, after_begin)) = comment.body.split_once(RUN_TERMINAL_BEGIN_MARKER) else {
            return false;
        };
        let Some((payload, _)) = after_begin.split_once(RUN_TERMINAL_END_MARKER) else {
            return false;
        };
        let Ok(mut object) = JsonParser::new(payload.trim())
            .parse()
            .and_then(|value| value.into_object("run terminal record"))
        else {
            return false;
        };
        matches!(object.remove("state"), Some(JsonValue::String(state)) if state == "merged")
    })
}

fn take_required(object: &mut BTreeMap<String, JsonValue>, key: &str) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("run-state record missing required key: {key}"))
}

fn take_optional_string(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Option<String>, String> {
    match object.remove(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => Ok(Some(value)),
        Some(_) => Err(format!("{context} {key} must be a JSON string or null")),
    }
}

fn require_only_keys(object: &BTreeMap<String, JsonValue>, allowed: &[&str]) -> Result<(), String> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown run-state record key: {key}"));
        }
    }
    Ok(())
}

fn escape_json(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            character if character.is_control() => format!("\\u{:04x}", character as u32)
                .chars()
                .collect::<Vec<_>>(),
            character => vec![character],
        })
        .collect()
}
