use std::collections::BTreeSet;
use std::process::Command;

use super::gh_read::run_command_read_with_retry;

use autospec_core::autonomous::tier15::{observe_tier15, Tier15Input, Tier15Observation};
use autospec_core::coordination::{RemoteIssue, RemoteIssuePage};

mod strict;

use strict::parse_projected_page_bytes;

const PAGE_SIZE: usize = 100;
const MAX_PAGES_PER_STATE: usize = 10_000;

pub(super) const ISSUE_PAGE_PROJECTION: &str = "{raw_count:length,items:[.[] | select(.pull_request == null) | {number,title:(.title // \"\"),body:(.body // \"\"),labels:[.labels[].name],author:{login:(.user.login // \"\")},state:(.state // \"OPEN\")}]}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IssueState {
    Open,
    Closed,
}

impl IssueState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Tier15Scan {
    Complete(Tier15Observation),
    Failed(String),
}

pub(super) fn scan(repo: &str, budget: usize) -> Tier15Scan {
    match repository_snapshot(repo) {
        Ok((open, closed)) => observe_snapshot(open, closed, budget),
        Err(error) => Tier15Scan::Failed(error),
    }
}

pub(super) fn scan_with<F>(budget: usize, mut fetch_page: F) -> Tier15Scan
where
    F: FnMut(IssueState, usize) -> Result<RemoteIssuePage, String>,
{
    let open = match collect_snapshot(&mut fetch_page, IssueState::Open) {
        Ok(issues) => issues,
        Err(error) => return Tier15Scan::Failed(error),
    };
    let closed = match collect_snapshot(&mut fetch_page, IssueState::Closed) {
        Ok(issues) => issues,
        Err(error) => return Tier15Scan::Failed(error),
    };
    observe_snapshot(open, closed, budget)
}

pub(super) fn repository_issues(repo: &str) -> Result<Vec<RemoteIssue>, String> {
    let (mut open, closed) = repository_snapshot(repo)?;
    open.extend(closed);
    Ok(open)
}

fn repository_snapshot(repo: &str) -> Result<(Vec<RemoteIssue>, Vec<RemoteIssue>), String> {
    validate_repo(repo)?;
    let mut fetch = |state, page| fetch_page(repo, state, page);
    let open = collect_snapshot(&mut fetch, IssueState::Open)?;
    let closed = collect_snapshot(&mut fetch, IssueState::Closed)?;
    validate_disjoint(&open, &closed)?;
    Ok((open, closed))
}

fn observe_snapshot(open: Vec<RemoteIssue>, closed: Vec<RemoteIssue>, budget: usize) -> Tier15Scan {
    if let Err(error) = validate_disjoint(&open, &closed) {
        return Tier15Scan::Failed(error);
    }
    match observe_tier15(Tier15Input::new(open, closed, budget)) {
        Ok(observation) => Tier15Scan::Complete(observation),
        Err(error) => Tier15Scan::Failed(format!("tier1_5 observer failed: {error}")),
    }
}

fn validate_disjoint(open: &[RemoteIssue], closed: &[RemoteIssue]) -> Result<(), String> {
    let open_numbers = open
        .iter()
        .map(|issue| issue.number)
        .collect::<BTreeSet<_>>();
    if let Some(issue) = closed
        .iter()
        .find(|issue| open_numbers.contains(&issue.number))
    {
        return Err(format!(
            "tier1_5 open and closed snapshots overlap issue {}",
            issue.number
        ));
    }
    Ok(())
}

