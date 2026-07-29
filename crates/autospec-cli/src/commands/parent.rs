use std::process::Command;

use autospec_core::state::SpecStateStore;

use super::CommandFailure;
use options::ReconcileOptions;
use support::{
    find_trusted_marked_comment, issue_close, issue_comment, issue_field, issue_reopen,
    ParentStateLock,
};

mod options;
mod support;

const PARENT_MARKER_PREFIX: &str = "<!-- autospec-parent:";
const DECOMPOSITION_MARKER: &str = "<!-- autospec-parent-decomposition:begin -->";
const COMPLETE_MARKER: &str = "<!-- autospec-parent-complete:begin -->";
const EXTENSION_STATE: &str = "append-only-parent-extension";

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [flag] if matches!(flag.as_str(), "--help" | "-h") => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "record" => record(rest),
        [command, rest @ ..] if command == "extend" => extend(rest),
        [command, rest @ ..] if command == "reconcile-child" => reconcile_child(rest),
        [command, rest @ ..] if command == "sweep" => sweep(rest),
        [] => Err(CommandFailure::diagnostic(
            "autospec parent requires a subcommand",
        )),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec parent command: {command}"
        ))),
    }
}

fn extend(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_extend(args)?;
    let _lock = ParentStateLock::acquire(&options.state_root)?;
    let ledger = find_trusted_marked_comment(&options.repo, options.parent, DECOMPOSITION_MARKER)?;
    let current = parse_child_list(options.parent, &ledger)?;
    let quarantined = ledger.contains("quarantined-parent-decomposed");
    let mut store =
        SpecStateStore::load_or_default(&options.state_root).map_err(CommandFailure::diagnostic)?;
    align_local_parent(&mut store, options.parent, &current, quarantined)?;
    let extension = store
        .extend_parent_decomposition(options.parent, options.children.clone())
        .map_err(CommandFailure::diagnostic)?;
    if extension.changed {
        let mut missing_markers = Vec::new();
        for child in &extension.added_children {
            let body = find_trusted_marked_comment(&options.repo, *child, PARENT_MARKER_PREFIX)?;
            match parse_parent_marker(&body) {
                Some(parent) if parent != options.parent => {
                    return Err(CommandFailure::diagnostic(format!(
                        "child issue #{child} is already linked to parent #{parent}"
                    )))
                }
                None => missing_markers.push(*child),
                Some(_) => {}
            }
        }
        let settled =
            find_trusted_marked_comment(&options.repo, options.parent, DECOMPOSITION_MARKER)?;
        if settled != ledger {
            return Err(CommandFailure::diagnostic(format!(
                "parent issue #{} decomposition changed before extension",
                options.parent
            )));
        }
        let parent_closed = issue_field(&options.repo, options.parent, "state")?.trim() == "CLOSED";
        for child in missing_markers {
            post_child_parent_comment(&options.repo, child, options.parent)?;
        }
        if parent_closed {
            issue_reopen(&options.repo, options.parent)?;
        }
        issue_comment(&options.repo, options.parent, &extension.comment_body)?;
    }
    store
        .save(&options.state_root)
        .map_err(CommandFailure::diagnostic)?;
    println!(
        "{{\"changed\":{},\"parent\":{},\"children\":{},\"added\":{}}}",
        extension.changed,
        extension.parent_issue,
        render_numbers(&extension.child_issues),
        render_numbers(&extension.added_children)
    );
    Ok(())
}

fn parse_extend(args: &[String]) -> Result<options::RecordOptions, CommandFailure> {
    if args.iter().any(|argument| argument == "--quarantined") {
        return Err(CommandFailure::diagnostic(
            "unknown autospec parent extend option: --quarantined",
        ));
    }
    options::parse_record(args)
}

fn align_local_parent(
    store: &mut SpecStateStore,
    parent: u64,
    children: &[u64],
    quarantined: bool,
) -> Result<(), CommandFailure> {
    match store.parent_issue_children(parent) {
        None => store
            .record_parent_decomposition(parent, children.to_vec(), quarantined)
            .map(|_| ())
            .map_err(CommandFailure::diagnostic),
        Some(local) if local == children => Ok(()),
        Some(local) if children.starts_with(&local) => store
            .extend_parent_decomposition(parent, children.to_vec())
            .map(|_| ())
            .map_err(CommandFailure::diagnostic),
        Some(_) => store
            .sync_parent_decomposition(parent, children.to_vec(), quarantined)
            .map(|_| ())
            .map_err(CommandFailure::diagnostic),
    }
}

