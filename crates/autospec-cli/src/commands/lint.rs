use std::fs;
use std::io::{self, Read};

use autospec_core::lint::{lint_issue_body, IssueLintFinding};

use super::CommandFailure;

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec lint requires a subcommand",
        )),
        [flag] if flag == "--help" || flag == "-h" => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "issue" => run_issue(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec lint command: {command}"
        ))),
    }
}

fn run_issue(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_issue_help();
        return Ok(());
    }
    let options = parse_issue_options(args)?;
    let body = read_body(&options.body_path)?;
    let findings = lint_issue_body(&body);

    if options.json {
        print_json(&findings);
    } else {
        print_text(&findings);
    }

    if findings.is_empty() {
        Ok(())
    } else {
        Err(CommandFailure::status(
            String::new(),
            findings.len().min(64) as i32,
        ))
    }
}

struct IssueOptions {
    body_path: String,
    json: bool,
}

fn parse_issue_options(args: &[String]) -> Result<IssueOptions, CommandFailure> {
    let mut body_path = None;
    let mut json = false;
    for argument in args {
        match argument.as_str() {
            "--json" => json = true,
            "--help" | "-h" => {
                return Err(CommandFailure::diagnostic(
                    "autospec lint issue --help cannot be combined with other arguments",
                ));
            }
            "-" => set_body_path(&mut body_path, argument)?,
            option if option.starts_with('-') => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec lint issue option: {option}"
                )));
            }
            path => set_body_path(&mut body_path, path)?,
        }
    }

    let Some(body_path) = body_path else {
        return Err(CommandFailure::diagnostic(
            "autospec lint issue requires a body path",
        ));
    };
    Ok(IssueOptions { body_path, json })
}

fn set_body_path(slot: &mut Option<String>, path: &str) -> Result<(), CommandFailure> {
    if slot.replace(path.to_owned()).is_some() {
        return Err(CommandFailure::diagnostic(
            "autospec lint issue accepts exactly one body path",
        ));
    }
    Ok(())
}

fn read_body(path: &str) -> Result<String, CommandFailure> {
    if path == "-" {
        let mut body = String::new();
        io::stdin().read_to_string(&mut body).map_err(|error| {
            CommandFailure::diagnostic(format!("could not read issue body from stdin: {error}"))
        })?;
        return Ok(body);
    }
    fs::read_to_string(path).map_err(|error| {
        CommandFailure::diagnostic(format!("could not read issue body {path}: {error}"))
    })
}

fn print_text(findings: &[IssueLintFinding]) {
    for finding in findings {
        eprintln!("{}: {}", finding.rule_id(), finding.message);
    }
}

fn print_json(findings: &[IssueLintFinding]) {
    if findings.is_empty() {
        println!("[]");
        return;
    }
    println!("[");
    for (index, finding) in findings.iter().enumerate() {
        let separator = if index + 1 == findings.len() { "" } else { "," };
        println!(
            "  {{\"rule\":\"{}\",\"description\":\"{}\"}}{separator}",
            finding.rule_id(),
            escape_json(&finding.message)
        );
    }
    println!("]");
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn print_help() {
    println!(
        "autospec lint\n\nUSAGE:\n    autospec lint <COMMAND>\n\nCOMMANDS:\n    issue       Lint an issue body"
    );
}

fn print_issue_help() {
    println!(
        "autospec lint issue\n\nUSAGE:\n    autospec lint issue [--json] <BODY_PATH>\n\nBODY_PATH:\n    -           Read the issue body from standard input\n\nOPTIONS:\n    --json      Write ordered findings as JSON\n    -h, --help  Print help"
    );
}
