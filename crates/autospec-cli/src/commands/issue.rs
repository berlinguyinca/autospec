use autospec_core::claim::{
    replace_safety_review_section, review_issue_safety_with_trusted_actors, ClaimSafetyInput,
    SafetyReviewDecision,
};
use autospec_core::coordination::RemoteIssue;
use autospec_core::safety::{
    evaluate_issue_promotion_with_trusted_actors, IssuePromotionDecision, IssuePromotionPayload,
    IssuePromotionSafetyDecision,
};
use std::collections::{BTreeMap, BTreeSet};

use super::lint::load_issue_safety_policy;
use super::CommandFailure;

mod output;
mod promotion;

use output::promotion_decision_json;
use promotion::{
    add_issue_label, read_issue, remove_issue_label, remove_owned_labels, rollback_cleanup_labels,
    rollback_owned_labels,
};

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec issue requires a subcommand",
        )),
        [flag] if matches!(flag.as_str(), "--help" | "-h") => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "promote" => promote(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec issue command: {command}"
        ))),
    }
}

#[derive(Debug, Default)]
struct PromoteOptions {
    number: Option<u64>,
    repo: Option<String>,
    remove_labels: Vec<String>,
}

fn promote(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_promote_help();
        return Ok(());
    }
    let options = parse_promote_options(args)?;
    let repo = options
        .repo
        .ok_or_else(|| CommandFailure::diagnostic("--repo is required"))?;
    let number = options
        .number
        .ok_or_else(|| CommandFailure::diagnostic("--number is required"))?;
    let policy = load_issue_safety_policy(None);
    if policy.has_unsupported_pattern {
        return Err(CommandFailure::diagnostic(
            "issue safety policy contains unsupported custom regex",
        ));
    }
    let trusted_actors = policy
        .trusted_actors
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let result = promote_remote_issue(&repo, number, &options.remove_labels, &trusted_actors)?;
    println!(
        "{}",
        promotion_decision_json(&result.decision, result.changed)
    );
    Ok(())
}

fn parse_promote_options(args: &[String]) -> Result<PromoteOptions, CommandFailure> {
    let mut options = PromoteOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => {
                let value = argument_value(args, &mut index, "--repo")?;
                set_once(&mut options.repo, value, "--repo accepts exactly one value")?;
            }
            "--number" => {
                let value = argument_value(args, &mut index, "--number")?;
                set_number_option(&mut options.number, &value)?;
            }
            "--remove-label" => {
                let value = argument_value(args, &mut index, "--remove-label")?;
                add_remove_label(&mut options.remove_labels, value)?;
            }
            "--json" => {}
            "--help" | "-h" => return Err(help_option_conflict()),
            option => return Err(unknown_promote_option(option)),
        }
        index += 1;
    }
    Ok(options)
}

fn help_option_conflict() -> CommandFailure {
    CommandFailure::diagnostic("--help cannot be combined with issue promote options")
}

fn unknown_promote_option(option: &str) -> CommandFailure {
    CommandFailure::diagnostic(format!("unknown autospec issue promote option: {option}"))
}

fn parse_positive_number(value: &str) -> Result<u64, CommandFailure> {
    value
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| CommandFailure::diagnostic("--number must be positive"))
}

fn set_number_option(target: &mut Option<u64>, value: &str) -> Result<(), CommandFailure> {
    let number = parse_positive_number(value)?;
    set_once(target, number, "--number accepts exactly one value")
}

fn add_remove_label(labels: &mut Vec<String>, value: String) -> Result<(), CommandFailure> {
    if value != "needs-autospec-template" {
        return Err(CommandFailure::diagnostic(
            "--remove-label only accepts needs-autospec-template",
        ));
    }
    if !labels.contains(&value) {
        labels.push(value);
    }
    Ok(())
}

fn argument_value(
    args: &[String],
    index: &mut usize,
    option: &str,
) -> Result<String, CommandFailure> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| CommandFailure::diagnostic(format!("{option} requires a value")))
}

fn set_once<T>(target: &mut Option<T>, value: T, message: &str) -> Result<(), CommandFailure> {
    if target.is_some() {
        return Err(CommandFailure::diagnostic(message));
    }
    *target = Some(value);
    Ok(())
}

struct PromotionResult {
    decision: IssuePromotionDecision,
    changed: bool,
}

