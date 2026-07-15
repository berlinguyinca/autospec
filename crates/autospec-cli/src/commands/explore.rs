use std::fs;

use autospec_core::exploration::{route_repositories, ExplorationInput};

use super::CommandFailure;

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec explore requires a subcommand",
        )),
        [flag] if matches!(flag.as_str(), "--help" | "-h") => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "repositories" => repositories(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec explore command: {command}"
        ))),
    }
}

fn repositories(args: &[String]) -> Result<(), CommandFailure> {
    let input = parse_input_path(args)?;
    let document = fs::read_to_string(&input).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not read repository exploration input {input}: {error}"
        ))
    })?;
    let input = ExplorationInput::from_json(&document).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not parse repository exploration input: {error}"
        ))
    })?;
    let report = route_repositories(&input).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not route repository exploration input: {error}"
        ))
    })?;
    println!("{}", report.to_json());
    Ok(())
}

fn parse_input_path(args: &[String]) -> Result<String, CommandFailure> {
    let mut input = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--input" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(CommandFailure::diagnostic("--input requires an argument"));
                };
                if value.is_empty() || value.starts_with('-') {
                    return Err(CommandFailure::diagnostic("--input requires an argument"));
                }
                if input.replace(value.clone()).is_some() {
                    return Err(CommandFailure::diagnostic(
                        "--input accepts exactly one value",
                    ));
                }
            }
            "--help" | "-h" => {
                return Err(CommandFailure::diagnostic(
                    "--help cannot be combined with explore repositories options",
                ));
            }
            option => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec explore repositories option: {option}"
                )));
            }
        }
        index += 1;
    }
    input.ok_or_else(|| {
        CommandFailure::diagnostic("autospec explore repositories requires --input <path>")
    })
}

fn print_help() {
    println!("USAGE:\n    autospec explore repositories --input <PATH>");
}
