use std::collections::{BTreeMap, BTreeSet};

use crate::claim::{evaluate_claim_safety_with_trusted_actors, ClaimSafetyInput};
use crate::state::json::{JsonParser, JsonValue};

const SERIAL_LABELS: &[&str] = &[
    "reasoning:deep",
    "priority:high",
    "regression",
    "audit",
    "release",
];
const BLOCKING_LABELS: &[(&str, &str)] = &[
    ("needs-classify", "needs_classify"),
    ("groom:proposed", "groom_proposed"),
    ("autospec:needs-human", "autospec_needs_human"),
    (
        "autospec:blocked-prerequisite",
        "security_prerequisite_blocked",
    ),
];
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteIssue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub author: String,
    pub closed: bool,
}

impl RemoteIssue {
    pub fn open(
        number: u64,
        title: impl Into<String>,
        body: impl Into<String>,
        labels: Vec<String>,
        author: impl Into<String>,
    ) -> Self {
        Self {
            number,
            title: title.into(),
            body: body.into(),
            labels,
            author: author.into(),
            closed: false,
        }
    }

    pub fn closed(
        number: u64,
        title: impl Into<String>,
        body: impl Into<String>,
        labels: Vec<String>,
        author: impl Into<String>,
    ) -> Self {
        Self {
            closed: true,
            ..Self::open(number, title, body, labels, author)
        }
    }

    fn has_label(&self, label: &str) -> bool {
        self.labels.iter().any(|candidate| candidate == label)
    }

