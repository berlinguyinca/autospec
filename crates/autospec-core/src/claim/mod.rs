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
pub mod intent_scope;

use std::{collections::BTreeMap, fmt, ops::Range};

use crate::state::json::{JsonParser, JsonValue};

pub const RUN_STATE_BEGIN_MARKER: &str = "<!-- autospec-run-state:begin -->";
pub const RUN_STATE_END_MARKER: &str = "<!-- autospec-run-state:end -->";
pub const RUN_TERMINAL_BEGIN_MARKER: &str = "<!-- autospec-run-terminal:begin -->";
pub const RUN_TERMINAL_END_MARKER: &str = "<!-- autospec-run-terminal:end -->";
pub const EXECUTOR_RESULT_BEGIN_MARKER: &str = "<!-- autospec-executor-result:begin -->";
pub const EXECUTOR_RESULT_END_MARKER: &str = "<!-- autospec-executor-result:end -->";
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
/// title/body after validating either an exact marker block or its review label.
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
    if labels.contains(&"autospec:needs-human") {
        return ClaimSafetyDecision::reject("autospec_needs_human");
    }
    if !labels.contains(&"safety:reviewed") {
        return ClaimSafetyDecision::reject("missing_safety_reviewed");
    }
    let (body_without_review, safety_actor) =
        match reviewed_body_without_safety_section(&input.body) {
            Ok(review) => review,
            Err(reason) => return ClaimSafetyDecision::reject(reason),
        };
    let scan = format!(
        "{}\n{}",
        input.title,
        strip_guardian_skips(&body_without_review)
    );
    let intent = evaluate_issue_intent_with_trusted_actors(&scan, &input.author, trusted_actors);
    if intent.blocking {
        return ClaimSafetyDecision::reject("current_body_safety_block");
    }
    if intent.ambiguous
        && !safety_actor
            .as_deref()
            .is_some_and(|actor| trusted_actors.contains(&actor))
    {
        return ClaimSafetyDecision::reject("current_body_safety_ambiguous");
    }
    ClaimSafetyDecision::pass()
}