fn collect_snapshot<F>(fetch_page: &mut F, state: IssueState) -> Result<Vec<RemoteIssue>, String>
where
    F: FnMut(IssueState, usize) -> Result<RemoteIssuePage, String>,
{
    let mut issues = Vec::new();
    let mut seen_numbers = BTreeSet::new();
    let mut page_number = 1usize;

    for _ in 0..MAX_PAGES_PER_STATE {
        let page = fetch_page(state, page_number).map_err(|error| {
            format!(
                "tier1_5 {} page {page_number} failed: {error}",
                state.as_str()
            )
        })?;
        if page.raw_count > PAGE_SIZE || page.issues.len() > page.raw_count {
            return Err(format!(
                "tier1_5 {} page {page_number} reported invalid raw count {}",
                state.as_str(),
                page.raw_count
            ));
        }
        let is_last_page = page.raw_count < PAGE_SIZE;
        for issue in page.issues {
            if !seen_numbers.insert(issue.number) {
                return Err(format!(
                    "tier1_5 {} snapshot repeated issue {} on page {page_number}",
                    state.as_str(),
                    issue.number
                ));
            }
            issues.push(issue);
        }
        if is_last_page {
            return Ok(issues);
        }
        page_number = next_page(page_number)?;
    }

    Err(format!(
        "tier1_5 {} pagination could not demonstrate termination within {MAX_PAGES_PER_STATE} pages",
        state.as_str()
    ))
}

fn fetch_page(repo: &str, state: IssueState, page: usize) -> Result<RemoteIssuePage, String> {
    validate_repo(repo)?;
    let arguments = gh_api_arguments(repo, state, page);
    let output = run_command_read_with_retry(
        || {
            let mut command = Command::new("gh");
            command.args(&arguments);
            command
        },
        "read Tier 1.5 issue page",
    )
    .map_err(|error| error.to_string())?;
    parse_projected_page_bytes(&output.stdout)
}

fn gh_api_arguments(repo: &str, state: IssueState, page: usize) -> Vec<String> {
    let endpoint = format!(
        "repos/{repo}/issues?state={}&per_page={PAGE_SIZE}&page={page}",
        state.as_str()
    );
    vec![
        "api".to_string(),
        "--method".to_string(),
        "GET".to_string(),
        endpoint,
        "--jq".to_string(),
        ISSUE_PAGE_PROJECTION.to_string(),
    ]
}

fn parse_projected_page(input: &str) -> Result<RemoteIssuePage, String> {
    parse_projected_page_bytes(input.as_bytes())
}

fn validate_repo(repo: &str) -> Result<(), String> {
    let Some((owner, name)) = repo.split_once('/') else {
        return Err(format!(
            "GitHub repository must use OWNER/REPO form: {repo}"
        ));
    };
    if !is_valid_owner(owner) || !is_valid_repository_name(name) {
        return Err(format!(
            "GitHub repository must use OWNER/REPO form: {repo}"
        ));
    }
    Ok(())
}

fn is_valid_owner(owner: &str) -> bool {
    (1..=39).contains(&owner.len())
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && owner
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && owner
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
}

fn is_valid_repository_name(name: &str) -> bool {
    (1..=100).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && name
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && !name.contains("..")
        && !name.to_ascii_lowercase().ends_with(".git")
}

fn next_page(page: usize) -> Result<usize, String> {
    page.checked_add(1)
        .ok_or_else(|| "tier1_5 GitHub page number overflowed".to_string())
}

#[cfg(test)]
mod tests {
    use autospec_core::autonomous::tier15::Tier15Decision;
    use autospec_core::coordination::{RemoteIssue, RemoteIssuePage};

    use super::strict::parse_projected_page_bytes;
    use super::{
        gh_api_arguments, next_page, parse_projected_page, scan, scan_with, validate_repo,
        IssueState, Tier15Scan, ISSUE_PAGE_PROJECTION,
    };

    fn open(number: u64) -> RemoteIssue {
        RemoteIssue::open(
            number,
            format!("feat: issue {number}"),
            "Fix: the adapter must retain this complete issue snapshot.",
            Vec::new(),
            "autospec",
        )
    }

    fn closed(number: u64) -> RemoteIssue {
        RemoteIssue::closed(
            number,
            format!("closed issue {number}"),
            "A complete closed snapshot prevents a duplicate candidate.",
            Vec::new(),
            "autospec",
        )
    }

    fn page(raw_count: usize, issues: Vec<RemoteIssue>) -> RemoteIssuePage {
        RemoteIssuePage { raw_count, issues }
    }

