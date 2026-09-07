use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::coordination::{parse_dependency_issue_json, RemoteIssue};
use autospec_core::spec_repair::{
    append_repair_event, check_spec_repair, classify_ambiguity_stakes, load_repair_events,
    proposal_count, propose_spec_repair, render_pr_assumption_section, should_trigger_spec_review,
    summarize_by_origin_template, summarize_by_shape, AmbiguityStakes, CheckOutcome,
    IssueCommentSnapshot, IssueRepairTracker, ProposeOutcome, SpecRepairEvent,
    SpecRepairProposalInput, StatedAssumption, ASSUMPTION_HEADING, NEEDS_SPEC_CLARIFICATION_LABEL,
    SPEC_REVIEW_STALL_THRESHOLD,
};

use super::CommandFailure;

const ISSUE_FIELDS: &str = "{number, title:(.title // \"\"), body:(.body // \"\"), labels:[.labels[].name], author:{login:(.user.login // \"\")}, state:(.state // \"OPEN\")}";
const COMMENTS_FIELDS: &str = "[.[] | {author:(.user.login // \"\"), body:(.body // \"\")}]";

/// The GitHub adapter for the repair loop. It implements exactly the surface the
/// core trait exposes — read, comment, label — and nothing else, so a body write is
/// not even reachable from repair code.
struct GhIssueRepairTracker;

fn run_gh(args: &[&str]) -> Result<Output, io::Error> {
    Command::new("gh")
        .args(args)
        .output()
        .map_err(|error| io::Error::other(format!("could not execute gh: {error}")))
}

fn require_gh_success(output: &Output, operation: &str) -> Result<(), io::Error> {
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "{operation} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim_end()
    )))
}

impl IssueRepairTracker for GhIssueRepairTracker {
    fn read_issue(&self, repo: &str, number: u64) -> io::Result<RemoteIssue> {
        let endpoint = format!("repos/{repo}/issues/{number}");
        let output = run_gh(&["api", "--method", "GET", &endpoint, "--jq", ISSUE_FIELDS])?;
        require_gh_success(&output, "gh issue read")?;
        parse_dependency_issue_json(&String::from_utf8_lossy(&output.stdout), number).map_err(
            |error| io::Error::other(format!("could not parse GitHub issue {number}: {error}")),
        )
    }

    fn post_comment(&self, repo: &str, number: u64, body: &str) -> io::Result<()> {
        let endpoint = format!("repos/{repo}/issues/{number}/comments");
        let field = format!("body={body}");
        let output = run_gh(&["api", "--method", "POST", &endpoint, "-f", &field])?;
        require_gh_success(&output, "gh issue comment write")
    }

    fn add_label(&self, repo: &str, number: u64, label: &str) -> io::Result<()> {
        let endpoint = format!("repos/{repo}/issues/{number}/labels");
        let field = format!("labels[]={label}");
        let output = run_gh(&["api", "--method", "POST", &endpoint, "-f", &field])?;
        require_gh_success(&output, "gh issue label write")
    }

    fn remove_label(&self, repo: &str, number: u64, label: &str) -> io::Result<()> {
        let endpoint = format!("repos/{repo}/issues/{number}/labels/{label}");
        let output = run_gh(&["api", "--method", "DELETE", &endpoint])?;
        require_gh_success(&output, "gh issue label removal")
    }

    fn list_comments(&self, repo: &str, number: u64) -> io::Result<Vec<IssueCommentSnapshot>> {
        let endpoint = format!("repos/{repo}/issues/{number}/comments");
        let output = run_gh(&["api", "--method", "GET", &endpoint, "--jq", COMMENTS_FIELDS])?;
        require_gh_success(&output, "gh issue comment list")?;
        let snapshots: Vec<IssueCommentSnapshot> =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                io::Error::other(format!("could not parse issue comments: {error}"))
            })?;
        Ok(snapshots)
    }
}

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec spec-repair requires a subcommand (propose|check|assumption|stall|ledger)",
        )),
        [flag] if flag == "--help" || flag == "-h" => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "propose" => run_propose(rest),
        [command, rest @ ..] if command == "check" => run_check(rest),
        [command, rest @ ..] if command == "assumption" => run_assumption(rest),
        [command, rest @ ..] if command == "stall" => run_stall(rest),
        [command, rest @ ..] if command == "ledger" => run_ledger(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec spec-repair command: {command}"
        ))),
    }
}

