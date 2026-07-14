use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::process::{Command, Output};

use autospec_core::coordination::{
    dependency_numbers, parse_dependency_issue_json, parse_remote_issue_list_json,
    parse_remote_pull_requests_json, plan_ready_queue, PullRequestEvidence, QueueIssueView,
    QueuePolicy, ReadyQueueInput, ReadyQueuePlan, RemoteIssue,
};

use super::claim::{reconcile_active_issue, recover_active_issue};
use super::CommandFailure;

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec queue requires a subcommand",
        )),
        [flag] if matches!(flag.as_str(), "--help" | "-h") => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "ready" => ready(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec queue command: {command}"
        ))),
    }
}

#[derive(Debug, Default)]
struct ReadyOptions {
    repo: Option<String>,
    batch_size: Option<usize>,
}

fn ready(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_ready_options(args)?;
    let repo = options.repo.map_or_else(infer_repo, Ok)?;
    let batch_size = options.batch_size.unwrap_or_else(default_batch_size);
    let candidates = list_issues(&repo, "auto-implement")?;
    let mut active = list_issues(&repo, "in-progress-by-bot")?;
    for issue in &active {
        let _ = reconcile_active_issue(&repo, issue.number);
    }
    for issue in &active {
        let _ = recover_active_issue(&repo, issue.number, 300);
    }
    active = list_issues(&repo, "in-progress-by-bot")?;
    let dependencies = load_dependencies(&repo, &candidates);
    let pull_requests = list_pull_requests(&repo);
    let mut policy = QueuePolicy::new(batch_size, max_repo_workers());
    policy.only_issues = only_issues();
    policy.non_blocking_dependency_labels = non_blocking_dependency_labels();
    let plan = plan_ready_queue(&ReadyQueueInput {
        candidates,
        active,
        dependencies,
        pull_requests,
        policy,
    });
    println!("{}", plan_json(&plan));
    Ok(())
}

fn parse_ready_options(args: &[String]) -> Result<ReadyOptions, CommandFailure> {
    let mut options = ReadyOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => {
                let value = next_value(args, &mut index, "--repo")?;
                if options.repo.replace(value).is_some() {
                    return Err(CommandFailure::diagnostic(
                        "--repo accepts exactly one value",
                    ));
                }
            }
            "--batch-size" => {
                let value = next_value(args, &mut index, "--batch-size")?;
                let batch_size = value
                    .parse::<usize>()
                    .map_err(|_| CommandFailure::diagnostic("--batch-size must be an integer"))?;
                if options.batch_size.replace(batch_size.max(1)).is_some() {
                    return Err(CommandFailure::diagnostic(
                        "--batch-size accepts exactly one value",
                    ));
                }
            }
            "--help" | "-h" => {
                return Err(CommandFailure::diagnostic(
                    "--help cannot be combined with queue ready options",
                ));
            }
            option => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec queue ready option: {option}"
                )));
            }
        }
        index += 1;
    }
    Ok(options)
}

fn next_value(args: &[String], index: &mut usize, option: &str) -> Result<String, CommandFailure> {
    *index += 1;
    let Some(value) = args.get(*index) else {
        return Err(CommandFailure::diagnostic(format!(
            "{option} requires an argument"
        )));
    };
    if value.is_empty() || value.starts_with('-') {
        return Err(CommandFailure::diagnostic(format!(
            "{option} requires an argument"
        )));
    }
    Ok(value.clone())
}

fn infer_repo() -> Result<String, CommandFailure> {
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ])
        .output()
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not run gh repo view: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandFailure::diagnostic(
            "--repo is required when gh cannot infer it",
        ));
    }
    let repo = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if repo.is_empty() {
        Err(CommandFailure::diagnostic(
            "--repo is required when gh cannot infer it",
        ))
    } else {
        Ok(repo)
    }
}

fn list_issues(repo: &str, label: &str) -> Result<Vec<RemoteIssue>, CommandFailure> {
    let output = run_gh(&[
        "issue",
        "list",
        "--repo",
        repo,
        "--state",
        "open",
        "--label",
        label,
        "--limit",
        "200",
        "--json",
        "number,title,body,labels,author",
        "--jq",
        "[.[] | {number, title:(.title // \"\"), body:(.body // \"\"), labels:[.labels[].name], author:{login:(.author.login // \"\")}}]",
    ])?;
    if !output.status.success() {
        return Err(CommandFailure::diagnostic(format!(
            "gh issue list for {label} failed: {}",
            command_error(&output)
        )));
    }
    parse_remote_issue_list_json(&String::from_utf8_lossy(&output.stdout)).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not parse GitHub {label} issue list: {error}"
        ))
    })
}