fn promote_remote_issue(
    repo: &str,
    number: u64,
    remove_labels: &[String],
    trusted_actors: &[&str],
) -> Result<PromotionResult, CommandFailure> {
    let initial = read_issue(repo, number)?;
    if initial.closed {
        return Err(promotion_conflict("issue is closed"));
    }
    if has_label(&initial, "auto-implement") {
        let mut normalized = initial.clone();
        normalized
            .labels
            .retain(|label| !remove_labels.contains(label));
        let decision = evaluate(&normalized, trusted_actors);
        if decision.auto_implement && decision.eligible {
            let removed = remove_owned_labels(repo, number, &initial, remove_labels)?;
            if !removed.is_empty() {
                verify_owned_cleanup(repo, number, &initial, &normalized, remove_labels, &removed)?;
            }
            return Ok(PromotionResult {
                decision,
                changed: !removed.is_empty(),
            });
        }
        remove_issue_label(repo, number, "auto-implement")?;
        let reread = read_issue(repo, number)?;
        if has_label(&reread, "auto-implement") {
            return Err(promotion_conflict(
                "unsafe pre-existing auto-implement label survived rollback",
            ));
        }
        return Ok(PromotionResult {
            decision: evaluate(&reread, trusted_actors),
            changed: true,
        });
    }

    let verdict = review_issue_safety_with_trusted_actors(&safety_input(&initial), trusted_actors);
    match verdict.decision {
        SafetyReviewDecision::Ambiguous => {
            return Ok(non_passing_result(
                &initial,
                IssuePromotionSafetyDecision::Ambiguous,
                "current_body_safety_ambiguous",
            ))
        }
        SafetyReviewDecision::Block => {
            return Ok(non_passing_result(
                &initial,
                IssuePromotionSafetyDecision::Blocked,
                "current_body_safety_block",
            ))
        }
        SafetyReviewDecision::Pass => {}
    }

    let confirmed = read_issue(repo, number)?;
    if confirmed != initial {
        return Err(promotion_conflict("issue changed before safety stamping"));
    }
    replace_safety_review_section(&initial.body, SafetyReviewDecision::Pass).map_err(|error| {
        promotion_conflict(&format!("canonical safety review is malformed: {error}"))
    })?;
    if let Err(error) = add_issue_label(repo, number, "safety:reviewed") {
        return Err(rollback_after_error(
            repo,
            number,
            &initial,
            remove_labels,
            error,
        ));
    }

    let stamped = match read_issue(repo, number) {
        Ok(issue) => issue,
        Err(error) => {
            return Err(rollback_after_error(
                repo,
                number,
                &initial,
                remove_labels,
                error,
            ))
        }
    };
    let stamped_labels = with_label(initial.labels.clone(), "safety:reviewed");
    if !snapshot_matches(&stamped, &initial, &initial.body, &stamped_labels) {
        return Err(rollback_after_error(
            repo,
            number,
            &initial,
            remove_labels,
            promotion_conflict("issue changed while safety was stamped"),
        ));
    }
    let stamped_decision = evaluate(&stamped, trusted_actors);
    if !stamped_decision.auto_implement || !stamped_decision.eligible {
        return Ok(PromotionResult {
            decision: stamped_decision,
            changed: true,
        });
    }

    if let Err(error) = add_issue_label(repo, number, "auto-implement") {
        return Err(rollback_after_error(
            repo,
            number,
            &initial,
            remove_labels,
            error,
        ));
    }
    for label in remove_labels {
        if has_label(&stamped, label) {
            if let Err(error) = remove_issue_label(repo, number, label) {
                return Err(rollback_after_error(
                    repo,
                    number,
                    &initial,
                    remove_labels,
                    error,
                ));
            }
        }
    }

    let final_issue = match read_issue(repo, number) {
        Ok(issue) => issue,
        Err(error) => {
            return Err(rollback_after_error(
                repo,
                number,
                &initial,
                remove_labels,
                error,
            ))
        }
    };
    let mut final_labels = with_label(stamped_labels, "auto-implement");
    for label in remove_labels {
        final_labels.retain(|current| current != label);
    }
    let final_decision = evaluate(&final_issue, trusted_actors);
    if !snapshot_matches(&final_issue, &initial, &initial.body, &final_labels)
        || !final_decision.auto_implement
        || !final_decision.eligible
    {
        return Err(rollback_after_error(
            repo,
            number,
            &initial,
            remove_labels,
            promotion_conflict("issue changed after auto-implement transition; label rolled back"),
        ));
    }
    Ok(PromotionResult {
        decision: final_decision,
        changed: true,
    })
}

fn rollback_after_error(
    repo: &str,
    number: u64,
    initial: &RemoteIssue,
    remove_labels: &[String],
    original: CommandFailure,
) -> CommandFailure {
    match rollback_owned_labels(repo, number, initial, remove_labels) {
        Ok(()) => original,
        Err(rollback) => CommandFailure::status(
            format!(
                "{}; original failure: {}",
                rollback.message, original.message
            ),
            rollback.exit_code,
        ),
    }
}