/// Parsed command-line flags: repeated value flags accumulate, boolean flags are
/// recorded without values.
struct Flags {
    values: BTreeMap<&'static str, Vec<String>>,
    booleans: Vec<String>,
}

fn parse_flags(
    args: &[String],
    value_flags: &[&'static str],
    boolean_flags: &[&'static str],
) -> Result<Flags, CommandFailure> {
    let mut values: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut booleans = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].clone();
        if let Some(flag) = value_flags.iter().copied().find(|flag| *flag == arg) {
            let Some(value) = args.get(index + 1) else {
                return Err(CommandFailure::diagnostic(format!(
                    "{flag} requires a value"
                )));
            };
            values.entry(flag).or_default().push(value.clone());
            index += 2;
        } else if boolean_flags.contains(&arg.as_str()) {
            booleans.push(arg);
            index += 1;
        } else {
            return Err(CommandFailure::diagnostic(format!("unknown option: {arg}")));
        }
    }
    Ok(Flags { values, booleans })
}

impl Flags {
    fn value(&self, flag: &str) -> Option<&str> {
        self.values
            .get(flag)
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    fn all(&self, flag: &str) -> Vec<String> {
        self.values.get(flag).cloned().unwrap_or_default()
    }

    fn has(&self, flag: &str) -> bool {
        self.booleans.iter().any(|present| present == flag)
    }

    fn require(&self, flag: &str) -> Result<String, CommandFailure> {
        self.value(flag)
            .map(str::to_string)
            .ok_or_else(|| CommandFailure::diagnostic(format!("{flag} is required")))
    }
}

fn parse_issue_number(value: &str) -> Result<u64, CommandFailure> {
    value
        .parse::<u64>()
        .map_err(|_| CommandFailure::diagnostic(format!("--issue must be a number: {value}")))
}

fn parse_runs(value: &str) -> Result<u32, CommandFailure> {
    value.parse::<u32>().map_err(|_| {
        CommandFailure::diagnostic(format!("--consecutive-runs must be a number: {value}"))
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn read_text_file(path: &PathBuf, what: &str) -> Result<String, CommandFailure> {
    fs::read_to_string(path).map_err(|error| {
        CommandFailure::diagnostic(format!("could not read {what} {}: {error}", path.display()))
    })
}

fn build_repair_event(
    tracker: &GhIssueRepairTracker,
    repo: &str,
    issue_number: u64,
    shape_id: &str,
    flags: &Flags,
) -> Result<SpecRepairEvent, CommandFailure> {
    let origin_author = match flags.value("--origin-author") {
        Some(author) => author.to_string(),
        None => {
            tracker
                .read_issue(repo, issue_number)
                .map_err(|error| {
                    CommandFailure::diagnostic(format!("could not read issue author: {error}"))
                })?
                .author
        }
    };
    // An absent origin is recorded as "(unknown)", matching the ledger summaries,
    // so the queryable ledger never carries an ambiguous empty string.
    Ok(SpecRepairEvent {
        recorded_at: unix_now(),
        repo: repo.to_string(),
        issue: issue_number,
        shape: shape_id.to_string(),
        origin_template: flags
            .value("--origin-template")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("(unknown)")
            .to_string(),
        origin_command: flags
            .value("--origin-command")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("(unknown)")
            .to_string(),
        origin_author,
    })
}

fn run_propose(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_propose_help();
        return Ok(());
    }
    let flags = parse_flags(
        args,
        &[
            "--repo",
            "--issue",
            "--proposal-file",
            "--ledger-file",
            "--origin-template",
            "--origin-command",
            "--origin-author",
        ],
        &["--judged-unusable"],
    )?;
    let repo = flags.require("--repo")?;
    let issue_number = parse_issue_number(&flags.require("--issue")?)?;
    let proposal_path = PathBuf::from(flags.require("--proposal-file")?);
    let ledger_path = PathBuf::from(
        flags
            .value("--ledger-file")
            .unwrap_or(".autospec/spec-repair-ledger.jsonl"),
    );
    let judged_unusable = flags.has("--judged-unusable");

    let proposal_source = read_text_file(&proposal_path, "proposal file")?;
    let input: SpecRepairProposalInput =
        serde_json::from_str(&proposal_source).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not parse proposal JSON {}: {error}",
                proposal_path.display()
            ))
        })?;
    let shape_id = input.shape.id().to_string();

    let tracker = GhIssueRepairTracker;
    let events = load_repair_events(&ledger_path).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not load repair ledger {}: {error}",
            ledger_path.display()
        ))
    })?;
    let prior = proposal_count(&events, &repo, issue_number);

    let outcome = propose_spec_repair(&tracker, &repo, issue_number, input, prior, judged_unusable)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;

    let mut report = serde_json::json!({
        "repo": repo,
        "issue": issue_number,
        "ledger_file": ledger_path.display().to_string(),
    });
    match &outcome {
        ProposeOutcome::Posted { comment } => {
            if let Some(parent) = ledger_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let event = build_repair_event(&tracker, &repo, issue_number, &shape_id, &flags)?;
            append_repair_event(&ledger_path, &event).map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "could not append repair ledger {}: {error}",
                    ledger_path.display()
                ))
            })?;
            report["outcome"] = serde_json::json!("posted");
            report["label"] = serde_json::json!(NEEDS_SPEC_CLARIFICATION_LABEL);
            report["recorded_event"] = serde_json::json!(true);
            report["proposal_count"] = serde_json::json!(prior + 1);
            report["comment"] = serde_json::json!(comment);
        }
        ProposeOutcome::AlreadyProposed => {
            report["outcome"] = serde_json::json!("already-proposed");
            report["label"] = serde_json::json!(NEEDS_SPEC_CLARIFICATION_LABEL);
            report["proposal_count"] = serde_json::json!(prior);
        }
        ProposeOutcome::EscalatedToHuman => {
            report["outcome"] = serde_json::json!("escalated-to-human");
            report["label"] = serde_json::json!(NEEDS_SPEC_CLARIFICATION_LABEL);
            report["proposal_count"] = serde_json::json!(prior);
        }
    }
    println!("{report}");
    Ok(())
}

