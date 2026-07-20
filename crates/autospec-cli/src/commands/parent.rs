use autospec_core::state::SpecStateStore;

use super::CommandFailure;
use options::ReconcileOptions;
use support::{find_trusted_marked_comment, issue_comment, issue_field, run_gh, ParentStateLock};

mod options;
mod support;

const PARENT_MARKER_PREFIX: &str = "<!-- autospec-parent:";
const DECOMPOSITION_MARKER: &str = "<!-- autospec-parent-decomposition:begin -->";
const COMPLETE_MARKER: &str = "<!-- autospec-parent-complete:begin -->";

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [flag] if matches!(flag.as_str(), "--help" | "-h") => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "record" => record(rest),
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
        print_pending(&reconciliation);
        return Ok(());
    }

    let settled = read_reconciliation(&options.repo, triggering_child, parent)?;
    let mut store = update_local_state(options, &settled)?;
    if !settled.is_complete() {
        print_pending(&settled);
        return Ok(());
    }
    close_complete_parent(options, &settled, &mut store)
}

struct Reconciliation {
    parent: u64,
    children: Vec<u64>,
    terminal: Vec<u64>,
    quarantined: bool,
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

    let mut terminal = Vec::new();
    for child in &children {
        if issue_field(repo, *child, "state")?.trim() == "CLOSED" {
            terminal.push(*child);
        }
    }
    Ok(Reconciliation {
        parent,
        children,
        terminal,
        quarantined: decomposition.contains("quarantined-parent-decomposed"),
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
    if existing.is_empty() {
        issue_comment(&options.repo, parent, &summary)?;
    } else if existing != summary {
        return Err(CommandFailure::diagnostic(format!(
            "trusted completion comment for parent #{parent} is malformed"
        )));
    }
    if issue_field(&options.repo, parent, "state")?.trim() != "CLOSED" {
        run_gh(
            &[
                "issue".into(),
                "close".into(),
                parent.to_string(),
                "--repo".into(),
                options.repo.clone(),
            ],
            "close completed parent issue",
        )?;
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
    let marker = format!("{PARENT_MARKER_PREFIX}{parent} -->");
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
    issue_comment(
        repo,
        child,
        &format!("{marker}\nChild issue #{child} belongs to parent issue #{parent}."),
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

fn print_help() {
    println!(
        "autospec parent\n\nUSAGE:\n    autospec parent record --repo OWNER/REPO --parent N --children N,N [--quarantined] [--state-root PATH]\n    autospec parent reconcile-child --repo OWNER/REPO --child N [--state-root PATH]\n    autospec parent sweep --repo OWNER/REPO [--state-root PATH]"
    );
}
