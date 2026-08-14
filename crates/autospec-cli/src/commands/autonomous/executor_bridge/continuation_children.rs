// executor_bridge/continuation_children.rs — carrying the unmet criteria forward as GitHub
// issues, and binding this run to the one it is executing.
//
// Once continuation.rs has decided a run is continuable and recorded the receipt, the
// remaining criteria have to become work someone can pick up. Each becomes a child issue under
// an umbrella parent — the original issue when this is the first split, or the parent already
// recorded when this run is itself a child — chained so that each depends on the one before,
// which is what stops the fragments from being implemented out of order or in parallel against
// each other.
//
// Everything here is written to survive a restart mid-publication. A child carries a marker
// naming the repository, issue, base, head, receipt digest and ordinal, so republishing finds
// the existing issue instead of creating a second one; every body is validated against the
// issue-quality contract before any of them is filed, so a rejected body cannot leave a
// half-published chain; and each issue is re-read after creation to prove GitHub stored what
// was sent. The authoritative child order lives in a comment on the parent written by a
// trusted actor, and recover_bound_continuation refuses to proceed when the binding persisted
// in the invocation disagrees with it.
//
// require_continuation_checkpoint is the single entry point for both halves and lives here,
// with the half that decides whether to publish. It returns an invariant failure for an
// oversized checkpoint: the receipt and the children are durable at that point, so the run
// must stop before it mutates anything remote.
//
// Split out of executor_bridge.rs, which is over the size limit and so may not grow.

use super::*;