fn load_dependencies(repo: &str, candidates: &[RemoteIssue]) -> BTreeMap<u64, RemoteIssue> {
    let candidate_numbers = candidates
        .iter()
        .map(|issue| issue.number)
        .collect::<BTreeSet<_>>();
    let dependency_numbers = candidates
        .iter()
        .flat_map(|issue| dependency_numbers(&issue.body))
        .filter(|number| !candidate_numbers.contains(number))
        .collect::<BTreeSet<_>>();
    dependency_numbers
        .into_iter()
        .map(|number| {
            let issue = load_dependency(repo, number).unwrap_or_else(|| {
                RemoteIssue::open(number, format!("issue-{number}"), "", Vec::new(), "")
            });
            (number, issue)
        })
        .collect()
}

fn load_dependency(repo: &str, number: u64) -> Option<RemoteIssue> {
    let output = run_gh(&[
        "issue",
        "view",
        &number.to_string(),
        "--repo",
        repo,
        "--json",
        "state,body,labels",
        "--jq",
        "{state:(.state // \"OPEN\"), body:(.body // \"\"), labels:[.labels[].name]}",
    ])
    .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_dependency_issue_json(&String::from_utf8_lossy(&output.stdout), number).ok()
}

fn list_pull_requests(repo: &str) -> PullRequestEvidence {
    let output = match run_gh(&[
        "pr",
        "list",
        "--repo",
        repo,
        "--state",
        "open",
        "--limit",
        "100",
        "--json",
        "number,state,body,statusCheckRollup",
        "--jq",
        "[.[] | {number, state:(.state // \"OPEN\"), body:(.body // \"\"), statusCheckRollup:[(.statusCheckRollup // [])[] | {name, status, conclusion}]}]",
    ]) {
        Ok(output) => output,
        Err(error) => return PullRequestEvidence::Unavailable(error.message),
    };
    if !output.status.success() {
        return PullRequestEvidence::Unavailable(command_error(&output));
    }
    parse_remote_pull_requests_json(&String::from_utf8_lossy(&output.stdout))
        .map(PullRequestEvidence::Available)
        .unwrap_or_else(PullRequestEvidence::Unavailable)
}

fn run_gh(arguments: &[&str]) -> Result<Output, CommandFailure> {
    Command::new("gh")
        .args(arguments)
        .output()
        .map_err(|error| CommandFailure::diagnostic(format!("could not run gh: {error}")))
}

fn command_error(output: &Output) -> String {
    let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if error.is_empty() {
        format!("gh exited with {}", output.status)
    } else {
        error
    }
}

fn only_issues() -> BTreeSet<u64> {
    std::env::var("AUTOSPEC_RUN_ONLY_ISSUES")
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|value| value.parse::<u64>().ok())
        .collect()
}

fn non_blocking_dependency_labels() -> BTreeSet<String> {
    std::env::var("AUTOSPEC_NON_BLOCKING_DEP_LABELS")
        .unwrap_or_else(|_| "epic umbrella".to_string())
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|label| !label.is_empty())
        .map(|label| label.to_ascii_lowercase())
        .collect()
}

