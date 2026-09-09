use std::fs;
use std::path::Path;

use autospec_core::graph::optimization::{compare, GraphMetrics};

use super::{is_json, CommandFailure};

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec graph requires a subcommand",
        )),
        [flag] if flag == "--help" || flag == "-h" => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "compare" => run_compare(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec graph command: {command}"
        ))),
    }
}

fn run_compare(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        print_compare_help();
        return Ok(());
    }
    let (before_path, after_path) = parse_compare_options(args)?;
    let before = read_metrics(&before_path, "before")?;
    let after = read_metrics(&after_path, "after")?;
    let summary = compare(&before, &after);

    if is_json(args) {
        let payload = serde_json::json!({
            "removed": summary.removed,
            "added": summary.added,
            "split": summary.split,
            "merged": summary.merged,
            "edge_delta": summary.edge_delta,
            "critical_path_before": summary.critical_path_before,
            "critical_path_after": summary.critical_path_after,
            "rejected": summary.rejected,
        });
        println!("{payload}");
    } else {
        println!("{}", summary.render());
    }

    if summary.rejected {
        return Err(CommandFailure::status(
            "retry rejected: after-graph critical path grew",
            1,
        ));
    }
    Ok(())
}

fn parse_compare_options(args: &[String]) -> Result<(String, String), CommandFailure> {
    let mut before: Option<String> = None;
    let mut after: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        let value = match args[index].as_str() {
            "--before" => &mut before,
            "--after" => &mut after,
            "--json" => {
                index += 1;
                continue;
            }
            other => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec graph compare option: {other}"
                )))
            }
        };
        let Some(argument) = args.get(index + 1) else {
            return Err(CommandFailure::diagnostic(format!(
                "{} requires a path",
                args[index]
            )));
        };
        *value = Some(argument.clone());
        index += 2;
    }
    match (before, after) {
        (Some(before), Some(after)) => Ok((before, after)),
        _ => Err(CommandFailure::diagnostic(
            "autospec graph compare requires --before <metrics.json> and --after <metrics.json>",
        )),
    }
}

fn read_metrics(path: &str, side: &str) -> Result<GraphMetrics, CommandFailure> {
    let source = fs::read_to_string(Path::new(path)).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot read {side} metrics: {error}"))
    })?;
    serde_json::from_str(&source)
        .map_err(|error| CommandFailure::diagnostic(format!("{side} metrics are invalid: {error}")))
}

fn print_help() {
    println!(
        "autospec graph\n\nUSAGE:\n    autospec graph <COMMAND>\n\nCOMMANDS:\n    compare   Diff dependency-graph metrics before/after a concurrency optimization pass\n\nOPTIONS:\n    -h, --help    Print help"
    );
}

fn print_compare_help() {
    println!(
        "autospec graph compare\n\nUSAGE:\n    autospec graph compare --before <metrics.json> --after <metrics.json> [--json]\n\nReads two GraphMetrics JSON files (issue_count, hard_edge_count, critical_path_length),\nprints the section 25 concurrency-review summary, and exits 1 if the after-graph\ncritical path grew.\n\nOPTIONS:\n    --before <path>    GraphMetrics JSON captured before the optimization pass\n    --after <path>     GraphMetrics JSON captured after the optimization pass\n    --json             Render the summary as JSON\n    -h, --help         Print help"
    );
}
