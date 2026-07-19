use std::fs;
use std::path::PathBuf;

use autospec_core::safety::{
    evaluate_issue_promotion, IssuePromotionDecision, IssuePromotionPayload,
};

use super::CommandFailure;

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
    title: Option<String>,
    body: Option<String>,
    body_file: Option<PathBuf>,
    author: Option<String>,
    labels: Vec<String>,
}

fn promote(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_promote_help();
        return Ok(());
    }
    let options = parse_promote_options(args)?;
    let body = match (options.body, options.body_file) {
        (Some(_), Some(_)) => {
            return Err(CommandFailure::diagnostic(
                "--body and --body-file are mutually exclusive",
            ))
        }
        (Some(body), None) => body,
        (None, Some(path)) => fs::read_to_string(&path).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not read --body-file {}: {error}",
                path.display()
            ))
        })?,
        (None, None) => {
            return Err(CommandFailure::diagnostic(
                "autospec issue promote requires --body or --body-file",
            ))
        }
    };
    let payload = IssuePromotionPayload::new(
        options
            .number
            .ok_or_else(|| CommandFailure::diagnostic("--number is required"))?,
        options
            .title
            .ok_or_else(|| CommandFailure::diagnostic("--title is required"))?,
        body,
        options
            .author
            .ok_or_else(|| CommandFailure::diagnostic("--author is required"))?,
        options.labels,
    );
    let decision = evaluate_issue_promotion(payload);
    println!("{}", promotion_decision_json(&decision));
    Ok(())
}

fn parse_promote_options(args: &[String]) -> Result<PromoteOptions, CommandFailure> {
    let mut options = PromoteOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--number" => {
                let value = argument_value(args, &mut index, "--number")?;
                let number = value
                    .parse::<u64>()
                    .ok()
                    .filter(|number| *number > 0)
                    .ok_or_else(|| CommandFailure::diagnostic("--number must be positive"))?;
                set_once(
                    &mut options.number,
                    number,
                    "--number accepts exactly one value",
                )?;
            }
            "--title" => {
                let value = argument_value(args, &mut index, "--title")?;
                set_once(
                    &mut options.title,
                    value,
                    "--title accepts exactly one value",
                )?;
            }
            "--body" => {
                let value = argument_value(args, &mut index, "--body")?;
                set_once(&mut options.body, value, "--body accepts exactly one value")?;
            }
            "--body-file" => {
                let value = argument_value(args, &mut index, "--body-file")?;
                set_once(
                    &mut options.body_file,
                    PathBuf::from(value),
                    "--body-file accepts exactly one value",
                )?;
            }
            "--author" => {
                let value = argument_value(args, &mut index, "--author")?;
                set_once(
                    &mut options.author,
                    value,
                    "--author accepts exactly one value",
                )?;
            }
            "--label" => {
                let value = argument_value(args, &mut index, "--label")?;
                options.labels.push(value);
            }
            "--labels" => {
                let value = argument_value(args, &mut index, "--labels")?;
                options.labels.extend(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|label| !label.is_empty())
                        .map(str::to_string),
                );
            }
            "--json" => {}
            "--help" | "-h" => {
                return Err(CommandFailure::diagnostic(
                    "--help cannot be combined with issue promote options",
                ))
            }
            option => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec issue promote option: {option}"
                )))
            }
        }
        index += 1;
    }
    Ok(options)
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

fn promotion_decision_json(decision: &IssuePromotionDecision) -> String {
    format!(
        "{{\"issue\":{{\"number\":{},\"title\":{}}},\"safety\":{{\"decision\":{},\"reason\":{}}},\"auto-implement\":{},\"drainable\":{},\"final_labels\":{},\"blocked_by_reason\":{}}}",
        decision.number,
        json_string(&decision.title),
        json_string(decision.safety_decision.as_str()),
        json_string(&decision.safety_reason),
        json_bool(decision.auto_implement),
        json_bool(decision.drainable),
        json_string_array(&decision.final_labels),
        json_usize_map(&decision.blocked_by_reason),
    )
}

fn json_usize_map(values: &std::collections::BTreeMap<String, usize>) -> String {
    format!(
        "{{{}}}",
        values
            .iter()
            .map(|(key, value)| format!("{}:{}", json_string(key), value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn json_string_array(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

fn json_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn print_help() {
    println!(
        "autospec issue\n\nUSAGE:\n    autospec issue [COMMAND]\n\nCOMMANDS:\n    promote    Evaluate whether a final issue payload may receive auto-implement"
    );
}

fn print_promote_help() {
    println!(
        "autospec issue promote\n\nUSAGE:\n    autospec issue promote --number N --title TITLE --body BODY --author LOGIN [--label LABEL ...]\n    autospec issue promote --number N --title TITLE --body-file PATH --author LOGIN [--labels CSV]\n\nOUTPUT:\n    JSON decision with safety verdict, auto-implement grant, drainability, and blocked reason counts"
    );
}
