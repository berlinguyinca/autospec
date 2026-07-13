use std::fs;
use std::path::PathBuf;
use std::process::Command;

use autospec_core::validation::affected::AffectedSet;
use autospec_core::validation::{ValidationAggregate, ValidationReport, ValidationStatus};

#[derive(Debug)]
enum Mode {
    Planning { paths: Vec<String> },
    ShadowResults(PathBuf),
}

#[derive(Debug)]
struct Options {
    mode: Mode,
    json: bool,
}

pub fn run(args: &[String]) -> Result<(), String> {
    if std::env::var("AUTOSPEC_VALIDATE_FROM_SHELL")
        .ok()
        .as_deref()
        == Some("1")
        && !is_shadow_results_command(args)
    {
        return run_legacy_shell(args);
    }

    let options = parse_options(args)?;
    match options.mode {
        Mode::Planning { paths } => {
            let affected = AffectedSet::from_paths(&paths);
            if options.json {
                render_json(&affected);
            } else {
                render_text(&affected);
            }
        }
        Mode::ShadowResults(path) => {
            let document = fs::read_to_string(&path).map_err(|error| {
                format!(
                    "failed to read captured validation results {}: {error}",
                    path.display()
                )
            })?;
            let aggregate = ValidationReport::from_json(&document)?.aggregate()?;
            if options.json {
                render_shadow_json(&aggregate);
            } else {
                render_shadow_text(&aggregate);
            }
            if aggregate.status == ValidationStatus::Failed {
                return Err("captured validation results failed required checks".to_string());
            }
        }
    }
    Ok(())
}

fn is_shadow_results_command(args: &[String]) -> bool {
    args.iter().any(|argument| argument == "--shadow-results")
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut paths = Vec::new();
    let mut shadow_results = None;
    let mut json = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--path" => {
                if shadow_results.is_some() {
                    return Err(
                        "autospec validate --path cannot be combined with --shadow-results"
                            .to_string(),
                    );
                }
                index += 1;
                let path = args
                    .get(index)
                    .filter(|path| !path.is_empty() && !path.starts_with("--"))
                    .ok_or_else(|| "autospec validate --path requires a path".to_string())?;
                paths.push(path.clone());
            }
            "--shadow-results" => {
                if !paths.is_empty() || shadow_results.is_some() {
                    return Err("autospec validate accepts only one mode".to_string());
                }
                index += 1;
                let path = args
                    .get(index)
                    .filter(|path| !path.is_empty() && !path.starts_with("--"))
                    .ok_or_else(|| {
                        "autospec validate --shadow-results requires a path".to_string()
                    })?;
                shadow_results = Some(PathBuf::from(path));
            }
            option => {
                return Err(format!(
                    "unknown autospec validate option: {option}; use bash scripts/validate.sh for execution"
                ));
            }
        }
        index += 1;
    }

    let mode = shadow_results
        .map(Mode::ShadowResults)
        .unwrap_or(Mode::Planning { paths });
    Ok(Options { mode, json })
}

fn render_text(affected: &AffectedSet) {
    println!(
        "AutoSpec validation plan: {} check(s); no commands were run",
        affected.rules.len()
    );
    for rule in &affected.rules {
        println!("- {}: {}", rule.check, rule.reason);
    }
}

fn render_json(affected: &AffectedSet) {
    let changed_paths = json_array(&affected.changed_paths);
    let checks = affected
        .rules
        .iter()
        .map(|rule| {
            format!(
                "{{\"name\":\"{}\",\"reason\":\"{}\"}}",
                escape_json(&rule.check),
                escape_json(&rule.reason)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"command\":\"validate\",\"mode\":\"planning\",\"changed_paths\":{changed_paths},\"checks\":[{checks}]}}"
    );
}

fn render_shadow_json(aggregate: &ValidationAggregate) {
    println!(
        "{{\"command\":\"validate\",\"mode\":\"shadow-results\",\"aggregate\":{}}}",
        aggregate.to_json()
    );
}

fn render_shadow_text(aggregate: &ValidationAggregate) {
    println!(
        "AutoSpec validation shadow: status={} total={} passed={} failed={} required_failed={} optional_failed={}; no commands were run",
        aggregate.status.as_str(),
        aggregate.total,
        aggregate.passed,
        aggregate.failed,
        aggregate.required_failed,
        aggregate.optional_failed
    );
}

fn json_array(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("\"{}\"", escape_json(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
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

fn run_legacy_shell(args: &[String]) -> Result<(), String> {
    let status = Command::new("bash")
        .arg("scripts/validate.sh")
        .args(args)
        .env("AUTOSPEC_FORCE_LEGACY_SHELL", "1")
        .env("AUTOSPEC_VALIDATE_FROM_RUST", "1")
        .status()
        .map_err(|error| format!("failed to run legacy shell validation: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "legacy shell validation failed with status {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string())
        ))
    }
}
