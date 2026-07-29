use autospec_core::runtime_policy::classify_path;

use crate::commands::CommandFailure;

mod audit;
mod env;

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [command, path] if command == "classify" => {
            print_classification(path, false);
            Ok(())
        }
        [command, path, flag] if command == "classify" && flag == "--json" => {
            print_classification(path, true);
            Ok(())
        }
        [command, rest @ ..] if command == "audit" => {
            audit::run(rest).map_err(CommandFailure::diagnostic)
        }
        [command, rest @ ..] if command == "env" => env::run(rest),
        [flag] if flag == "--help" || flag == "-h" => {
            print_help();
            Ok(())
        }
        [] => Err(CommandFailure::diagnostic(
            "autospec runtime requires a subcommand",
        )),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec runtime command: {command}"
        ))),
    }
}

fn print_classification(path: &str, json: bool) {
    let verdict = classify_path(path);
    if json {
        println!(
            "{{\"command\":\"runtime classify\",\"path\":\"{}\",\"runtime\":\"{}\",\"class\":\"{}\",\"reasons\":[{}]}}",
            escape_json(&verdict.path),
            verdict.runtime.as_str(),
            verdict.class.as_str(),
            verdict
                .reasons
                .iter()
                .map(|reason| format!("\"{}\"", escape_json(reason)))
                .collect::<Vec<_>>()
                .join(",")
        );
    } else {
        println!(
            "{} {} {}",
            verdict.class.as_str(),
            verdict.path,
            verdict.reasons.join("; ")
        );
    }
}

fn print_help() {
    println!(
        "autospec runtime\n\nUSAGE:\n    autospec runtime classify <PATH> [--json]\n    autospec runtime audit [--root PATH] [--json]\n    autospec runtime env init [--repo PATH] [--manifest agent|autospec] [--force]\n    autospec runtime env up [--repo PATH] [--mode MODE]\n    autospec runtime env status [--repo PATH] [--mode MODE]\n    autospec runtime env down [--repo PATH] [--mode MODE]\n    autospec runtime env exec [--repo PATH] [--mode MODE] -- COMMAND [ARGS...]\n    autospec runtime env session [--repo PATH] [--mode MODE] [--keep-alive] -- COMMAND [ARGS...]\n\nCOMMANDS:\n    classify       Classify a repository path by runtime ownership policy\n    audit          List platform files grouped by runtime migration class\n    env            Manage isolated runtime environment state"
    );
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