    fn safety_input(&self) -> ClaimSafetyInput {
        ClaimSafetyInput::new(
            self.labels.clone(),
            self.title.clone(),
            self.body.clone(),
            self.author.clone(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteIssuePage {
    pub issues: Vec<RemoteIssue>,
    pub raw_count: usize,
}

pub fn parse_remote_issue_list_json(input: &str) -> Result<Vec<RemoteIssue>, String> {
    let values = JsonParser::new(input)
        .parse()?
        .into_array("GitHub queue issue list")?;
    parse_remote_issues(values)
}

pub fn parse_remote_issue_page_json(input: &str) -> Result<RemoteIssuePage, String> {
    let value = JsonParser::new(input).parse()?;
    match value {
        JsonValue::Array(values) => {
            let raw_count = values.len();
            Ok(RemoteIssuePage {
                issues: parse_remote_issues(values)?,
                raw_count,
            })
        }
        JsonValue::Object(mut object) => {
            const CONTEXT: &str = "GitHub queue issue page";
            reject_unknown_keys(&object, &["raw_count", "items"], CONTEXT)?;
            let raw_count = take_required(&mut object, "raw_count", CONTEXT)?
                .into_number(&format!("{CONTEXT}.raw_count"))?;
            let raw_count = usize::try_from(raw_count)
                .map_err(|_| format!("{CONTEXT}.raw_count exceeds this platform"))?;
            let values = take_required(&mut object, "items", CONTEXT)?
                .into_array(&format!("{CONTEXT}.items"))?;
            Ok(RemoteIssuePage {
                issues: parse_remote_issues(values)?,
                raw_count,
            })
        }
        _ => Err("GitHub queue issue page must be an array or object".to_string()),
    }
}

fn parse_remote_issues(values: Vec<JsonValue>) -> Result<Vec<RemoteIssue>, String> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            parse_issue(value, &format!("GitHub queue issue list[{index}]"), None)
        })
        .collect()
}

pub fn parse_dependency_issue_json(input: &str, number: u64) -> Result<RemoteIssue, String> {
    parse_issue(
        JsonParser::new(input).parse()?,
        "GitHub queue dependency issue",
        Some(number),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePullRequestCheck {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
}

impl RemotePullRequestCheck {
    pub fn in_progress(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: "IN_PROGRESS".to_string(),
            conclusion: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePullRequest {
    pub number: u64,
    pub open: bool,
    pub body: String,
    pub checks: Vec<RemotePullRequestCheck>,
}

impl RemotePullRequest {
    pub fn open(number: u64, body: impl Into<String>, checks: Vec<RemotePullRequestCheck>) -> Self {
        Self {
            number,
            open: true,
            body: body.into(),
            checks,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePullRequestPage {
    pub pull_requests: Vec<RemotePullRequest>,
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

pub fn parse_remote_pull_requests_json(input: &str) -> Result<Vec<RemotePullRequest>, String> {
    let values = JsonParser::new(input)
        .parse()?
        .into_array("GitHub queue pull request list")?;
    parse_remote_pull_requests(values)
}

pub fn parse_remote_pull_request_page_json(input: &str) -> Result<RemotePullRequestPage, String> {
    let value = JsonParser::new(input).parse()?;
    match value {
        JsonValue::Array(values) => Ok(RemotePullRequestPage {
            pull_requests: parse_remote_pull_requests(values)?,
            has_next_page: false,
            end_cursor: None,
        }),
        JsonValue::Object(mut object) => {
            const CONTEXT: &str = "GitHub queue pull request page";
            reject_unknown_keys(&object, &["items", "page_info"], CONTEXT)?;
            let values = take_required(&mut object, "items", CONTEXT)?
                .into_array(&format!("{CONTEXT}.items"))?;
            let mut page_info = take_required(&mut object, "page_info", CONTEXT)?
                .into_object(&format!("{CONTEXT}.page_info"))?;
            reject_unknown_keys(
                &page_info,
                &["has_next_page", "end_cursor"],
                &format!("{CONTEXT}.page_info"),
            )?;
            let has_next_page = take_required(
                &mut page_info,
                "has_next_page",
                &format!("{CONTEXT}.page_info"),
            )?
            .into_bool(&format!("{CONTEXT}.page_info.has_next_page"))?;
            let end_cursor = take_required(
                &mut page_info,
                "end_cursor",
                &format!("{CONTEXT}.page_info"),
            )?
            .into_optional_string(&format!("{CONTEXT}.page_info.end_cursor"))?;
            if has_next_page && end_cursor.is_none() {
                return Err(format!(
                    "{CONTEXT}.page_info.end_cursor is required when another page exists"
                ));
            }
            Ok(RemotePullRequestPage {
                pull_requests: parse_remote_pull_requests(values)?,
                has_next_page,
                end_cursor,
            })
        }
        _ => Err("GitHub queue pull request page must be an array or object".to_string()),
    }
}

fn parse_remote_pull_requests(values: Vec<JsonValue>) -> Result<Vec<RemotePullRequest>, String> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("GitHub queue pull request list[{index}]");
            let mut object = value.into_object(&context)?;
            reject_unknown_keys(
                &object,
                &["number", "state", "body", "statusCheckRollup"],
                &context,
            )?;
            let number = take_required(&mut object, "number", &context)?
                .into_number(&format!("{context}.number"))?;
            let state = take_optional_string(&mut object, "state", &context)?
                .unwrap_or_else(|| "OPEN".to_string());
            let body = take_optional_string(&mut object, "body", &context)?.unwrap_or_default();
            let checks = take_optional(&mut object, "statusCheckRollup")
                .unwrap_or(JsonValue::Array(Vec::new()))
                .into_array(&format!("{context}.statusCheckRollup"))?
                .into_iter()
                .enumerate()
                .map(|(check_index, value)| {
                    let check_context = format!("{context}.statusCheckRollup[{check_index}]");
                    let mut check = value.into_object(&check_context)?;
                    reject_unknown_keys(&check, &["name", "status", "conclusion"], &check_context)?;
                    Ok(RemotePullRequestCheck {
                        name: take_optional_string(&mut check, "name", &check_context)?
                            .unwrap_or_default(),
                        status: take_optional_string(&mut check, "status", &check_context)?
                            .unwrap_or_default(),
                        conclusion: take_optional_string(&mut check, "conclusion", &check_context)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(RemotePullRequest {
                number,
                open: state.eq_ignore_ascii_case("OPEN"),
                body,
                checks,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestEvidence {
    Available(Vec<RemotePullRequest>),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuePolicy {
    pub batch_size: usize,
    pub max_repo_workers: usize,
    pub only_issues: BTreeSet<u64>,
    pub non_blocking_dependency_labels: BTreeSet<String>,
}

impl QueuePolicy {
    pub fn new(batch_size: usize, max_repo_workers: usize) -> Self {
        Self {
            batch_size: batch_size.max(1),
            max_repo_workers,
            only_issues: BTreeSet::new(),
            non_blocking_dependency_labels: ["epic".to_string(), "umbrella".to_string()]
                .into_iter()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyQueueInput {
    pub candidates: Vec<RemoteIssue>,
    pub active: Vec<RemoteIssue>,
    pub dependencies: BTreeMap<u64, RemoteIssue>,
    pub pull_requests: PullRequestEvidence,
    pub policy: QueuePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonBlockingReference {
    pub issue: u64,
    pub reason: String,
    pub cycle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyGate {
    pub ok: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueIssueView {
    pub issue: RemoteIssue,
    pub reason: Option<String>,
    pub blocked_label: Option<String>,
    pub safety_gate: Option<SafetyGate>,
    pub linked_pr: Option<u64>,
    pub unmet_dependencies: Vec<u64>,
    pub cycle_dependencies: Vec<u64>,
    pub non_blocking_refs: Vec<NonBlockingReference>,
    pub conflicts_with: Option<u64>,
    pub path: Option<String>,
    pub paths: Vec<String>,
    pub serialization_reasons: Vec<String>,
    pub parallel_safe: Option<bool>,
}

impl QueueIssueView {
    fn plain(issue: RemoteIssue) -> Self {
        Self {
            issue,
            reason: None,
            blocked_label: None,
            safety_gate: None,
            linked_pr: None,
            unmet_dependencies: Vec::new(),
            cycle_dependencies: Vec::new(),
            non_blocking_refs: Vec::new(),
            conflicts_with: None,
            path: None,
            paths: Vec::new(),
            serialization_reasons: Vec::new(),
            parallel_safe: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerCap {
    pub max_repo_workers: usize,
    pub active_count: usize,
    pub remaining: usize,
    pub reached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueueGateCounts {
    pub open: usize,
    pub candidate: usize,
    pub reviewed: usize,
    pub blocked: usize,
    pub dependency_blocked: usize,
    pub linked_pr_blocked: usize,
    pub path_conflicted: usize,
    pub ready: usize,
    pub claimed: usize,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyQueuePlan {
    pub ready: Vec<QueueIssueView>,
    pub blocked: Vec<QueueIssueView>,
    pub claimed: Vec<RemoteIssue>,
    pub conflicts: Vec<QueueIssueView>,
    pub worker_cap: WorkerCap,
    pub batch: Vec<QueueIssueView>,
    pub gate_counts: QueueGateCounts,
}

impl ReadyQueuePlan {
    pub fn ready_numbers(&self) -> Vec<u64> {
        self.ready.iter().map(|view| view.issue.number).collect()
    }

    pub fn batch_numbers(&self) -> Vec<u64> {
        self.batch.iter().map(|view| view.issue.number).collect()
    }
}

pub fn plan_ready_queue(input: &ReadyQueueInput) -> ReadyQueuePlan {
    plan_ready_queue_with_trusted_actors(input, &["berlinguyinca"])
}

/// Plan a ready queue with the configured trusted actors used by the current
/// issue-intent policy. The planner stays pure: the CLI owns config loading.
pub fn plan_ready_queue_with_trusted_actors(
    input: &ReadyQueueInput,
    trusted_actors: &[&str],
) -> ReadyQueuePlan {
    let candidates = deduplicate_issues(&input.candidates);
    let open_count = candidates.iter().filter(|issue| !issue.closed).count();
    let mut known = input.dependencies.clone();
    for issue in &candidates {
        known.insert(issue.number, issue.clone());
    }
    let active = deduplicate_issues(&input.active);
    let active_paths = active
        .iter()
        .map(|issue| (issue.number, extract_paths(&issue.body)))
        .collect::<Vec<_>>();
    let worker_cap = worker_cap(&input.policy, active.len());
    let effective_batch_size = if worker_cap.reached {
        0
    } else if input.policy.max_repo_workers > 0 {
        input.policy.batch_size.min(worker_cap.remaining)
    } else {
        input.policy.batch_size
    };

    let mut ready: Vec<QueueIssueView> = Vec::new();
    let mut blocked: Vec<QueueIssueView> = Vec::new();
    let mut conflicts: Vec<QueueIssueView> = Vec::new();
    let mut candidate_count = 0;
    let mut reviewed_count = 0;
    for issue in candidates {
        if issue.closed {
            continue;
        }
        if !input.policy.only_issues.is_empty() && !input.policy.only_issues.contains(&issue.number)
        {
            continue;
        }
        candidate_count += 1;
        if issue.has_label("safety:reviewed") {
            reviewed_count += 1;
        }
        let mut view = QueueIssueView::plain(issue);
        if let Some((label, reason)) = BLOCKING_LABELS
            .iter()
            .find(|(label, _)| view.issue.has_label(label))
        {
            view.reason = Some(reason.to_string());
            view.blocked_label = Some(label.to_string());
            blocked.push(view);
            continue;
        }
        if !view.issue.has_label("auto-implement") {
            view.reason = Some("missing_auto_implement".to_string());
            blocked.push(view);
            continue;
        }
        let safety =
            evaluate_claim_safety_with_trusted_actors(&view.issue.safety_input(), trusted_actors);
        if !safety.allowed {
            view.reason = Some("safety_gate_failed".to_string());
            view.safety_gate = Some(SafetyGate {
                ok: false,
                reason: safety.reason.to_string(),
            });
            blocked.push(view);
            continue;
        }
        match linked_pr_with_nonterminal_checks(&input.pull_requests, view.issue.number) {
            LinkedPr::Unavailable => {
                view.reason = Some("linked_pr_evidence_unavailable".to_string());
                blocked.push(view);
                continue;
            }
            LinkedPr::Open(number) => {
                view.reason = Some("linked_pr_open".to_string());
                view.linked_pr = Some(number);
                blocked.push(view);
                continue;
            }
            LinkedPr::None => {}
        }

        let (unmet_dependencies, cycle_dependencies, non_blocking_refs) =
            evaluate_dependencies(&view.issue, &known, &input.policy);
        if !unmet_dependencies.is_empty() {
            view.reason = Some(
                if cycle_dependencies.is_empty() {
                    "blocked_dependencies"
                } else {
                    "blocked_cycle"
                }
                .to_string(),
            );
            view.unmet_dependencies = unmet_dependencies;
            view.cycle_dependencies = cycle_dependencies;
            view.non_blocking_refs = non_blocking_refs;
            blocked.push(view);
            continue;
        }

        view.paths = extract_paths(&view.issue.body);
        view.non_blocking_refs = non_blocking_refs;
        if let Some((number, path)) = first_path_conflict(&view.paths, &active_paths) {
            view.reason = Some("path_conflict".to_string());
            view.conflicts_with = Some(number);
            view.path = Some(path);
            conflicts.push(view);
            continue;
        }
        if let Some((number, path)) = first_path_conflict(
            &view.paths,
            &ready
                .iter()
                .map(|ready| (ready.issue.number, ready.paths.clone()))
                .collect::<Vec<_>>(),
        ) {
            view.reason = Some("batch_path_conflict".to_string());
            view.conflicts_with = Some(number);
            view.path = Some(path);
            conflicts.push(view);
            continue;
        }
        view.serialization_reasons = serialization_reasons(&view.issue);
        view.parallel_safe = Some(view.serialization_reasons.is_empty());
        ready.push(view);
    }

    let batch = if effective_batch_size == 0 {
        Vec::new()
    } else if ready
        .first()
        .is_some_and(|issue| issue.parallel_safe == Some(false))
    {
        ready.first().cloned().into_iter().collect()
    } else {
        ready
            .iter()
            .filter(|issue| issue.parallel_safe != Some(false))
            .take(effective_batch_size)
            .cloned()
            .collect()
    };
    let mut plan = ReadyQueuePlan {
        ready,
        blocked,
        claimed: active,
        conflicts,
        worker_cap,
        batch,
        gate_counts: QueueGateCounts::default(),
    };
    plan.gate_counts = queue_gate_counts(&plan, open_count, candidate_count, reviewed_count);
    plan
}

fn deduplicate_issues(issues: &[RemoteIssue]) -> Vec<RemoteIssue> {
    let mut deduplicated = BTreeMap::new();
    for issue in issues {
        deduplicated
            .entry(issue.number)
            .or_insert_with(|| issue.clone());
    }
    deduplicated.into_values().collect()
}

fn queue_gate_counts(
    plan: &ReadyQueuePlan,
    open: usize,
    candidate: usize,
    reviewed: usize,
) -> QueueGateCounts {
    QueueGateCounts {
        open,
        candidate,
        reviewed,
        blocked: plan.blocked.len(),
        dependency_blocked: plan
            .blocked
            .iter()
            .filter(|view| {
                matches!(
                    view.reason.as_deref(),
                    Some("blocked_dependencies") | Some("blocked_cycle")
                )
            })
            .count(),
        linked_pr_blocked: plan
            .blocked
            .iter()
            .filter(|view| {
                view.reason
                    .as_deref()
                    .is_some_and(|reason| reason.starts_with("linked_pr_"))
            })
            .count(),
        path_conflicted: plan.conflicts.len(),
        ready: plan.ready.len(),
        claimed: plan.claimed.len(),
        selected: plan.batch.len(),
    }
}

fn worker_cap(policy: &QueuePolicy, active_count: usize) -> WorkerCap {
    let remaining = if policy.max_repo_workers == 0 {
        policy.batch_size
    } else {
        policy.max_repo_workers.saturating_sub(active_count)
    };
    WorkerCap {
        max_repo_workers: policy.max_repo_workers,
        active_count,
        remaining,
        reached: policy.max_repo_workers > 0 && active_count >= policy.max_repo_workers,
    }
}

enum LinkedPr {
    None,
    Open(u64),
    Unavailable,
}

fn linked_pr_with_nonterminal_checks(evidence: &PullRequestEvidence, issue: u64) -> LinkedPr {
    let PullRequestEvidence::Available(pull_requests) = evidence else {
        return LinkedPr::Unavailable;
    };
    let mut matching = pull_requests
        .iter()
        .filter(|pull_request| {
            pull_request.open
                && references_issue(&pull_request.body, issue)
                && (pull_request.checks.is_empty()
                    || pull_request.checks.iter().any(|check| {
                        !check.status.eq_ignore_ascii_case("COMPLETED")
                            || check.conclusion.is_none()
                    }))
        })
        .map(|pull_request| pull_request.number)
        .collect::<Vec<_>>();
    matching.sort_unstable();
    matching
        .into_iter()
        .next()
        .map_or(LinkedPr::None, LinkedPr::Open)
}

fn references_issue(body: &str, issue: u64) -> bool {
    let lower = body.to_ascii_lowercase();
    let issue = issue.to_string();
    [
        "close", "closed", "closes", "fix", "fixed", "fixes", "resolve", "resolved", "resolves",
    ]
    .iter()
    .any(|verb| contains_issue_reference(&lower, verb, &issue))
}

fn contains_issue_reference(text: &str, verb: &str, issue: &str) -> bool {
    let mut start = 0;
    while let Some(offset) = text[start..].find(verb) {
        let index = start + offset;
        let before = text[..index].chars().next_back();
        let mut reference_start = index + verb.len();
        while text
            .as_bytes()
            .get(reference_start)
            .is_some_and(u8::is_ascii_whitespace)
        {
            reference_start += 1;
        }
        let has_whitespace = reference_start > index + verb.len();
        let issue_start = reference_start + 1;
        let has_issue = text.as_bytes().get(reference_start) == Some(&b'#')
            && text[issue_start..].starts_with(issue);
        let after = has_issue
            .then_some(issue_start + issue.len())
            .and_then(|end| text[end..].chars().next());
        if has_whitespace
            && has_issue
            && before.is_none_or(|character| !character.is_ascii_alphanumeric())
            && after.is_none_or(|character| !character.is_ascii_digit())
        {
            return true;
        }
        start = index + verb.len();
    }
    false
}

fn evaluate_dependencies(
    issue: &RemoteIssue,
    known: &BTreeMap<u64, RemoteIssue>,
    policy: &QueuePolicy,
) -> (Vec<u64>, Vec<u64>, Vec<NonBlockingReference>) {
    let mut unmet = Vec::new();
    let mut cycles = Vec::new();
    let mut refs = Vec::new();
    for dependency in dependency_numbers(&issue.body) {
        let target = known.get(&dependency).cloned().unwrap_or_else(|| {
            RemoteIssue::open(
                dependency,
                format!("issue-{dependency}"),
                "",
                Vec::new(),
                "",
            )
        });
        if target.closed {
            continue;
        }
        if target.labels.iter().any(|label| {
            policy
                .non_blocking_dependency_labels
                .contains(&label.to_ascii_lowercase())
        }) {
            refs.push(NonBlockingReference {
                issue: dependency,
                reason: "epic_label".to_string(),
                cycle: false,
            });
            continue;
        }
        if target_tracks_issue(&target.body, issue.number) {
            refs.push(NonBlockingReference {
                issue: dependency,
                reason: "children_back_edge".to_string(),
                cycle: true,
            });
            continue;
        }
        if dependency_reaches(&target, issue.number, known, &mut BTreeSet::new()) {
            cycles.push(dependency);
        }
        unmet.push(dependency);
    }
    (unmet, cycles, refs)
}

fn dependency_reaches(
    issue: &RemoteIssue,
    target: u64,
    known: &BTreeMap<u64, RemoteIssue>,
    seen: &mut BTreeSet<u64>,
) -> bool {
    if !seen.insert(issue.number) {
        return false;
    }
    dependency_numbers(&issue.body)
        .into_iter()
        .any(|dependency| {
            dependency == target
                || known
                    .get(&dependency)
                    .is_some_and(|next| dependency_reaches(next, target, known, seen))
        })
}

pub fn dependency_numbers(body: &str) -> Vec<u64> {
    let section = markdown_section(body, "Dependencies");
    let mut dependencies = BTreeSet::new();
    for line in section.lines() {
        let lower = line.to_ascii_lowercase();
        let mut cursor = 0;
        while let Some(offset) = lower[cursor..].find("depends on") {
            let mut candidate = &line[cursor + offset + "depends on".len()..];
            candidate = candidate.trim_start();
            if candidate.to_ascii_lowercase().starts_with("issue") {
                candidate = candidate["issue".len()..].trim_start();
            }
            candidate = candidate.strip_prefix('#').unwrap_or(candidate);
            let digits = candidate
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>();
            if let Ok(dependency) = digits.parse::<u64>() {
                dependencies.insert(dependency);
            }
            cursor += offset + "depends on".len();
        }
    }
    dependencies.into_iter().collect()
}

fn target_tracks_issue(body: &str, dependent: u64) -> bool {
    let section = markdown_sections(
        body,
        &["children", "child issues", "tasks", "task list", "subtasks"],
    );
    let needle = format!("#{dependent}");
    section.lines().any(|line| {
        let trimmed = line.trim_start();
        let list_item = trimmed.starts_with("- ") || trimmed.starts_with("* ");
        let at_boundary = trimmed.find(&needle).is_some_and(|index| {
            trimmed[index + needle.len()..]
                .chars()
                .next()
                .is_none_or(|character| !character.is_ascii_digit())
        });
        list_item && at_boundary
    })
}

fn extract_paths(body: &str) -> Vec<String> {
    let section = markdown_section(body, "Implementation outline");
    let mut paths = BTreeSet::new();
    let mut cursor = 0;
    while let Some(start) = section[cursor..].find('`') {
        let start = cursor + start + 1;
        let Some(end) = section[start..].find('`') else {
            break;
        };
        let path = &section[start..start + end];
        if path.contains('/') && !path.chars().any(char::is_whitespace) {
            paths.insert(path.to_string());
        }
        cursor = start + end + 1;
    }
    paths.into_iter().collect()
}

fn first_path_conflict(
    candidate: &[String],
    others: &[(u64, Vec<String>)],
) -> Option<(u64, String)> {
    for (number, paths) in others {
        for path in candidate {
            if paths.binary_search(path).is_ok() || paths.iter().any(|other| other == path) {
                return Some((*number, path.clone()));
            }
        }
    }
    None
}

fn serialization_reasons(issue: &RemoteIssue) -> Vec<String> {
    SERIAL_LABELS
        .iter()
        .filter(|label| issue.labels.iter().any(|current| current == **label))
        .map(|label| (*label).to_string())
        .collect()
}

fn markdown_section<'a>(body: &'a str, name: &str) -> &'a str {
    markdown_sections(body, &[name])
}

fn markdown_sections<'a>(body: &'a str, names: &[&str]) -> &'a str {
    let mut start = None;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let text = line.trim_end_matches(['\r', '\n']);
        if let Some(section_start) = start {
            if text.starts_with("## ") {
                return &body[section_start..offset];
            }
        } else if let Some(heading) = text.strip_prefix("## ") {
            let heading = heading.trim().to_ascii_lowercase();
            if names
                .iter()
                .any(|name| heading == name.to_ascii_lowercase())
            {
                start = Some(offset + line.len());
            }
        }
        offset += line.len();
    }
    start.map_or("", |section_start| &body[section_start..])
}

fn parse_issue(
    value: JsonValue,
    context: &str,
    fallback_number: Option<u64>,
) -> Result<RemoteIssue, String> {
    let mut object = value.into_object(context)?;
    reject_unknown_keys(
        &object,
        &["number", "title", "body", "labels", "author", "state"],
        context,
    )?;
    let number = match take_optional(&mut object, "number") {
        Some(value) => value.into_number(&format!("{context}.number"))?,
        None => fallback_number.ok_or_else(|| format!("{context}.number is required"))?,
    };
    let labels = take_optional(&mut object, "labels")
        .unwrap_or(JsonValue::Array(Vec::new()))
        .into_array(&format!("{context}.labels"))?
        .into_iter()
        .enumerate()
        .map(|(index, value)| parse_label(value, &format!("{context}.labels[{index}]")))
        .collect::<Result<Vec<_>, _>>()?;
    let author = match take_optional(&mut object, "author") {
        None | Some(JsonValue::Null) => String::new(),
        Some(value) => {
            let mut author = value.into_object(&format!("{context}.author"))?;
            reject_unknown_keys(&author, &["login"], &format!("{context}.author"))?;
            take_optional_string(&mut author, "login", &format!("{context}.author"))?
                .unwrap_or_default()
        }
    };
    let state =
        take_optional_string(&mut object, "state", context)?.unwrap_or_else(|| "OPEN".to_string());
    Ok(RemoteIssue {
        number,
        title: take_optional_string(&mut object, "title", context)?.unwrap_or_default(),
        body: take_optional_string(&mut object, "body", context)?.unwrap_or_default(),
        labels,
        author,
        closed: state.eq_ignore_ascii_case("CLOSED"),
    })
}

fn parse_label(value: JsonValue, context: &str) -> Result<String, String> {
    match value {
        JsonValue::String(label) => Ok(label),
        JsonValue::Object(mut label) => {
            reject_unknown_keys(&label, &["name"], context)?;
            take_required(&mut label, "name", context)?.into_string(&format!("{context}.name"))
        }
        _ => Err(format!("{context} must be a label object or string")),
    }
}

fn take_required(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("{context}.{key} is required"))
}

fn take_optional(object: &mut BTreeMap<String, JsonValue>, key: &str) -> Option<JsonValue> {
    object.remove(key)
}

fn take_optional_string(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Option<String>, String> {
    match take_optional(object, key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => value.into_string(&format!("{context}.{key}")).map(Some),
    }
}

fn reject_unknown_keys(
    object: &BTreeMap<String, JsonValue>,
    allowed: &[&str],
    context: &str,
) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{context} contains unknown key: {key}"));
    }
    Ok(())
}