fn verify_owned_cleanup(
    repo: &str,
    number: u64,
    initial: &RemoteIssue,
    normalized: &RemoteIssue,
    owned_labels: &[String],
    removed_labels: &[String],
) -> Result<(), CommandFailure> {
    let reread = match read_issue(repo, number) {
        Ok(issue) => issue,
        Err(error) => {
            return Err(cleanup_rollback_after_error(
                repo,
                number,
                initial,
                owned_labels,
                removed_labels,
                error,
            ))
        }
    };
    if snapshot_matches(&reread, initial, &initial.body, &normalized.labels) {
        return Ok(());
    }
    Err(cleanup_rollback_after_error(
        repo,
        number,
        initial,
        owned_labels,
        removed_labels,
        promotion_conflict("issue changed while owned labels were finalized"),
    ))
}

fn cleanup_rollback_after_error(
    repo: &str,
    number: u64,
    initial: &RemoteIssue,
    owned_labels: &[String],
    removed_labels: &[String],
    original: CommandFailure,
) -> CommandFailure {
    match rollback_cleanup_labels(repo, number, initial, owned_labels, removed_labels) {
        Ok(()) => original,
        Err(rollback) => CommandFailure::status(
            format!(
                "{}; original failure: {}",
                rollback.message, original.message
            ),
            rollback.exit_code,
        ),
    }
}

fn snapshot_matches(
    current: &RemoteIssue,
    initial: &RemoteIssue,
    expected_body: &str,
    expected_labels: &[String],
) -> bool {
    !current.closed
        && current.number == initial.number
        && current.title == initial.title
        && current.author == initial.author
        && current.body == expected_body
        && label_set(&current.labels) == label_set(expected_labels)
}

fn evaluate(issue: &RemoteIssue, trusted_actors: &[&str]) -> IssuePromotionDecision {
    evaluate_issue_promotion_with_trusted_actors(
        IssuePromotionPayload::new(
            issue.number,
            issue.title.clone(),
            issue.body.clone(),
            issue.author.clone(),
            issue.labels.clone(),
        ),
        trusted_actors,
    )
}

fn safety_input(issue: &RemoteIssue) -> ClaimSafetyInput {
    ClaimSafetyInput::new(
        issue.labels.clone(),
        issue.title.clone(),
        issue.body.clone(),
        issue.author.clone(),
    )
}

fn non_passing_decision(
    issue: &RemoteIssue,
    safety_decision: IssuePromotionSafetyDecision,
    reason: &str,
) -> IssuePromotionDecision {
    let mut blocked_by_reason = BTreeMap::new();
    if safety_decision == IssuePromotionSafetyDecision::Blocked {
        blocked_by_reason.insert(reason.to_string(), 1);
    }
    IssuePromotionDecision {
        number: issue.number,
        title: issue.title.clone(),
        safety_decision,
        safety_reason: reason.to_string(),
        auto_implement: false,
        eligible: false,
        final_labels: without_label(issue.labels.clone(), "auto-implement"),
        blocked_by_reason,
    }
}

fn non_passing_result(
    issue: &RemoteIssue,
    safety_decision: IssuePromotionSafetyDecision,
    reason: &str,
) -> PromotionResult {
    PromotionResult {
        decision: non_passing_decision(issue, safety_decision, reason),
        changed: false,
    }
}

fn has_label(issue: &RemoteIssue, label: &str) -> bool {
    issue.labels.iter().any(|current| current == label)
}

fn with_label(mut labels: Vec<String>, label: &str) -> Vec<String> {
    if !labels.iter().any(|current| current == label) {
        labels.push(label.to_string());
    }
    labels
}

fn without_label(labels: Vec<String>, label: &str) -> Vec<String> {
    labels
        .into_iter()
        .filter(|current| current != label)
        .collect()
}

fn label_set(labels: &[String]) -> BTreeSet<&str> {
    labels.iter().map(String::as_str).collect()
}

fn promotion_conflict(detail: &str) -> CommandFailure {
    CommandFailure::diagnostic(format!("ISSUE_PROMOTION_CONFLICT: {detail}"))
}

fn print_help() {
    println!(
        "autospec issue\n\nUSAGE:\n    autospec issue [COMMAND]\n\nCOMMANDS:\n    promote    Safely stamp and promote the canonical GitHub issue"
    );
}

fn print_promote_help() {
    println!(
        "autospec issue promote\n\nUSAGE:\n    autospec issue promote --repo OWNER/REPO --number N [--remove-label needs-autospec-template] [--json]\n\nOUTPUT:\n    JSON decision with authoritative safety verdict, auto-implement state, payload eligibility, transition state, and blocked reason counts"
    );
}
