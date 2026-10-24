use autospec_core::incident::{parse_agent_failure_event, IncidentIssueProjection};
use serde_json::json;
use std::fs;
use std::process::Command;

use super::CommandFailure;

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    if args.first().map(String::as_str) != Some("ingest") {
        return Err(CommandFailure::diagnostic(
            "autospec incident requires the ingest subcommand",
        ));
    }
    let options = parse_options(&args[1..])?;
    let event = read_event(&options)?;
    let projection = IncidentIssueProjection::from_event(&event);
    if options.dry_run {
        println!("{}", render_dry_run(&event, &projection));
        return Ok(());
    }

    let existing = find_existing_issue(&options.repo, &projection.marker())?;
    let (number, deduplicated) = match existing {
        Some(number) => (number, true),
        None => (create_issue(&options.repo, &projection)?, false),
    };
    println!(
        "{}",
        render_result(&event, &projection, number, deduplicated)
    );
    Ok(())
}

fn read_event(
    options: &IncidentOptions,
) -> Result<autospec_core::incident::AgentFailureEvent, CommandFailure> {
    let document = fs::read_to_string(&options.event_file).map_err(|error| {
        CommandFailure::diagnostic(format!("could not read incident event: {error}"))
    })?;
    let event = parse_agent_failure_event(&document).map_err(CommandFailure::diagnostic)?;
    if event.repo != options.repo {
        return Err(CommandFailure::diagnostic(format!(
            "incident repository {} does not match --repo {}",
            event.repo, options.repo
        )));
    }
    Ok(event)
}

fn render_dry_run(
    event: &autospec_core::incident::AgentFailureEvent,
    projection: &IncidentIssueProjection,
) -> String {
    serde_json::to_string_pretty(&json!({
        "ok": true,
        "dry_run": true,
        "fingerprint": event.fingerprint,
        "deduplicated": false,
        "issue_title": projection.title,
        "issue_body": projection.body,
        "labels": projection.labels,
        "debugger_dispatch": debugger_json(projection),
    }))
    .expect("incident projection is JSON serializable")
}

fn render_result(
    event: &autospec_core::incident::AgentFailureEvent,
    projection: &IncidentIssueProjection,
    number: u64,
    deduplicated: bool,
) -> String {
    serde_json::to_string(&json!({
        "ok": true,
        "dry_run": false,
        "fingerprint": event.fingerprint,
        "deduplicated": deduplicated,
        "issue": number,
        "debugger_dispatch": debugger_json(projection),
    }))
    .expect("incident result is JSON serializable")
}

fn debugger_json(projection: &IncidentIssueProjection) -> serde_json::Value {
    json!({
        "role": projection.debugger_dispatch.role,
        "dry_run": projection.debugger_dispatch.dry_run,
        "prompt": projection.debugger_dispatch.prompt,
    })
}

#[derive(Debug, Default)]
struct RawIncidentOptions {
    repo: Option<String>,
    event_file: Option<String>,
    dry_run: bool,
}

struct IncidentOptions {
    repo: String,
    event_file: String,
    dry_run: bool,
}

fn parse_options(args: &[String]) -> Result<IncidentOptions, CommandFailure> {
    let mut options = RawIncidentOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => {
                index += 1;
                options.repo = Some(required_value(args, index, "--repo")?);
            }
            "--event-file" => {
                index += 1;
                options.event_file = Some(required_value(args, index, "--event-file")?);
            }
            "--dry-run" | "--json" => {}
            "--help" | "-h" => {
                print_help();
                return Err(CommandFailure::status("", 0));
            }
            option => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown incident option: {option}"
                )))
            }
        }
        index += 1;
    }
    Ok(IncidentOptions {
        repo: options
            .repo
            .ok_or_else(|| CommandFailure::diagnostic("--repo is required"))?,
        event_file: options
            .event_file
            .ok_or_else(|| CommandFailure::diagnostic("--event-file is required"))?,
        dry_run: options.dry_run || args.iter().any(|arg| arg == "--dry-run"),
    })
}

fn required_value(args: &[String], index: usize, option: &str) -> Result<String, CommandFailure> {
    args.get(index)
        .filter(|value| !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| CommandFailure::diagnostic(format!("{option} requires a value")))
}

fn print_help() {
    println!("autospec incident ingest --repo OWNER/REPO --event-file PATH [--dry-run] [--json]");
}

fn find_existing_issue(repo: &str, marker: &str) -> Result<Option<u64>, CommandFailure> {
    let endpoint = format!("repos/{repo}/issues?state=open&labels=origin:pi-incident&per_page=100");
    let output = Command::new("gh")
        .args(["api", &endpoint])
        .output()
        .map_err(|error| {
            CommandFailure::transient(format!("could not execute gh issue lookup: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandFailure::transient(format!(
            "gh issue lookup failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let issues: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        CommandFailure::transient(format!("could not parse gh issue lookup: {error}"))
    })?;
    Ok(issues.as_array().and_then(|items| {
        items.iter().find_map(|issue| {
            (issue.get("body")?.as_str()?.contains(marker))
                .then(|| issue.get("number")?.as_u64())
                .flatten()
        })
    }))
}

fn create_issue(repo: &str, projection: &IncidentIssueProjection) -> Result<u64, CommandFailure> {
    let labels = projection.labels.join(",");
    let output = Command::new("gh")
        .args([
            "api",
            "--method",
            "POST",
            &format!("repos/{repo}/issues"),
            "-f",
            &format!("title={}", projection.title),
            "-f",
            &format!("body={}", projection.body),
            "-f",
            &format!("labels={labels}"),
            "--jq",
            ".number",
        ])
        .output()
        .map_err(|error| {
            CommandFailure::transient(format!("could not execute gh issue create: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandFailure::transient(format!(
            "gh issue create failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| {
            CommandFailure::transient("gh issue create returned an invalid issue number")
        })
}