fn record(args: &[String]) -> Result<(), CommandFailure> {
    let options = options::parse_record(args)?;
    let _lock = ParentStateLock::acquire(&options.state_root)?;
    let mut store =
        SpecStateStore::load_or_default(&options.state_root).map_err(CommandFailure::diagnostic)?;
    let update = match store.parent_issue_children(options.parent) {
        Some(children) if children == options.children => store
            .parent_issue_update(options.parent)
            .ok_or_else(|| CommandFailure::diagnostic("tracked parent has no update"))?,
        Some(_) => {
            return Err(CommandFailure::diagnostic(format!(
                "parent issue #{} is already linked to different children",
                options.parent
            )))
        }
        None => store
            .record_parent_decomposition(
                options.parent,
                options.children.clone(),
                options.quarantined,
            )
            .map_err(CommandFailure::diagnostic)?,
    };

    for child in &options.children {
        ensure_child_parent_comment(&options.repo, *child, options.parent)?;
    }
    let existing =
        find_trusted_marked_comment(&options.repo, options.parent, DECOMPOSITION_MARKER)?;
    if existing.is_empty() {
        issue_comment(&options.repo, options.parent, &update.comment_body)?;
    } else if existing != update.comment_body {
        return Err(CommandFailure::diagnostic(format!(
            "trusted parent decomposition comment for #{} does not match the requested children",
            options.parent
        )));
    }
    store
        .save(&options.state_root)
        .map_err(CommandFailure::diagnostic)?;
    println!(
        "{{\"recorded\":true,\"parent\":{},\"children\":{}}}",
        options.parent,
        render_numbers(&options.children)
    );
    Ok(())
}

fn reconcile_child(args: &[String]) -> Result<(), CommandFailure> {
    let options = options::parse_reconcile(args)?;
    let _lock = ParentStateLock::acquire(&options.state_root)?;
    let child_comment =
        find_trusted_marked_comment(&options.repo, options.child, PARENT_MARKER_PREFIX)?;
    let Some(parent) = parse_parent_marker(&child_comment) else {
        println!(
            "{{\"reconciled\":false,\"child\":{},\"reason\":\"no_parent\"}}",
            options.child
        );
        return Ok(());
    };
    reconcile_parent(&options, parent, Some(options.child))
}

fn sweep(args: &[String]) -> Result<(), CommandFailure> {
    let options = options::parse_sweep(args)?;
    let _lock = ParentStateLock::acquire(&options.state_root)?;
    let store =
        SpecStateStore::load_or_default(&options.state_root).map_err(CommandFailure::diagnostic)?;
    let parents = store.parent_issue_numbers();
    let reconcile = ReconcileOptions {
        repo: options.repo,
        child: 0,
        state_root: options.state_root,
    };
    for parent in &parents {
        reconcile_parent(&reconcile, *parent, None)?;
    }
    println!("{{\"swept\":true,\"parents\":{}}}", parents.len());
    Ok(())
}

fn reconcile_parent(
    options: &ReconcileOptions,
    parent: u64,
    triggering_child: Option<u64>,
) -> Result<(), CommandFailure> {
    let reconciliation = read_reconciliation(&options.repo, triggering_child, parent)?;
    update_local_state(options, &reconciliation)?;
    if !reconciliation.is_complete() {
        return finish_pending_parent(options, &reconciliation);
    }

    let settled = read_reconciliation(&options.repo, triggering_child, parent)?;
    let mut store = update_local_state(options, &settled)?;
    if !settled.is_complete() {
        return finish_pending_parent(options, &settled);
    }
    close_complete_parent(options, &settled, &mut store)
}

struct Reconciliation {
    parent: u64,
    children: Vec<u64>,
    terminal: Vec<u64>,
    quarantined: bool,
    requires_merged_pr: bool,
}

impl Reconciliation {
    fn is_complete(&self) -> bool {
        self.terminal.len() == self.children.len()
    }
}

fn read_reconciliation(
    repo: &str,
    triggering_child: Option<u64>,
    parent: u64,
) -> Result<Reconciliation, CommandFailure> {
    let decomposition = find_trusted_marked_comment(repo, parent, DECOMPOSITION_MARKER)?;
    let children = parse_child_list(parent, &decomposition)?;
    if triggering_child.is_some_and(|child| !children.contains(&child)) {
        return Err(CommandFailure::diagnostic(format!(
            "parent issue #{parent} does not list child issue #{}",
            triggering_child.unwrap_or_default()
        )));
    }

    let requires_merged_pr = decomposition.contains(EXTENSION_STATE);
    let mut terminal = Vec::new();
    for child in &children {
        let complete = if requires_merged_pr {
            merged_child(repo, *child)?
        } else {
            issue_field(repo, *child, "state")?.trim() == "CLOSED"
        };
        if complete {
            terminal.push(*child);
        }
    }
    Ok(Reconciliation {
        parent,
        children,
        terminal,
        quarantined: decomposition.contains("quarantined-parent-decomposed"),
        requires_merged_pr,
    })
}