    #[test]
    fn scan_collects_all_open_pages_before_closed_snapshot() {
        let mut calls = Vec::new();
        let scan = scan_with(2, |state, page_number| {
            calls.push((state, page_number));
            match (state, page_number) {
                (IssueState::Open, 1) => Ok(page(100, vec![open(1)])),
                (IssueState::Open, 2) => Ok(page(1, vec![open(2)])),
                (IssueState::Closed, 1) => Ok(page(1, vec![closed(3)])),
                _ => panic!("unexpected page request: {state:?}/{page_number}"),
            }
        });

        assert_eq!(
            calls,
            vec![
                (IssueState::Open, 1),
                (IssueState::Open, 2),
                (IssueState::Closed, 1),
            ]
        );
        let Tier15Scan::Complete(observation) = scan else {
            panic!("complete snapshots must reach the pure observer");
        };
        assert_eq!(observation.produced_count(), 2);
        assert!(matches!(
            observation.decisions(),
            [
                Tier15Decision::Produced { number: 1, .. },
                Tier15Decision::Produced { number: 2, .. }
            ]
        ));
    }

    #[test]
    fn projected_page_keeps_raw_count_while_pull_requests_are_filtered() {
        let page = parse_projected_page(
            r#"{"raw_count":2,"items":[{"number":7,"title":"feat: issue","body":"Fix: retain filtered issue data for the observer.","labels":[],"author":{"login":"autospec"},"state":"OPEN"}]}"#,
        )
        .expect("strict projection parsed");

        assert_eq!(page.raw_count, 2);
        assert_eq!(page.issues.len(), 1);
        assert!(ISSUE_PAGE_PROJECTION.contains("select(.pull_request == null)"));
        assert!(ISSUE_PAGE_PROJECTION.contains("raw_count:length"));
        assert!(
            parse_projected_page("[]").is_err(),
            "Tier 1.5 must reject an array form because it lacks the unfiltered raw count"
        );
    }