fn run_check(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_check_help();
        return Ok(());
    }
    let flags = parse_flags(args, &["--repo", "--issue"], &[])?;
    let repo = flags.require("--repo")?;
    let issue_number = parse_issue_number(&flags.require("--issue")?)?;
    let outcome = check_spec_repair(&GhIssueRepairTracker, &repo, issue_number)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let (outcome_id, label_removed, redispatch) = match &outcome {
        CheckOutcome::NoProposal => ("no-proposal", false, "blocked"),
        CheckOutcome::AwaitingMaintainer => ("awaiting-maintainer", false, "blocked"),
        CheckOutcome::Approved { label_removed } => ("approved", *label_removed, "allowed"),
        CheckOutcome::Rejected => ("rejected", false, "blocked"),
    };
    let report = serde_json::json!({
        "repo": repo,
        "issue": issue_number,
        "outcome": outcome_id,
        "label_removed": label_removed,
        "redispatch": redispatch,
    });
    println!("{report}");
    Ok(())
}

fn run_assumption(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_assumption_help();
        return Ok(());
    }
    let flags = parse_flags(
        args,
        &[
            "--statement",
            "--statement-file",
            "--context-file",
            "--alternative",
            "--rollback",
            "--pr-body-file",
        ],
        &[],
    )?;
    let statement = match flags.value("--statement") {
        Some(statement) => statement.to_string(),
        None => match flags.value("--statement-file") {
            Some(path) => read_text_file(&PathBuf::from(path), "statement file")?,
            None => {
                return Err(CommandFailure::diagnostic(
                    "assumption requires --statement or --statement-file",
                ))
            }
        },
    };
    let context = match flags.value("--context-file") {
        Some(path) => read_text_file(&PathBuf::from(path), "context file")?,
        None => String::new(),
    };
    let stakes = classify_ambiguity_stakes(&format!("{statement}\n{context}"));
    let assumption = StatedAssumption {
        statement,
        alternatives_considered: flags.all("--alternative"),
        rollback: flags.value("--rollback").unwrap_or_default().to_string(),
    };
    // A high-stakes ambiguity cannot ride on an assumption: this exits 2 with the
    // blocking message and leaves the PR body untouched. The JSON report still goes
    // to stdout so a caller can consume the decision mechanically.
    let pr_body_path = PathBuf::from(flags.require("--pr-body-file")?);
    if stakes == AmbiguityStakes::High {
        let reason =
            render_pr_assumption_section(&assumption, stakes).expect_err("high stakes must refuse");
        println!(
            "{}",
            serde_json::json!({
                "stakes": "high",
                "blocked": true,
                "recorded": false,
                "already_present": false,
                "pr_body_file": pr_body_path.display().to_string(),
                "reason": reason,
            })
        );
        return Err(CommandFailure::diagnostic(reason));
    }
    let section =
        render_pr_assumption_section(&assumption, stakes).expect("low stakes renders a section");
    let existing = fs::read_to_string(&pr_body_path).unwrap_or_default();
    let already_present = existing.contains(ASSUMPTION_HEADING);
    if !already_present {
        let mut updated = existing;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push('\n');
        updated.push_str(&section);
        fs::write(&pr_body_path, updated).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not write PR body {}: {error}",
                pr_body_path.display()
            ))
        })?;
    }
    let report = serde_json::json!({
        "stakes": stakes.id(),
        "blocked": false,
        "recorded": !already_present,
        "already_present": already_present,
        "pr_body_file": pr_body_path.display().to_string(),
        "section": section,
    });
    println!("{report}");
    Ok(())
}

