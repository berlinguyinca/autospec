use std::fs;

use autospec_core::coordination::{
    parse_repository_routing_input_json, plan_repository_routing, CanonicalTarget,
    DoNotFileRepository, RepositoryRoutingReport, RoutedFinding,
};
use autospec_core::explore::specialists::{scan_specialists_json, ScanOptions};

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
        [command, rest @ ..] if command == "specialists" => specialists(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec explore command: {command}"
        ))),
    }
}

fn repositories(args: &[String]) -> Result<(), CommandFailure> {
    if matches!(args, [flag] if flag == "--help" || flag == "-h") {
        print_repositories_help();
        return Ok(());
    }
    let options = parse_repository_options(args)?;
    let input_path = options.input.ok_or_else(|| {
        CommandFailure::diagnostic("autospec explore repositories requires --input <path>")
    })?;
    let input = fs::read_to_string(&input_path).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not read repository input {input_path}: {error}"
        ))
    })?;
    let input = parse_repository_routing_input_json(&input).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not parse repository input {input_path}: {error}"
        ))
    })?;
    let report = plan_repository_routing(&input);
    println!("{}", report_json(&report));
    Ok(())
}

fn specialists(args: &[String]) -> Result<(), CommandFailure> {
    if matches!(args, [flag] if flag == "--help" || flag == "-h") {
        print_specialists_help();
        return Ok(());
    }
    let options = parse_specialist_options(args)?;
    let repo_dir = options.repo_dir.ok_or_else(|| {
        CommandFailure::diagnostic("autospec explore specialists requires --repo-dir <path>")
    })?;
    let mut scan_options = ScanOptions::new(&repo_dir)
        .with_num_specialists(options.num_specialists.unwrap_or(3))
        .force(options.force);
    // Keep the stored path exactly user-selected but make diagnostics clearer if it is unreadable.
    if !std::path::Path::new(&repo_dir).is_dir() {
        return Err(CommandFailure::diagnostic(format!(
            "autospec explore specialists --repo-dir is not a directory: {repo_dir}"
        )));
    }
    scan_options.repo_dir = std::path::PathBuf::from(&repo_dir);
    let json = scan_specialists_json(&scan_options).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not scan explore specialists in {repo_dir}: {error}"
        ))
    })?;
    print!("{json}");
    Ok(())
}

#[derive(Debug, Default)]
struct SpecialistOptions {
    repo_dir: Option<String>,
    num_specialists: Option<usize>,
    force: bool,
}

fn parse_specialist_options(args: &[String]) -> Result<SpecialistOptions, CommandFailure> {
    let mut options = SpecialistOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo-dir" => {
                let value = option_value(args, &mut index, "--repo-dir")?;
                replace_once(&mut options.repo_dir, value, "--repo-dir")?;
            }
            "--num-specialists" => {
                let value = option_value(args, &mut index, "--num-specialists")?;
                let parsed = value.parse::<usize>().map_err(|_| {
                    CommandFailure::diagnostic("--num-specialists requires a non-negative integer")
                })?;
                if options.num_specialists.replace(parsed).is_some() {
                    return Err(CommandFailure::diagnostic(
                        "--num-specialists accepts exactly one value",
                    ));
                }
            }
            "--force" => options.force = true,
            option => return unknown_specialist_option(option),
        }
        index += 1;
    }
    Ok(options)
}

fn unknown_specialist_option(option: &str) -> Result<SpecialistOptions, CommandFailure> {
    Err(CommandFailure::diagnostic(format!(
        "unknown autospec explore specialists option: {option}"
    )))
}

#[derive(Debug, Default)]
struct RepositoryOptions {
    input: Option<String>,
}

fn parse_repository_options(args: &[String]) -> Result<RepositoryOptions, CommandFailure> {
    let mut options = RepositoryOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--input" => {
                let value = option_value(args, &mut index, "--input")?;
                replace_once(&mut options.input, value, "--input")?;
            }
            option => return unknown_repository_option(option),
        }
        index += 1;
    }
    Ok(options)
}

fn option_value(
    args: &[String],
    index: &mut usize,
    option: &str,
) -> Result<String, CommandFailure> {
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

fn replace_once(
    target: &mut Option<String>,
    value: String,
    option: &str,
) -> Result<(), CommandFailure> {
    if target.replace(value).is_some() {
        return Err(CommandFailure::diagnostic(format!(
            "{option} accepts exactly one value"
        )));
    }
    Ok(())
}

fn unknown_repository_option(option: &str) -> Result<RepositoryOptions, CommandFailure> {
    Err(CommandFailure::diagnostic(format!(
        "unknown autospec explore repositories option: {option}"
    )))
}

fn print_help() {
    println!(
        "autospec explore\n\nUSAGE:\n    autospec explore [COMMAND]\n\nCOMMANDS:\n    repositories    Infer canonical repositories for org-sweep findings\n    specialists     Discover domain-specialist roster for autospec-explore\n\nOPTIONS:\n    -h, --help       Print help"
    );
}

fn print_repositories_help() {
    println!(
        "autospec explore repositories\n\nUSAGE:\n    autospec explore repositories --input <PATH>\n\nOPTIONS:\n    --input <PATH>   Read repository evidence JSON\n    -h, --help       Print help"
    );
}

fn print_specialists_help() {
    println!(
        "autospec explore specialists\n\nUSAGE:\n    autospec explore specialists --repo-dir <PATH> [--num-specialists <N>] [--force]\n\nOPTIONS:\n    --repo-dir <PATH>       Repository directory to scan\n    --num-specialists <N>   Maximum roster size (default 3, cap 6)\n    --force                 Refresh even when a valid cache exists\n    -h, --help              Print help"
    );
}

fn report_json(report: &RepositoryRoutingReport) -> String {
    format!(
        "{{\"canonical_targets\":[{}],\"do_not_file_by_default\":[{}],\"routed_findings\":[{}]}}",
        report
            .canonical_targets
            .iter()
            .map(canonical_target_json)
            .collect::<Vec<_>>()
            .join(","),
        report
            .do_not_file_by_default
            .iter()
            .map(do_not_file_json)
            .collect::<Vec<_>>()
            .join(","),
        report
            .routed_findings
            .iter()
            .map(routed_finding_json)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn canonical_target_json(target: &CanonicalTarget) -> String {
    format!(
        "{{\"repository\":{},\"score\":{},\"reasons\":{},\"routed_fingerprints\":{}}}",
        json_string(&target.repository),
        target.score,
        json_string_array(&target.reasons),
        json_string_array(&target.routed_fingerprints)
    )
}

fn do_not_file_json(repository: &DoNotFileRepository) -> String {
    format!(
        "{{\"repository\":{},\"reason\":{}}}",
        json_string(&repository.repository),
        json_string(&repository.reason)
    )
}

fn routed_finding_json(finding: &RoutedFinding) -> String {
    format!(
        "{{\"target_repository\":{},\"source_repository\":{},\"fingerprint\":{},\"title\":{},\"evidence\":{}}}",
        json_string(&finding.target_repository),
        json_string(&finding.source_repository),
        json_string(&finding.fingerprint),
        json_string(&finding.title),
        json_string(&finding.evidence)
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