    #[test]
    fn malformed_projected_data_fails_without_a_partial_dry_result() {
        let scan = scan_with(2, |_, _| parse_projected_page(r#"{"items":[]}"#));

        assert!(matches!(
            scan,
            Tier15Scan::Failed(reason) if reason.contains("could not parse projected")
        ));
    }

    #[test]
    fn invalid_projected_stdout_bytes_fail_without_a_partial_dry_result() {
        let scan = scan_with(2, |_, _| parse_projected_page_bytes(&[0xff]));

        assert!(matches!(
            scan,
            Tier15Scan::Failed(reason) if reason.contains("valid UTF-8")
        ));
    }

    #[test]
    fn projected_page_missing_required_issue_field_fails_closed() {
        let scan = scan_with(2, |_, _| {
            parse_projected_page_bytes(
                br#"{"raw_count":1,"items":[{"number":7,"title":"feat: issue","labels":[],"author":{"login":"autospec"},"state":"OPEN"}]}"#,
            )
        });

        assert!(matches!(
            scan,
            Tier15Scan::Failed(reason) if reason.contains("body is required")
        ));
    }

    #[test]
    fn projected_page_accepts_github_rest_lowercase_state() {
        let page = parse_projected_page_bytes(
            br#"{"raw_count":1,"items":[{"number":7,"title":"closed issue","body":"A complete closed snapshot retains REST state.","labels":[],"author":{"login":"autospec"},"state":"closed"}]}"#,
        )
        .expect("GitHub REST state parsed");

        assert!(page.issues[0].closed);
    }

    #[test]
    fn impossible_filtered_page_count_fails_without_a_partial_dry_result() {
        let scan = scan_with(2, |_, _| Ok(page(0, vec![open(1)])));

        assert!(matches!(
            scan,
            Tier15Scan::Failed(reason) if reason.contains("invalid raw count")
        ));
    }

    #[test]
    fn later_page_failure_fails_without_calling_the_observer() {
        let scan = scan_with(2, |state, page_number| match (state, page_number) {
            (IssueState::Open, 1) => Ok(page(100, vec![open(1)])),
            (IssueState::Open, 2) => Err("later open page failed".to_string()),
            _ => panic!("closed pages must not be read after an open failure"),
        });

        assert!(matches!(
            scan,
            Tier15Scan::Failed(reason) if reason.contains("later open page failed")
        ));
    }

    #[test]
    fn repeated_issue_across_pages_fails_closed() {
        let scan = scan_with(2, |state, page_number| match (state, page_number) {
            (IssueState::Open, 1) => Ok(page(100, vec![open(1)])),
            (IssueState::Open, 2) => Ok(page(1, vec![open(1)])),
            _ => panic!("the repeated page must fail before closed collection"),
        });

        assert!(matches!(
            scan,
            Tier15Scan::Failed(reason) if reason.contains("repeated issue 1")
        ));
    }

    #[test]
    fn issue_present_in_open_and_closed_snapshots_fails_closed() {
        let scan = scan_with(2, |state, page_number| match (state, page_number) {
            (IssueState::Open, 1) => Ok(page(1, vec![open(1)])),
            (IssueState::Closed, 1) => Ok(page(1, vec![closed(1)])),
            _ => panic!("unexpected page"),
        });

        assert!(matches!(
            scan,
            Tier15Scan::Failed(reason) if reason.contains("open and closed snapshots overlap issue 1")
        ));
    }

    #[test]
    fn nonterminating_pages_fail_closed_before_any_partial_observation() {
        let scan = scan_with(2, |state, page_number| match state {
            IssueState::Open => Ok(page(100, vec![open(page_number as u64)])),
            IssueState::Closed => panic!("open pagination must fail before closed collection"),
        });

        assert!(matches!(
            scan,
            Tier15Scan::Failed(reason) if reason.contains("could not demonstrate termination")
        ));
    }

    #[test]
    fn page_cursor_overflow_is_rejected() {
        assert!(next_page(usize::MAX).is_err());
    }

    #[test]
    fn invalid_repository_is_rejected_before_github_execution() {
        for repository in [
            "owner/repo/extra",
            "owner/",
            "./repo",
            "owner/.",
            "owner/..",
            "owner/repo..name",
            "owner/repo.git",
            "owner-/repo",
            "owner_name/repo",
        ] {
            assert!(validate_repo(repository).is_err(), "{repository}");
        }
        assert!(validate_repo("owner/repo").is_ok());
        assert!(validate_repo("org-name/repo.name_1").is_ok());
        assert!(matches!(
            scan("owner/repo/extra", 2),
            Tier15Scan::Failed(reason) if reason.contains("OWNER/REPO")
        ));
    }

    #[test]
    fn production_fetch_arguments_are_direct_read_only_rest_api() {
        assert_eq!(
            gh_api_arguments("owner/repo", IssueState::Closed, 3),
            vec![
                "api".to_string(),
                "--method".to_string(),
                "GET".to_string(),
                "repos/owner/repo/issues?state=closed&per_page=100&page=3".to_string(),
                "--jq".to_string(),
                ISSUE_PAGE_PROJECTION.to_string(),
            ]
        );
    }

    #[test]
    fn source_keeps_tier15_authority_at_the_single_direct_get_boundary() {
        let source = include_str!("tier15.rs");
        let production = source
            .split("\n#[cfg(test)]")
            .next()
            .expect("production source before module tests");
        let fetcher = production
            .split("fn fetch_page")
            .nth(1)
            .expect("narrow fetcher exists")
            .split("fn gh_api_arguments")
            .next()
            .expect("fetcher ends before argument builder");

        assert_eq!(production.matches("Command::new").count(), 1);
        assert!(fetcher.contains("Command::new(\"gh\")"));
        assert!(!fetcher.contains("from_utf8_lossy(&output.stdout)"));
        for forbidden in [
            "queue::",
            "claim::",
            "safety::",
            "legacy",
            "Command::new(\"sh\")",
            "Command::new(\"bash\")",
            "Command::new(\"zsh\")",
            "\"POST\"",
            "\"PATCH\"",
            "\"PUT\"",
            "\"DELETE\"",
            "graphql",
            "issue edit",
            "issue comment",
            "label edit",
        ] {
            assert!(
                !production.contains(forbidden),
                "Tier 1.5 adapter retains prohibited authority: {forbidden}"
            );
        }

        let strict_source = include_str!("tier15/strict.rs");
        for forbidden in [
            "Command",
            "std::process",
            "queue::",
            "claim::",
            "safety::",
            "legacy",
            "gh ",
        ] {
            assert!(
                !strict_source.contains(forbidden),
                "Tier 1.5 projection validator retains prohibited authority: {forbidden}"
            );
        }
    }
}