fn continuation_gh(arguments: &[String], action: &str) -> Result<String, String> {
    let output = Command::new("gh")
        .args(arguments)
        .output()
        .map_err(|error| format!("{action}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
fn trusted_continuation_comment(
    repository: &str,
    issue: u64,
    marker: &str,
) -> Result<String, String> {
    let actors =
        crate::commands::lint::configured_safety_trusted_actors().map_err(|error| error.message)?;
    let predicates = actors
        .iter()
        .map(|actor| format!(".author.login == {actor:?}"))
        .collect::<Vec<_>>()
        .join(" or ");
    let marker = serde_json::to_string(marker).map_err(|error| error.to_string())?;
    continuation_gh(
        &[
            "issue".into(),
            "view".into(),
            issue.to_string(),
            "--repo".into(),
            repository.into(),
            "--json".into(),
            "comments".into(),
            "--jq".into(),
            format!(
                "[.comments[] | select(({}) and (.body | contains({marker})))] | last | .body // \"\"",
                predicates
            ),
        ],
        "read continuation parent metadata",
    )
}
pub(super) fn continuation_parent(repository: &str, issue: u64) -> Result<Option<u64>, String> {
    let body = trusted_continuation_comment(repository, issue, "<!-- autospec-parent:")?;
    body.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("<!-- autospec-parent:")
                .and_then(|value| value.strip_suffix(" -->"))
                .map(str::parse::<u64>)
        })
        .transpose()
        .map_err(|_| "continuation parent marker is malformed".to_string())
}
pub(super) fn continuation_child_list(repository: &str, parent: u64) -> Result<Vec<u64>, String> {
    let body = trusted_continuation_comment(
        repository,
        parent,
        "<!-- autospec-parent-decomposition:begin -->",
    )?;
    let children = body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- #"))
        .map(|value| value.parse::<u64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "continuation parent child order is malformed".to_string())?;
    if children.is_empty() {
        return Err("continuation parent has no authoritative child order".to_string());
    }
    Ok(children)
}
fn continuation_child_document(
    receipt: &ContinuationReceipt,
    umbrella: u64,
    ordinal: usize,
    criterion: &str,
    dependency: Option<u64>,
) -> Result<(String, String, String), String> {
    let criterion = criterion.trim();
    if criterion.len() > 120 || criterion.lines().count() != 1 {
        return Err("continuation criterion must be one line of at most 120 bytes".to_string());
    }
    let marker = format!(
        "<!-- autospec-continuation:v1:repo={}:issue={}:base={}:head={}:receipt={}:ordinal={} -->",
        receipt.repository,
        receipt.issue,
        receipt.base_oid,
        receipt.head_oid,
        receipt.content_digest,
        ordinal
    );
    let mut body = format!(
        "{marker}\n\n## Goal\n\n{}.\n\n## Acceptance criteria\n\n- [ ] {}\n",
        criterion.trim_end_matches(['.', '?', '!']),
        criterion
    );
    if let Some(previous) = dependency {
        body.push_str(&format!(
            "\n## Dependencies\n\nDepends on issue #{previous}\n"
        ));
    }
    let issue = receipt.issue;
    body.push_str(&format!("\n## Context\n\nPart of #{umbrella}.\n\n## Files to read first\n\n- `AGENTS.md`\n\n## Implementation outline\n\n- Implement continuation ordinal {ordinal} within the original issue #{issue} scope.\n\n## Tests required\n\n- smoke: verify continuation ordinal {ordinal}.\n\n### Primary smoke test (inner loop)\n\n```bash\ngit diff --check\n```\n"));
    if !autospec_core::lint::lint_issue_body(&body).is_empty() {
        return Err("continuation issue body failed the issue-quality contract".to_string());
    }
    let mut title = format!("Continuation {ordinal}: {criterion}");
    while title.len() > 120 {
        title.pop();
    }
    Ok((marker, title, body))
}
fn publish_continuation_child(
    receipt: &ContinuationReceipt,
    umbrella: u64,
    ordinal: usize,
    criterion: &str,
    dependency: Option<u64>,
) -> Result<u64, String> {
    let (marker, title, body) =
        continuation_child_document(receipt, umbrella, ordinal, criterion, dependency)?;
    let mut matches = continuation_gh(
        &[
            "issue".into(),
            "list".into(),
            "--repo".into(),
            receipt.repository.clone(),
            "--state".into(),
            "all".into(),
            "--search".into(),
            format!("{marker} in:body"),
            "--limit".into(),
            "2".into(),
            "--json".into(),
            "number,body".into(),
            "--jq".into(),
            format!(".[] | select(.body | contains({marker:?})) | .number"),
        ],
        "find continuation child",
    )?;
    if matches.is_empty() {
        let mut arguments = vec![
            "issue".into(),
            "create".into(),
            "--repo".into(),
            receipt.repository.clone(),
            "--title".into(),
            title,
            "--body".into(),
            body.clone(),
        ];
        if dependency.is_some() || receipt.status == "oversized_checkpoint" {
            arguments.extend(["--label".into(), "auto-implement".into()]);
        }
        matches = continuation_gh(&arguments, "create continuation child")?
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();
    }
    let numbers = matches
        .lines()
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "continuation child identity is malformed".to_string())?;
    let [number] = numbers.as_slice() else {
        return Err("continuation child marker is missing or ambiguous".to_string());
    };
    let observed = continuation_gh(
        &[
            "issue".into(),
            "view".into(),
            number.to_string(),
            "--repo".into(),
            receipt.repository.clone(),
            "--json".into(),
            "body".into(),
            "--jq".into(),
            ".body".into(),
        ],
        "reread continuation child",
    )?;
    if observed != body.trim() {
        return Err("continuation child body changed after publication".to_string());
    }
    Ok(*number)
}
pub(super) fn publish_continuation_children(
    state_path: &Path,
    receipt: &ContinuationReceipt,
) -> Result<(u64, u64), String> {
    let parent = continuation_parent(&receipt.repository, receipt.issue)?;
    let (umbrella, mut children, start, mut previous) = match parent {
        Some(parent) => {
            let children = continuation_child_list(&receipt.repository, parent)?;
            if !children.contains(&receipt.issue) {
                return Err("continuation parent does not contain current child".to_string());
            }
            let start = children.len() + 1;
            let previous = Some(receipt.issue);
            (parent, children, start, previous)
        }
        None => (receipt.issue, Vec::new(), 1, None),
    };
    let mut criteria = receipt.unmet.clone();
    if parent.is_none() {
        let current = if receipt.status == "oversized_checkpoint" {
            "reapply the current oversized slice ordinal 1"
        } else {
            "complete the current capped slice ordinal 1"
        };
        criteria.insert(0, current.to_string());
    }
    for (offset, criterion) in criteria.iter().enumerate() {
        let dependency = (parent.is_some() || offset > 0).then_some(receipt.issue);
        continuation_child_document(receipt, umbrella, start + offset, criterion, dependency)?;
    }
    for (offset, criterion) in criteria.iter().enumerate() {
        let child =
            publish_continuation_child(receipt, umbrella, start + offset, criterion, previous)?;
        children.push(child);
        previous = Some(child);
    }
    let state_root = state_path
        .parent()
        .ok_or_else(|| "continuation state has no parent".to_string())?;
    let parent_arguments = |command: &str, selected: &[u64]| {
        vec![
            command.into(),
            "--repo".into(),
            receipt.repository.clone(),
            "--parent".into(),
            umbrella.to_string(),
            "--children".into(),
            selected
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(","),
            "--state-root".into(),
            state_root.to_string_lossy().into_owned(),
        ]
    };
    if parent.is_none()
        && trusted_continuation_comment(
            &receipt.repository,
            umbrella,
            "<!-- autospec-parent-decomposition:begin -->",
        )?
        .is_empty()
    {
        crate::commands::parent::run(&parent_arguments("record", &children[..1]))
            .map_err(|error| error.message)?;
    }
    crate::commands::parent::run(&parent_arguments("extend", &children))
        .map_err(|error| error.message)?;
    Ok((umbrella, parent.map_or(children[0], |_| receipt.issue)))
}