fn print_pending(reconciliation: &Reconciliation) {
    println!(
        "{{\"reconciled\":true,\"parent\":{},\"closed\":false,\"terminal\":{},\"total\":{}}}",
        reconciliation.parent,
        reconciliation.terminal.len(),
        reconciliation.children.len()
    );
}

fn update_local_state(
    options: &ReconcileOptions,
    reconciliation: &Reconciliation,
) -> Result<SpecStateStore, CommandFailure> {
    let mut store =
        SpecStateStore::load_or_default(&options.state_root).map_err(CommandFailure::diagnostic)?;
    ensure_local_parent(
        &mut store,
        reconciliation.parent,
        &reconciliation.children,
        reconciliation.quarantined,
    )?;
    for child in &reconciliation.terminal {
        store
            .record_child_terminal(*child)
            .map_err(CommandFailure::diagnostic)?;
    }
    store
        .save(&options.state_root)
        .map_err(CommandFailure::diagnostic)?;
    Ok(store)
}

fn close_complete_parent(
    options: &ReconcileOptions,
    reconciliation: &Reconciliation,
    store: &mut SpecStateStore,
) -> Result<(), CommandFailure> {
    let parent = reconciliation.parent;
    let summary = store
        .parent_issue_terminal_action(parent)
        .map(|action| action.completion_summary)
        .ok_or_else(|| CommandFailure::diagnostic("complete parent produced no close action"))?;
    let existing = find_trusted_marked_comment(&options.repo, parent, COMPLETE_MARKER)?;
    if existing.is_empty() || (reconciliation.requires_merged_pr && existing != summary) {
        issue_comment(&options.repo, parent, &summary)?;
    } else if existing != summary {
        return Err(CommandFailure::diagnostic(format!(
            "trusted completion comment for parent #{parent} is malformed"
        )));
    }
    if issue_field(&options.repo, parent, "state")?.trim() != "CLOSED" {
        issue_close(&options.repo, parent)?;
    }

    let confirmed = read_reconciliation(&options.repo, None, parent)?;
    *store = update_local_state(options, &confirmed)?;
    if !confirmed.is_complete() {
        return finish_pending_parent(options, &confirmed);
    }
    if !matches!(
        store.parent_issue_status(parent),
        Some(autospec_core::state::ParentIssueStatus::Closed)
    ) {
        store
            .record_parent_closed(parent)
            .map_err(CommandFailure::diagnostic)?;
    }
    store
        .save(&options.state_root)
        .map_err(CommandFailure::diagnostic)?;
    println!("{{\"reconciled\":true,\"parent\":{parent},\"closed\":true}}");
    Ok(())
}

fn finish_pending_parent(
    options: &ReconcileOptions,
    reconciliation: &Reconciliation,
) -> Result<(), CommandFailure> {
    if issue_field(&options.repo, reconciliation.parent, "state")?.trim() == "CLOSED" {
        issue_reopen(&options.repo, reconciliation.parent)?;
    }
    print_pending(reconciliation);
    Ok(())
}

fn ensure_local_parent(
    store: &mut SpecStateStore,
    parent: u64,
    children: &[u64],
    quarantined: bool,
) -> Result<(), CommandFailure> {
    store
        .sync_parent_decomposition(parent, children.to_vec(), quarantined)
        .map(|_| ())
        .map_err(CommandFailure::diagnostic)
}

fn ensure_child_parent_comment(repo: &str, child: u64, parent: u64) -> Result<(), CommandFailure> {
    let body = find_trusted_marked_comment(repo, child, PARENT_MARKER_PREFIX)?;
    if let Some(existing_parent) = parse_parent_marker(&body) {
        return if existing_parent == parent {
            Ok(())
        } else {
            Err(CommandFailure::diagnostic(format!(
                "child issue #{child} is already linked to parent #{existing_parent}"
            )))
        };
    }
    post_child_parent_comment(repo, child, parent)
}

fn post_child_parent_comment(repo: &str, child: u64, parent: u64) -> Result<(), CommandFailure> {
    issue_comment(
        repo,
        child,
        &format!(
            "{PARENT_MARKER_PREFIX}{parent} -->\nChild issue #{child} belongs to parent issue #{parent}."
        ),
    )
}

fn parse_parent_marker(body: &str) -> Option<u64> {
    body.lines().find_map(|line| {
        line.trim()
            .strip_prefix(PARENT_MARKER_PREFIX)
            .and_then(|value| value.strip_suffix(" -->"))
            .and_then(|value| value.parse().ok())
            .filter(|value| *value > 0)
    })
}