fn run_stall(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_stall_help();
        return Ok(());
    }
    let flags = parse_flags(args, &["--consecutive-runs", "--issue"], &[])?;
    let runs = parse_runs(&flags.require("--consecutive-runs")?)?;
    let required = should_trigger_spec_review(runs);
    let mut report = serde_json::json!({
        "consecutive_no_output_runs": runs,
        "threshold": SPEC_REVIEW_STALL_THRESHOLD,
        "spec_review_required": required,
    });
    if let Some(issue) = flags.value("--issue") {
        report["issue"] = serde_json::json!(parse_issue_number(issue)?);
    }
    println!("{report}");
    Ok(())
}

fn run_ledger(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_ledger_help();
        return Ok(());
    }
    let flags = parse_flags(args, &["--file", "--issue", "--shape", "--template"], &[])?;
    let ledger_path = PathBuf::from(flags.require("--file")?);
    let events = load_repair_events(&ledger_path).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not load repair ledger {}: {error}",
            ledger_path.display()
        ))
    })?;
    let issue_filter = match flags.value("--issue") {
        Some(value) => Some(parse_issue_number(value)?),
        None => None,
    };
    let shape_filter = flags.value("--shape");
    let template_filter = flags.value("--template");
    let filtered: Vec<SpecRepairEvent> = events
        .iter()
        .filter(|event| {
            issue_filter.is_none_or(|number| event.issue == number)
                && shape_filter.is_none_or(|shape| event.shape == shape)
                && template_filter.is_none_or(|template| event.origin_template == template)
        })
        .cloned()
        .collect();
    let report = serde_json::json!({
        "ledger_file": ledger_path.display().to_string(),
        "total": filtered.len(),
        "events": filtered,
        "by_shape": summarize_by_shape(&filtered),
        "by_origin_template": summarize_by_origin_template(&filtered),
    });
    println!("{report}");
    Ok(())
}