fn default_batch_size() -> usize {
    std::env::var("AUTOSPEC_BATCH_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1)
}

fn max_repo_workers() -> usize {
    let configured = config_scalar("autonomous.concurrency.max_concurrent_repo_workers")
        .or_else(|| std::env::var("AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS").ok());
    match configured.as_deref().map(str::trim) {
        Some("auto") | None => discovered_workers(),
        Some(value) => value.parse::<usize>().unwrap_or(0),
    }
}

fn config_scalar(path: &str) -> Option<String> {
    let config = std::env::var("AUTOSPEC_CONFIG_FILE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ".autospec/autospec.yml".to_string());
    let text = fs::read_to_string(config).ok()?;
    let mut indentation = Vec::new();
    for raw in text.lines() {
        if raw.trim().is_empty() || raw.trim_start().starts_with('#') || !raw.contains(':') {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        let (key, value) = raw.trim().split_once(':')?;
        while indentation
            .last()
            .is_some_and(|(level, _)| *level >= indent)
        {
            indentation.pop();
        }
        if value.trim().is_empty() {
            indentation.push((indent, key.trim().to_string()));
            continue;
        }
        let mut parts = indentation
            .iter()
            .map(|(_, key)| key.as_str())
            .collect::<Vec<_>>();
        parts.push(key.trim());
        if parts.join(".") == path {
            return Some(value.trim().trim_matches(['\'', '\"']).to_string());
        }
    }
    None
}

fn discovered_workers() -> usize {
    for (command, arguments) in [
        ("getconf", &["_NPROCESSORS_ONLN"][..]),
        ("sysctl", &["-n", "hw.ncpu"][..]),
        ("nproc", &[][..]),
    ] {
        let Ok(output) = Command::new(command).args(arguments).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        if let Ok(count) = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<usize>()
        {
            return (count / 4).clamp(1, 4);
        }
    }
    1
}

fn plan_json(plan: &ReadyQueuePlan) -> String {
    format!(
        "{{\"ready\":{},\"blocked\":{},\"claimed\":{},\"conflicts\":{},\"worker_cap\":{{\"max_repo_workers\":{},\"active_count\":{},\"remaining\":{},\"reached\":{}}},\"batch\":{}}}",
        views_json(&plan.ready),
        views_json(&plan.blocked),
        issues_json(&plan.claimed),
        views_json(&plan.conflicts),
        plan.worker_cap.max_repo_workers,
        plan.worker_cap.active_count,
        plan.worker_cap.remaining,
        json_bool(plan.worker_cap.reached),
        views_json(&plan.batch),
    )
}

fn views_json(views: &[QueueIssueView]) -> String {
    format!(
        "[{}]",
        views.iter().map(view_json).collect::<Vec<_>>().join(",")
    )
}

fn issues_json(issues: &[RemoteIssue]) -> String {
    format!(
        "[{}]",
        issues.iter().map(issue_json).collect::<Vec<_>>().join(",")
    )
}

fn view_json(view: &QueueIssueView) -> String {
    let mut fields = issue_fields(&view.issue);
    if let Some(reason) = &view.reason {
        fields.push(json_field("reason", json_string(reason)));
    }
    if let Some(blocked_label) = &view.blocked_label {
        fields.push(json_field("blocked_label", json_string(blocked_label)));
    }
    if let Some(safety_gate) = &view.safety_gate {
        fields.push(json_field(
            "safety_gate",
            format!(
                "{{\"ok\":{},\"reason\":{}}}",
                json_bool(safety_gate.ok),
                json_string(&safety_gate.reason)
            ),
        ));
    }
    if let Some(linked_pr) = view.linked_pr {
        fields.push(json_field("linked_pr", linked_pr.to_string()));
    }
    if !view.unmet_dependencies.is_empty() {
        fields.push(json_field(
            "unmet_dependencies",
            numbers_json(&view.unmet_dependencies),
        ));
        fields.push(json_field(
            "non_blocking_refs",
            references_json(&view.non_blocking_refs),
        ));
    }
    if !view.cycle_dependencies.is_empty() {
        fields.push(json_field(
            "cycle_dependencies",
            numbers_json(&view.cycle_dependencies),
        ));
    }
    if let Some(conflicts_with) = view.conflicts_with {
        fields.push(json_field("conflicts_with", conflicts_with.to_string()));
    }
    if let Some(path) = &view.path {
        fields.push(json_field("path", json_string(path)));
    }
    if view.parallel_safe.is_some() {
        fields.push(json_field("paths", strings_json(&view.paths)));
        fields.push(json_field(
            "non_blocking_refs",
            references_json(&view.non_blocking_refs),
        ));
        fields.push(json_field(
            "serialization_reasons",
            strings_json(&view.serialization_reasons),
        ));
        fields.push(json_field(
            "parallel_safe",
            json_bool(view.parallel_safe == Some(true)).to_string(),
        ));
    }
    format!("{{{}}}", fields.join(","))
}

fn issue_json(issue: &RemoteIssue) -> String {
    format!("{{{}}}", issue_fields(issue).join(","))
}

fn issue_fields(issue: &RemoteIssue) -> Vec<String> {
    vec![
        json_field("number", issue.number.to_string()),
        json_field("title", json_string(&issue.title)),
        json_field("body", json_string(&issue.body)),
        json_field(
            "labels",
            format!(
                "[{}]",
                issue
                    .labels
                    .iter()
                    .map(|label| format!("{{\"name\":{}}}", json_string(label)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        ),
        json_field(
            "author",
            if issue.author.is_empty() {
                "null".to_string()
            } else {
                format!("{{\"login\":{}}}", json_string(&issue.author))
            },
        ),
    ]
}

fn references_json(references: &[autospec_core::coordination::NonBlockingReference]) -> String {
    format!(
        "[{}]",
        references
            .iter()
            .map(|reference| {
                format!(
                    "{{\"issue\":{},\"reason\":{},\"cycle\":{}}}",
                    reference.issue,
                    json_string(&reference.reason),
                    json_bool(reference.cycle)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn numbers_json(numbers: &[u64]) -> String {
    format!(
        "[{}]",
        numbers
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn strings_json(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn json_field(key: &str, value: String) -> String {
    format!("\"{key}\":{value}")
}

fn json_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn json_string(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    format!("\"{escaped}\"")
}

fn print_help() {
    println!(
        "autospec queue\n\nUSAGE:\n    autospec queue ready [--repo OWNER/REPO] [--batch-size N]\n\nCOMMANDS:\n    ready    Compute the safe, dependency-aware GitHub issue batch"
    );
}