fn reviewed_body_without_safety_section(
    body: &str,
) -> Result<(String, Option<String>), &'static str> {
    let begin_count = body.matches(SAFETY_BEGIN_MARKER).count();
    let end_count = body.matches(SAFETY_END_MARKER).count();
    if begin_count == 0 && end_count == 0 {
        return if last_safety_heading(body).is_none() {
            Ok((body.to_string(), None))
        } else {
            Err("invalid_safety_markers")
        };
    }
    if begin_count != 1 || end_count != 1 {
        return Err("invalid_safety_markers");
    }
    let begin = body
        .find(SAFETY_BEGIN_MARKER)
        .ok_or("invalid_safety_markers")?;
    let end = body
        .find(SAFETY_END_MARKER)
        .ok_or("invalid_safety_markers")?;
    if begin >= end {
        return Err("invalid_safety_markers");
    }

    let prefix = &body[..begin];
    let (heading_start, heading_end) =
        last_safety_heading(prefix).ok_or("missing_safety_review_heading")?;
    if prefix[heading_end..]
        .lines()
        .any(|line| !line.trim().is_empty())
    {
        return Err("unexpected_safety_review_preamble");
    }
    let block_start = begin + SAFETY_BEGIN_MARKER.len();
    let lines = body[block_start..end]
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.iter().any(|line| {
        !line.starts_with("- **decision:**")
            && !line.starts_with("- **actor:**")
            && !line.starts_with("- **reviewer:**")
            && !line.starts_with("- **semantic-reviewer:**")
    }) {
        return Err("unexpected_safety_block_content");
    }
    let decisions = lines
        .iter()
        .filter(|line| line.starts_with("- **decision:**"))
        .collect::<Vec<_>>();
    if decisions.len() != 1 {
        return Err("missing_safety_pass");
    }
    if decisions[0] != &"- **decision:** `SAFETY_PASS`" {
        return Err("non_pass_safety_decision");
    }
    let actors = lines
        .iter()
        .filter_map(|line| {
            [
                "- **actor:** `",
                "- **reviewer:** `",
                "- **semantic-reviewer:** `",
            ]
            .iter()
            .find_map(|prefix| {
                line.strip_prefix(prefix)
                    .and_then(|value| value.strip_suffix('`'))
            })
        })
        .collect::<Vec<_>>();
    if lines.len() != 1 + actors.len() || actors.len() > 1 {
        return Err("unexpected_safety_block_content");
    }

    let after_end = end + SAFETY_END_MARKER.len();
    Ok((
        format!("{}{}", &body[..heading_start], &body[after_end..]),
        actors.first().map(|actor| (*actor).to_string()),
    ))
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
    let lower = format!("{title}\n{}", strip_guardian_skips(body)).to_ascii_lowercase();
    let infra_lower = strip_out_of_scope_sections(&lower);
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
    if intent_scope::mentions_production_or_infra_touch(&infra_lower) {
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

fn contains_any_word(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| {
        text.match_indices(word).any(|(offset, _)| {
            let before = text[..offset].chars().next_back();
            let after = text[offset + word.len()..].chars().next();
            before.is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
                && after.is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
        })
    })
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
        .map(|line| guardian_skip_reason(line).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_out_of_scope_sections(body: &str) -> String {
    let mut result = Vec::new();
    let mut excluded = false;
    for line in body.lines() {
        let heading = line.trim_start().starts_with("## ");
        if heading {
            excluded = line.trim().eq_ignore_ascii_case("## out of scope");
        }
        if !excluded && !line.trim_start().starts_with("out of scope:") {
            result.push(line);
        }
    }
    result.join("\n")
}

fn guardian_skip_reason(line: &str) -> Option<&str> {
    const RULE_IDS: &[&str] = &[
        "OUT_OF_SCOPE",
        "MISSING_TEST",
        "COMPLEXITY",
        "SECURITY",
        "TODO_LEFT",
        "MOCK_DB",
        "HALLUCINATED_API",
        "DUPLICATE_CODE",
        "STRING_MATCH_DOMAIN_LOGIC",
        "REPEATED_STRUCTURE_AS_CODE",
        "DOC_OUT_OF_SYNC",
        "INVENTED_CONFIG",
        "VACUOUS_GREP_INVERSE_OR_TRUE",
        "VACUOUS_OR_TRUE",
        "VACUOUS_TAUTOLOGY",
        "VACUOUS_AC_STUB",
        "VACUOUS_EMPTY_TEST",
        "VACUOUS_NO_ASSERT",
        "ASSERTION_DENSITY",
        "REINVENT_REPO_UTIL",
        "NEW_DEP_UNJUSTIFIED",
        "NEW_ABSTRACTION_SINGLE_CALLER",
    ];

    let tail = line.strip_prefix("Guardian:")?;
    if !tail.starts_with(char::is_whitespace) {
        return None;
    }
    let (rules, reason) = tail.trim_start().split_once('#')?;
    if !reason.starts_with(char::is_whitespace) || !rules.ends_with(char::is_whitespace) {
        return None;
    }
    let reason = reason.trim_start_matches(char::is_whitespace);
    if reason.chars().count() < 2 {
        return None;
    }
    let rules = rules
        .trim_end()
        .split(',')
        .enumerate()
        .map(|(index, rule)| {
            let rule = if index == 0 { rule } else { rule.trim_start() };
            if rule.chars().any(char::is_whitespace) {
                return None;
            }
            rule.strip_prefix("skip-")
                .filter(|rule| RULE_IDS.contains(rule))
        });
    rules.collect::<Option<Vec<_>>>()?;
    Some(reason)
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

/// Disclosure verbs, with the inflections the previous substring test already
/// caught (every entry embeds its base verb). Forms that do NOT embed their
/// base — "written", "wrote", "sent", "exposing" — never matched before and
/// stay out: adding them would widen a rule this change narrows.
const CREDENTIAL_DISCLOSURE_VERBS: &[&str] = &[
    "print", "prints", "printed", "printing", "dump", "dumps", "dumped", "dumping", "log", "logs",
    "logged", "logging", "write", "writes", "show", "shows", "showed", "showing", "shown",
    "expose", "exposes", "exposed", "send", "sends", "sending",
];

/// Credential nouns, including the separator-free and plural spellings the CLI
/// policy regex declares via `api[ -]?keys?`.
const CREDENTIAL_DISCLOSURE_NOUNS: &[&str] = &[
    "credential",
    "credentials",
    "password",
    "passwords",
    "api key",
    "api keys",
    "api-key",
    "api-keys",
    "apikey",
    "apikeys",
    "private key",
    "private keys",
    "private-key",
    "private-keys",
    "privatekey",
    "privatekeys",
    "token",
    "tokens",
];

/// Disclosure destinations for the noun-first arm, covering the optional
/// article and plural the CLI regex spells `to (the )?(logs?|console|stdout)`.
const CREDENTIAL_DISCLOSURE_DESTINATIONS: &[&str] = &[
    "to the log",
    "to the logs",
    "to log",
    "to logs",
    "to the console",
    "to console",
    "to the stdout",
    "to stdout",
    "in log",
    "in logs",
];

/// Credential disclosure is line-scoped, word-boundary aware, and ordered —
/// the same shape as `contains_production_destruction` and the CLI policy
/// regexes in `commands/lint.rs`, where `.` never crosses a newline and the
/// verb precedes the noun.
///
/// The previous implementation tested two unordered whole-document substring
/// memberships, so a lexical scanner's own vocabulary tripped it: `token`
/// inside "tokens", `print` inside "fingerprint", `write` inside "writer",
/// `log` inside "logic" (issues #3111, #3173, #3349). Requiring a real word on
/// the same line, in order, removes those without weakening the posture: a
/// genuine request ("print the API key to stdout", "write the token to the
/// log") still blocks.
fn contains_credential_printing(text: &str) -> bool {
    text.lines().any(|line| {
        CREDENTIAL_DISCLOSURE_VERBS.iter().any(|verb| {
            CREDENTIAL_DISCLOSURE_NOUNS
                .iter()
                .any(|noun| ordered_pair_start(line, verb, noun).is_some())
        }) || CREDENTIAL_DISCLOSURE_NOUNS.iter().any(|noun| {
            CREDENTIAL_DISCLOSURE_DESTINATIONS
                .iter()
                .any(|destination| ordered_pair_start(line, noun, destination).is_some())
        })
    })
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
        // Collect every bypass-phrase start position on the line, across both
        // arms.
        // (start, end) — `end` is where the SECOND word of the pair begins, which
        // failure_condition_governs needs: the governing phrase can sit between
        // the two words ("test below, and confirm each one fails when … removed")
        // rather than before them.
        let mut matches: Vec<(usize, usize)> = Vec::new();
        // verb→noun arm.
        for verb in VERBS {
            for noun in NOUNS {
                if let Some(pair) = ordered_pair_span(line, verb, noun) {
                    matches.push(pair);
                }
            }
        }
        // noun→verb arm. Present-tense "skips"/"disables"/"removes" stay
        // excluded (the #1799 shape) via NOUN_FIRST_VERBS.
        for noun in NOUNS {
            for verb in NOUN_FIRST_VERBS {
                if let Some(pair) = ordered_pair_span(line, noun, verb) {
                    matches.push(pair);
                }
            }
        }
        // A bypass phrase governed by a nearby prohibition cue ("do not skip
        // the tests", "no existing test is #[ignore]d") is a guardrail, not a
        // bypass request — suppress it (issue #2175). The line still fires if
        // ANY bypass phrase is ungoverned, so a real request ("disable CI",
        // "skip the test suite", or "do not touch X; skip CI") still trips.
        matches.iter().any(|&(start, end)| {
            !prohibition_precedes(line, start) && !failure_condition_governs(line, end)
        })
    })
}

/// Byte offset of `first` when a `second` word occurs later on the same line.
/// Retained for the callers that need only the start; the CI/review-bypass arm
/// uses `ordered_pair_span` because it also needs where `second` begins.
fn ordered_pair_start(text: &str, first: &str, second: &str) -> Option<usize> {
    ordered_pair_span(text, first, second).map(|(start, _)| start)
}

/// Byte offsets of `first` and of the `second` word occurring later on the same
/// line.
fn ordered_pair_span(text: &str, first: &str, second: &str) -> Option<(usize, usize)> {
    word_positions(text, first).into_iter().find_map(|index| {
        let tail = index + first.len();
        word_positions(&text[tail..], second)
            .first()
            .map(|offset| (index, tail + offset))
    })
}

/// True when the bypass phrase is the CONDITION of a failure requirement rather
/// than an instruction — "confirm each one fails when the control it covers is
/// removed", "verify each assertion fails if the hook is disabled".
///
/// This is the anti-vacuous proof rule AGENTS.md requires: a test must fail when
/// the control it covers is taken away, or it is not evidence. Read as a bypass
/// it inverts the rule's meaning — and it quarantined the root bootstrap task of
/// a 123-issue programme along with five others (InferWeave/inferweave #1, #2,
/// #5, #10, #50, #123), leaving the whole queue transitively blocked.
///
/// `prohibition_precedes` cannot cover this: it recognises prohibitions ("do not
/// skip"), and this is a requirement. The signal is a failure verb followed by a
/// conditional connector, both before the bypass word and in the same clause.
/// A real request keeps blocking because it states no failure at all ("disable
/// the CI checks before merging"), and so does a bypass merely adjacent to one
/// ("the build fails, so disable CI" — no connector between them).
fn failure_condition_governs(line: &str, match_end: usize) -> bool {
    const CONNECTORS: &[&str] = &["when ", "if ", "unless ", "without "];
    let prefix = &line[..match_end];
    let clause_start = [
        prefix.rfind(';').map(|index| index + 1),
        prefix.rfind(". ").map(|index| index + 2),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);
    let clause = &prefix[clause_start..];
    match clause.rfind("fail") {
        Some(index) => CONNECTORS
            .iter()
            .any(|connector| clause[index..].contains(connector)),
        None => false,
    }
}

/// True when a prohibition cue sits in the SAME CLAUSE before a bypass phrase
/// at `match_index`, marking it as a guardrail rather than an instruction to
/// bypass CI/review (issue #2175). Input is already lowercased by
/// `lint_issue_intent`.
///
/// The clause is bounded by strong separators (`;` or `. `) — NOT commas, so a
/// list guardrail like "…test weakened, loosened, `#[ignore]`d" keeps its
/// leading "no". A real bypass in a later clause ("do not touch X; skip CI")
/// is not suppressed because the cue is in the previous clause.
fn prohibition_precedes(line: &str, match_index: usize) -> bool {
    const CUES: &[&str] = &[
        "no ",
        "not ",
        "n't",
        "never",
        "without",
        "cannot",
        "can not",
        "do not",
        "does not",
        "must not",
        "should not",
        "may not",
        "will not",
        "avoid ",
        "prohibit",
        "forbid",
        "prevent",
        "refuse",
        "disallow",
        "keep every",
        "keep all",
        "leave every",
        "leave all",
    ];
    let prefix = &line[..match_index];
    let clause_start = [
        prefix.rfind(';').map(|index| index + 1),
        prefix.rfind(". ").map(|index| index + 2),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);
    let clause = &prefix[clause_start..];
    CUES.iter().any(|cue| clause.contains(cue))
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
    pub head_ref_name: String,
    pub head_ref_oid: String,
    pub is_draft: bool,
    pub base_ref_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredCheck {
    pub name: String,
    pub state: String,
}

impl RequiredCheck {
    pub fn new(name: impl Into<String>, state: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: state.into(),
        }
    }
}

pub fn parse_required_checks_json(input: &str) -> Result<Vec<RequiredCheck>, String> {
    JsonParser::new(input)
        .parse()?
        .into_array("GitHub required checks")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("GitHub required checks[{index}]");
            let mut object = value.into_object(&context)?;
            require_only_keys(&object, &["name", "state"])?;
            Ok(RequiredCheck::new(
                take_required(&mut object, "name")?.into_string(&format!("{context} name"))?,
                take_required(&mut object, "state")?.into_string(&format!("{context} state"))?,
            ))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimRecoveryBlock {
    LiveLease,
    IdentityMismatch,
    PullRequestNotMergeReady,
    MissingRequiredChecks,
    RequiredCheckPending,
    RequiredCheckFailed,
}

impl ClaimRecoveryBlock {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LiveLease => "live_lease",
            Self::IdentityMismatch => "identity_mismatch",
            Self::PullRequestNotMergeReady => "pull_request_not_merge_ready",
            Self::MissingRequiredChecks => "missing_required_checks",
            Self::RequiredCheckPending => "required_check_pending",
            Self::RequiredCheckFailed => "required_check_failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimRecoveryDecision {
    Recover { pull_request: u64 },
    Blocked(ClaimRecoveryBlock),
}

pub fn evaluate_merge_ready_claim_recovery(
    claim: &RunStateRecord,
    evidence: &ExecutorResultEvidence,
    pull_request: &OpenPullRequest,
    required_checks: &[RequiredCheck],
    lease_is_live: bool,
) -> ClaimRecoveryDecision {
    if lease_is_live {
        return ClaimRecoveryDecision::Blocked(ClaimRecoveryBlock::LiveLease);
    }

    let exact_identity = claim.state == "claimed"
        && claim.repo == evidence.repo
        && claim.issue == evidence.issue
        && claim.worker_id == evidence.worker_id
        && claim.branch == evidence.branch
        && claim.claim_id.as_deref().is_some_and(|claim_id| {
            !claim_id.is_empty() && evidence.claim_id.as_deref() == Some(claim_id)
        })
        && evidence.outcome == "succeeded"
        && evidence.pr == Some(pull_request.number)
        && (claim.pr.is_empty() || claim.pr == pull_request.number.to_string())
        && pull_request.head_ref_name == claim.branch
        && evidence.commit.as_deref() == Some(pull_request.head_ref_oid.as_str());
    if !exact_identity {
        return ClaimRecoveryDecision::Blocked(ClaimRecoveryBlock::IdentityMismatch);
    }
    if !is_reconcilable_pull_request(pull_request, claim.issue) {
        return ClaimRecoveryDecision::Blocked(ClaimRecoveryBlock::PullRequestNotMergeReady);
    }
    if required_checks.is_empty() {
        return ClaimRecoveryDecision::Blocked(ClaimRecoveryBlock::MissingRequiredChecks);
    }
    if required_checks
        .iter()
        .any(|check| check.state.eq_ignore_ascii_case("PENDING"))
    {
        return ClaimRecoveryDecision::Blocked(ClaimRecoveryBlock::RequiredCheckPending);
    }
    if required_checks
        .iter()
        .any(|check| !check.state.eq_ignore_ascii_case("SUCCESS"))
    {
        return ClaimRecoveryDecision::Blocked(ClaimRecoveryBlock::RequiredCheckFailed);
    }
    ClaimRecoveryDecision::Recover {
        pull_request: pull_request.number,
    }
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
            require_only_keys(
                &object,
                &[
                    "number",
                    "body",
                    "headRefName",
                    "headRefOid",
                    "isDraft",
                    "baseRefName",
                ],
            )?;
            Ok(OpenPullRequest {
                number: take_required(&mut object, "number")?
                    .into_number(&format!("{context} number"))?,
                body: take_optional_string(&mut object, "body", &context)?.unwrap_or_default(),
                head_ref_name: take_optional_string(&mut object, "headRefName", &context)?
                    .unwrap_or_default(),
                head_ref_oid: take_required(&mut object, "headRefOid")?
                    .into_string(&format!("{context} headRefOid"))?,
                is_draft: take_required(&mut object, "isDraft")?
                    .into_bool(&format!("{context} isDraft"))?,
                base_ref_name: take_required(&mut object, "baseRefName")?
                    .into_string(&format!("{context} baseRefName"))?,
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
        .filter(|pull_request| is_reconcilable_pull_request(pull_request, issue))
        .min_by_key(|pull_request| pull_request.number)
}

pub fn is_reconcilable_pull_request(pull_request: &OpenPullRequest, issue: u64) -> bool {
    !pull_request.is_draft
        && closes_issue(&pull_request.body, issue)
        && closeout_count(&pull_request.body) == 1
}

pub fn is_executor_result_pull_request(
    pull_request: &OpenPullRequest,
    issue: u64,
    branch: &str,
) -> bool {
    is_reconcilable_pull_request(pull_request, issue) && pull_request.head_ref_name == branch
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorResultEvidence {
    pub repo: String,
    pub issue: u64,
    pub worker_id: String,
    pub branch: String,
    pub outcome: String,
    pub pr: Option<u64>,
    pub step: String,
    pub receipt_id: String,
    pub claim_id: Option<String>,
    pub commit: Option<String>,
    pub premerge_receipt: Option<String>,
}

impl ExecutorResultEvidence {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: impl Into<String>,
        issue: u64,
        worker_id: impl Into<String>,
        branch: impl Into<String>,
        outcome: impl Into<String>,
        pr: Option<u64>,
        step: impl Into<String>,
        receipt_id: impl Into<String>,
        claim_id: Option<String>,
        commit: Option<String>,
        premerge_receipt: Option<String>,
    ) -> Self {
        Self {
            repo: repo.into(),
            issue,
            worker_id: worker_id.into(),
            branch: branch.into(),
            outcome: outcome.into(),
            pr,
            step: step.into(),
            receipt_id: receipt_id.into(),
            claim_id,
            commit,
            premerge_receipt,
        }
    }

    pub fn to_marked_comment(&self) -> String {
        format!(
            "{EXECUTOR_RESULT_BEGIN_MARKER}\n{}\n{EXECUTOR_RESULT_END_MARKER}",
            self.to_json()
        )
    }

    fn to_json(&self) -> String {
        let pr = self
            .pr
            .map_or_else(|| "null".to_string(), |pr| pr.to_string());
        let claim_id = optional_json_string(self.claim_id.as_deref());
        let commit = optional_json_string(self.commit.as_deref());
        let premerge_receipt = optional_json_string(self.premerge_receipt.as_deref());
        format!(
            "{{\"schema\":1,\"repo\":\"{}\",\"issue\":{},\"worker_id\":\"{}\",\"branch\":\"{}\",\"outcome\":\"{}\",\"pr\":{},\"step\":\"{}\",\"receipt_id\":\"{}\",\"claim_id\":{claim_id},\"commit\":{commit},\"premerge_receipt\":{premerge_receipt}}}",
            escape_json(&self.repo),
            self.issue,
            escape_json(&self.worker_id),
            escape_json(&self.branch),
            escape_json(&self.outcome),
            pr,
            escape_json(&self.step),
            escape_json(&self.receipt_id),
        )
    }

    fn parse_json(input: &str) -> Result<Self, String> {
        let mut object = JsonParser::new(input)
            .parse()?
            .into_object("executor result evidence")?;
        require_only_keys(
            &object,
            &[
                "schema",
                "repo",
                "issue",
                "worker_id",
                "branch",
                "outcome",
                "pr",
                "step",
                "receipt_id",
                "claim_id",
                "commit",
                "premerge_receipt",
            ],
        )?;
        let schema = take_required(&mut object, "schema")?.into_number("executor result schema")?;
        if schema != 1 {
            return Err(format!("unsupported executor result schema: {schema}"));
        }
        let pr = match object.remove("pr") {
            Some(JsonValue::Null) => None,
            Some(value) => Some(value.into_number("executor result pr")?),
            None => return Err("executor result evidence missing required key: pr".to_string()),
        };
        let evidence = Self {
            repo: take_required(&mut object, "repo")?.into_string("executor result repo")?,
            issue: take_required(&mut object, "issue")?.into_number("executor result issue")?,
            worker_id: take_required(&mut object, "worker_id")?
                .into_string("executor result worker_id")?,
            branch: take_required(&mut object, "branch")?.into_string("executor result branch")?,
            outcome: take_required(&mut object, "outcome")?
                .into_string("executor result outcome")?,
            pr,
            step: take_required(&mut object, "step")?.into_string("executor result step")?,
            receipt_id: take_required(&mut object, "receipt_id")?
                .into_string("executor result receipt_id")?,
            claim_id: take_optional_string(&mut object, "claim_id", "executor result")?,
            commit: take_optional_string(&mut object, "commit", "executor result")?,
            premerge_receipt: take_optional_string(
                &mut object,
                "premerge_receipt",
                "executor result",
            )?,
        };
        evidence.validate()?;
        Ok(evidence)
    }

    fn validate(&self) -> Result<(), String> {
        if self.issue == 0 {
            return Err("executor result issue must be positive".to_string());
        }
        for (name, value) in [
            ("repo", &self.repo),
            ("worker_id", &self.worker_id),
            ("branch", &self.branch),
            ("outcome", &self.outcome),
            ("step", &self.step),
            ("receipt_id", &self.receipt_id),
        ] {
            if value.trim().is_empty() {
                return Err(format!("executor result {name} must not be empty"));
            }
        }
        let success_binding = [
            self.claim_id.as_deref(),
            self.commit.as_deref(),
            self.premerge_receipt.as_deref(),
        ];
        if self.claim_id.is_none() {
            return Err("executor result requires claim_id".to_string());
        }
        if self.outcome == "succeeded" && success_binding.iter().any(Option::is_none) {
            return Err(
                "succeeded executor result requires claim_id, commit, and premerge_receipt"
                    .to_string(),
            );
        }
        if self.outcome != "succeeded"
            && [self.commit.as_deref(), self.premerge_receipt.as_deref()]
                .iter()
                .any(Option::is_some)
        {
            return Err(
                "non-succeeded executor result rejects commit and premerge_receipt".to_string(),
            );
        }
        for (name, value) in [
            ("claim_id", self.claim_id.as_deref()),
            ("commit", self.commit.as_deref()),
        ] {
            if value.is_some_and(|value| value.trim().is_empty()) {
                return Err(format!("executor result {name} must not be empty"));
            }
        }
        if self.premerge_receipt.as_deref().is_some_and(|digest| {
            digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) {
            return Err(
                "executor result premerge_receipt must be 64-character lowercase hex".to_string(),
            );
        }
        Ok(())
    }
}

pub fn executor_result_evidence_exists(
    comments: &[RemoteComment],
    expected: &ExecutorResultEvidence,
) -> bool {
    comments.iter().any(|comment| {
        parse_executor_result_evidence_comment(&comment.body)
            .is_ok_and(|evidence| evidence == *expected)
    })
}

pub fn successful_executor_result_for_pull_request(
    comments: &[RemoteComment],
    pull_request: u64,
) -> Option<ExecutorResultEvidence> {
    let mut matching = comments.iter().filter_map(|comment| {
        parse_executor_result_evidence_comment(&comment.body)
            .ok()
            .filter(|evidence| evidence.outcome == "succeeded" && evidence.pr == Some(pull_request))
    });
    let evidence = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    Some(evidence)
}

fn parse_executor_result_evidence_comment(body: &str) -> Result<ExecutorResultEvidence, String> {
    if body.matches(EXECUTOR_RESULT_BEGIN_MARKER).count() != 1
        || body.matches(EXECUTOR_RESULT_END_MARKER).count() != 1
    {
        return Err("executor result evidence markers must occur exactly once".to_string());
    }
    let (_, after_begin) = body
        .split_once(EXECUTOR_RESULT_BEGIN_MARKER)
        .ok_or_else(|| "missing executor result evidence begin marker".to_string())?;
    let (payload, _) = after_begin
        .split_once(EXECUTOR_RESULT_END_MARKER)
        .ok_or_else(|| "missing executor result evidence end marker".to_string())?;
    ExecutorResultEvidence::parse_json(payload.trim())
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
    pub claim_id: Option<String>,
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
            claim_id: None,
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
                "claim_id",
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
            claim_id: take_optional_string(&mut object, "claim_id", "run-state record")?,
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
        let claim_id = self.claim_id.as_ref().map_or_else(String::new, |claim_id| {
            format!(",\"claim_id\":\"{}\"", escape_json(claim_id))
        });
        format!(
            "{{\"schema\":1,\"repo\":\"{}\",\"issue\":{},\"worker_id\":\"{}\",\"state\":\"{}\",\"branch\":\"{}\",\"pr\":\"{}\",\"step\":\"{}\",\"paths\":[{}],\"claimed_at\":\"{}\",\"updated_at\":\"{}\",\"ttl_seconds\":{}{}}}",
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
            claim_id,
        )
    }

    pub fn with_claim_id(mut self, claim_id: impl Into<String>) -> Self {
        self.claim_id = Some(claim_id.into());
        self
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
        if self
            .claim_id
            .as_ref()
            .is_some_and(|claim_id| claim_id.trim().is_empty())
        {
            return Err("run-state claim_id must not be empty".to_string());
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

/// Return the exact marked comment a losing worker may delete.
///
/// The CAS owner is the lowest marked comment ID. A worker that loses claim
/// confirmation may self-clean only a higher-ID comment whose parsed
/// `worker_id` is literally equal to its own ID. This keeps dotted IDs distinct
/// from near-collisions and prevents regex-like matching from deleting another
/// worker's comment.
pub fn claim_losing_worker_comment_id(comments: &[RemoteComment], worker_id: &str) -> Option<u64> {
    let lowest = lowest_marked_comment(comments).map(|comment| comment.id);
    comments
        .iter()
        .filter_map(|comment| {
            parse_run_state_comment(&comment.body)
                .ok()
                .filter(|record| record.worker_id == worker_id)
                .map(|_| comment.id)
        })
        .filter(|comment_id| Some(*comment_id) != lowest)
        .max()
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

fn optional_json_string(value: Option<&str>) -> String {
    value.map_or_else(
        || "null".to_string(),
        |value| format!("\"{}\"", escape_json(value)),
    )
}