fn print_help() {
    println!(
        "autospec spec-repair\n\n\
         Repair loop for issues the pipeline judged unusable: propose a specification\n\
         fix as a comment, never by rewriting the issue body.\n\n\
         COMMANDS:\n    \
         propose     Post the five-part repair proposal and label the issue\n    \
         check       Classify the maintainer reply to a posted proposal\n    \
         assumption  Record a low-stakes assumption in a PR body (high-stakes blocks)\n    \
         stall       Decide whether repeated no-output runs must trigger spec review\n    \
         ledger      Query the JSONL ledger of recorded defect classifications\n\n\
         Run `autospec spec-repair <command> --help` for per-command options."
    );
}

fn print_propose_help() {
    println!(
        "autospec spec-repair propose\n\n\
         Post one five-part repair proposal as an issue comment (defect, reading,\n\
         falsifiable criteria with check commands, one question, what was checked)\n\
         and label the issue `needs-spec-clarification`.\n\n\
         OPTIONS:\n    \
         --repo <OWNER/REPO>      Target repository\n    \
         --issue <NUMBER>         Target issue\n    \
         --proposal-file <PATH>   Proposal JSON (shape, reading, criteria, question, checked)\n    \
         --ledger-file <PATH>     JSONL repair ledger (default .autospec/spec-repair-ledger.jsonl)\n    \
         --origin-template <T>    Issue template the issue originated from (ledger provenance)\n    \
         --origin-command <C>     Command that created the issue (ledger provenance)\n    \
         --origin-author <A>      Issue author (default: read from the issue)\n    \
         --judged-unusable        The caller judged the issue unusable beyond the\n    \
                                  mechanical all-criteria-true state\n\n\
         Two proposals already on the ledger for the issue escalate to a human\n\
         instead of posting a third."
    );
}

fn print_check_help() {
    println!(
        "autospec spec-repair check\n\n\
         Classify the maintainer reply to a posted repair proposal. On approval the\n\
         `needs-spec-clarification` label is removed; on rejection it stays on and\n\
         a human rewrites the issue. Exit status is 0 for every classified outcome.\n\n\
         OPTIONS:\n    \
         --repo <OWNER/REPO>      Target repository\n    \
         --issue <NUMBER>         Target issue"
    );
}

fn print_assumption_help() {
    println!(
        "autospec spec-repair assumption\n\n\
         Record a LOW-stakes assumption under `## Stated assumption (reviewable)` in\n\
         a PR body. HIGH-stakes ambiguities (security, migration, public interface,\n\
         destructive action) block with exit 2 instead.\n\n\
         OPTIONS:\n    \
         --statement <TEXT>       The assumption (or --statement-file <PATH>)\n    \
         --context-file <PATH>    Extra context joined into the stakes classification\n    \
         --alternative <TEXT>     Alternative reading considered (repeatable)\n    \
         --rollback <TEXT>        What to do if the assumption is wrong\n    \
         --pr-body-file <PATH>    PR body to append the section to"
    );
}

fn print_stall_help() {
    println!(
        "autospec spec-repair stall\n\n\
         Decide whether repeated no-output runs on one issue must trigger spec\n\
         review. Threshold: 2 consecutive no-output runs.\n\n\
         OPTIONS:\n    \
         --consecutive-runs <N>   Count of consecutive no-output runs\n    \
         --issue <NUMBER>         Optional issue for the report"
    );
}

fn print_ledger_help() {
    println!(
        "autospec spec-repair ledger\n\n\
         Query the JSONL ledger of defect classifications. Summaries are computed\n\
         over the filtered events.\n\n\
         OPTIONS:\n    \
         --file <PATH>            JSONL repair ledger\n    \
         --issue <NUMBER>         Only events for this issue\n    \
         --shape <ID>             Only events with this defect shape\n    \
         --template <TEMPLATE>    Only events with this origin template"
    );
}