pub(super) fn recover_bound_continuation(
    state: &PersistedInvocation,
) -> Result<Option<(u64, u64)>, String> {
    let (Some(umbrella), Some(current_child)) = (state.umbrella, state.current_child) else {
        return Ok(None);
    };
    let children = continuation_child_list(&state.identity.repository, umbrella)?;
    let expected_child = if umbrella == state.identity.issue {
        children.first().copied()
    } else if children.contains(&state.identity.issue) {
        Some(state.identity.issue)
    } else {
        None
    };
    if expected_child != Some(current_child) {
        return Err(
            "executor continuation binding does not match the authoritative child order"
                .to_string(),
        );
    }
    Ok(Some((umbrella, current_child)))
}

pub(super) fn bind_continuation_part(
    state_path: &Path,
    state: &mut PersistedInvocation,
    binding: (u64, u64),
) -> Result<(), String> {
    if binding.0 == 0 || binding.1 == 0 || binding.0 == binding.1 {
        return Err("executor continuation part binding is invalid".to_string());
    }
    match (state.umbrella, state.current_child) {
        (None, None) => {
            state.umbrella = Some(binding.0);
            state.current_child = Some(binding.1);
            write_invocation_atomic(state_path, state)
        }
        (Some(umbrella), Some(child)) if (umbrella, child) == binding => Ok(()),
        _ => Err("executor continuation part binding changed".to_string()),
    }
}

pub(super) fn require_continuation_checkpoint(
    state_path: &Path,
    event_log: &Path,
    state: &PersistedInvocation,
    proof: &ImplementationProof,
    issue_body: &str,
    publish_children: bool,
) -> Result<(), BridgeRunFailure> {
    let Some(checkpoint) = prepare_continuation_checkpoint(state_path, state, proof, issue_body)?
    else {
        return Ok(());
    };
    emit_continuation_events(state_path, event_log, state, &checkpoint)?;
    if publish_children {
        let binding = match recover_bound_continuation(state)? {
            Some(binding) => binding,
            None => publish_continuation_children(state_path, &checkpoint.receipt)?,
        };
        let mut bound = state.clone();
        bind_continuation_part(state_path, &mut bound, binding)?;
    }
    match checkpoint.receipt.status.as_str() {
        "oversized_checkpoint" => Err(BridgeRunFailure::invariant(
            "executor preserved an oversized continuation checkpoint before remote mutation",
        )),
        _ => Ok(()),
    }
}