fn parse_child_list(parent: u64, comment: &str) -> Result<Vec<u64>, CommandFailure> {
    if !comment.starts_with(DECOMPOSITION_MARKER)
        || !comment.ends_with("<!-- autospec-parent-decomposition:end -->")
        || !comment.contains(&format!("Parent issue #{parent} was decomposed"))
    {
        return Err(CommandFailure::diagnostic(
            "invalid trusted parent decomposition comment",
        ));
    }
    let mut children = Vec::new();
    for line in comment.lines() {
        if let Some(value) = line.trim().strip_prefix("- #") {
            let issue = value.parse::<u64>().map_err(|_| {
                CommandFailure::diagnostic("invalid child issue in decomposition comment")
            })?;
            if issue == 0 || children.contains(&issue) {
                return Err(CommandFailure::diagnostic(
                    "invalid child list in decomposition comment",
                ));
            }
            children.push(issue);
        }
    }
    if children.is_empty() {
        return Err(CommandFailure::diagnostic(
            "parent decomposition comment has no child issues",
        ));
    }
    Ok(children)
}

fn render_numbers(values: &[u64]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn merged_child(repo: &str, issue: u64) -> Result<bool, CommandFailure> {
    let (owner, name) = repo
        .split_once('/')
        .filter(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'))
        .ok_or_else(|| CommandFailure::diagnostic("repository must use OWNER/NAME format"))?;
    let query = "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){issue(number:$number){number state closedByPullRequestsReferences(first:100){nodes{mergedAt}}}}}";
    let output = Command::new("gh")
        .args([
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-F",
            &format!("owner={owner}"),
            "-F",
            &format!("name={name}"),
            "-F",
            &format!("number={issue}"),
        ])
        .output()
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not read merged child evidence: {error}"))
        })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CommandFailure::diagnostic(format!(
            "could not read merged child evidence: {detail}"
        )));
    }
    merged_child_from_graphql(&String::from_utf8_lossy(&output.stdout), issue)
}

fn merged_child_from_graphql(document: &str, expected: u64) -> Result<bool, CommandFailure> {
    let value: serde_json::Value = serde_json::from_str(document).map_err(|error| {
        CommandFailure::diagnostic(format!("invalid merged child evidence: {error}"))
    })?;
    if value.get("errors").is_some() {
        return Err(CommandFailure::diagnostic(
            "merged child evidence contains GraphQL errors",
        ));
    }
    let issue = value
        .pointer("/data/repository/issue")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| CommandFailure::diagnostic("merged child evidence is missing issue data"))?;
    if issue.get("number").and_then(serde_json::Value::as_u64) != Some(expected) {
        return Err(CommandFailure::diagnostic(
            "merged child evidence issue number does not match",
        ));
    }
    let state = issue
        .get("state")
        .and_then(serde_json::Value::as_str)
        .filter(|state| matches!(*state, "OPEN" | "CLOSED"))
        .ok_or_else(|| CommandFailure::diagnostic("merged child evidence has invalid state"))?;
    let nodes = issue
        .get("closedByPullRequestsReferences")
        .and_then(|references| references.get("nodes"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CommandFailure::diagnostic("merged child evidence has invalid PR list"))?;
    let mut merged = false;
    for node in nodes {
        match node.get("mergedAt") {
            Some(serde_json::Value::String(value)) if !value.is_empty() => merged = true,
            Some(serde_json::Value::Null) => {}
            _ => {
                return Err(CommandFailure::diagnostic(
                    "merged child evidence has invalid mergedAt",
                ))
            }
        }
    }
    Ok(state == "CLOSED" && merged)
}

fn print_help() {
    println!(
        "autospec parent\n\nUSAGE:\n    autospec parent record --repo OWNER/REPO --parent N --children N,N [--quarantined] [--state-root PATH]\n    autospec parent extend --repo OWNER/REPO --parent N --children N,N [--state-root PATH]\n    autospec parent reconcile-child --repo OWNER/REPO --child N [--state-root PATH]\n    autospec parent sweep --repo OWNER/REPO [--state-root PATH]"
    );
}

#[cfg(test)]
mod tests {
    use super::{merged_child_from_graphql, parse_extend};

    #[test]
    fn extend_reconciliation_requires_a_merged_closing_pull_request() {
        let manually_closed = r#"{"data":{"repository":{"issue":{"number":13,"state":"CLOSED","closedByPullRequestsReferences":{"nodes":[]}}}}}"#;
        let merged = r#"{"data":{"repository":{"issue":{"number":13,"state":"CLOSED","closedByPullRequestsReferences":{"nodes":[{"mergedAt":"2026-07-29T10:00:00Z"}]}}}}}"#;
        assert!(!merged_child_from_graphql(manually_closed, 13).expect("valid manual close"));
        assert!(merged_child_from_graphql(merged, 13).expect("valid merged close"));
        assert!(merged_child_from_graphql(r#"{"data":{}}"#, 13).is_err());
        let args =
            ["--repo", "o/r", "--parent", "10", "--children", "11,12,13"].map(str::to_string);
        assert_eq!(
            parse_extend(&args).expect("extend options").children,
            [11, 12, 13]
        );
    }
}
