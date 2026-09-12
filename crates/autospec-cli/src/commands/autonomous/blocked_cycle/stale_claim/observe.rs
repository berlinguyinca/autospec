//! Remote observation for a stalled claim: what the branch looks like on
//! GitHub, and what that means for the durable claim. Pure decoding plus one
//! read-only `gh` call; nothing here mutates a claim, PR, or branch.

use super::StaleClaimFacts;
use crate::commands::autonomous::gh_read::run_gh_read_with_retry;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ObservedPullRequest {
    pub(super) number: u64,
    pub(super) state: String,
    pub(super) head_oid: String,
}

pub(super) fn parse_pull_requests(document: &str) -> Result<Vec<ObservedPullRequest>, String> {
    let value: serde_json::Value = serde_json::from_str(document)
        .map_err(|error| format!("parse branch pull request list: {error}"))?;
    let rows = value
        .as_array()
        .ok_or("branch pull request list is not an array")?;
    let mut observed = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(number) = row.get("number").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        observed.push(ObservedPullRequest {
            number,
            state: row
                .get("state")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_ascii_uppercase(),
            head_oid: row
                .get("headRefOid")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase(),
        });
    }
    Ok(observed)
}

pub(super) fn observe_branch_pull_requests(
    repository: &str,
    branch: &str,
) -> Result<Vec<ObservedPullRequest>, String> {
    let output = run_gh_read_with_retry(
        &[
            "pr",
            "list",
            "--repo",
            repository,
            "--head",
            branch,
            "--state",
            "all",
            "--limit",
            "100",
            "--json",
            "number,state,headRefOid",
        ],
        "list stale claim pull requests",
    )
    .map_err(|failure| failure.message)?;
    if !output.status.success() {
        return Err(format!(
            "list stale claim pull requests failed with exit {}",
            output.status.code().unwrap_or(-1)
        ));
    }
    parse_pull_requests(&String::from_utf8_lossy(&output.stdout))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn derive_facts(
    recorded_pr: Option<u64>,
    recorded_head_oid: Option<&str>,
    live_descendants: Option<u32>,
    local_identity_intact: bool,
    progress_at: u64,
    observed_at: u64,
    remote: Result<Vec<ObservedPullRequest>, String>,
) -> StaleClaimFacts {
    let mut facts = StaleClaimFacts {
        recorded_pr,
        pr_state_observed: false,
        pr_open: false,
        pr_merged: false,
        remote_branch_present: false,
        // Without a recorded head there is no provenance to diverge from.
        head_oid_matches: true,
        local_identity_intact,
        live_descendants,
        progress_at,
        observed_at,
    };
    let Ok(rows) = remote else {
        return facts;
    };
    facts.pr_state_observed = true;
    facts.remote_branch_present = !rows.is_empty();
    let selected = rows
        .iter()
        .find(|row| Some(row.number) == recorded_pr)
        .or_else(|| rows.iter().find(|row| row.state == "OPEN"))
        .or_else(|| rows.first());
    let Some(row) = selected else {
        return facts;
    };
    facts.pr_open = row.state == "OPEN";
    facts.pr_merged = row.state == "MERGED";
    if let Some(recorded) = recorded_head_oid {
        facts.head_oid_matches = recorded.eq_ignore_ascii_case(&row.head_oid);
    }
    facts
}
