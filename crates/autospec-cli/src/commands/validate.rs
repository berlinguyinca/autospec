use std::fs;
use std::path::PathBuf;
use std::process::Command;

use autospec_core::validation::affected::AffectedSet;
use autospec_core::validation::{
    ValidationAggregate, ValidationCatalog, ValidationOptions, ValidationPlan, ValidationReport,
    ValidationRunner, ValidationStatus,
};

pub fn run(args: &[String]) -> Result<(), String> {
    let options = ValidationOptions::parse(args)?;
    if let Some(path) = options.shadow_results.as_ref() {
        render_shadow_results(path, options.json)
    } else if options.requests_execution() {
        run_direct(&options)
    } else {
        let affected = AffectedSet::from_paths(&options.paths);
        if options.json {
            render_json(&affected);
        } else {
            render_text(&affected);
        }
        Ok(())
    }
}

fn run_direct(options: &ValidationOptions) -> Result<(), String> {
    let root = std::env::current_dir()
        .map_err(|error| format!("could not determine validation root: {error}"))?;
    let catalog = ValidationCatalog::standard();
    let plan = match options.changed_base.as_deref() {
        Some(base) => {
            let changed_paths = changed_paths_from_git(&root, base)?;
            ValidationPlan::build_with_changed_paths(&catalog, options, changed_paths)?
        }
        None => ValidationPlan::build(&catalog, options)?,
    };
    let report = ValidationRunner::run_plan(&plan, &root);
    let aggregate = report.aggregate()?;

    if options.json {
        println!("{}", report.to_json()?);
    } else {
        println!(
            "AutoSpec validation: status={} total={} passed={} failed={} unknown={} required_failed={} required_unknown={} optional_failed={}",
            aggregate.status.as_str(),
            aggregate.total,
            aggregate.passed,
            aggregate.failed,
            aggregate.unknown,
            aggregate.required_failed,
            aggregate.required_unknown,
            aggregate.optional_failed
        );
        for result in &report.results {
            match &result.unmeasured {
                Some(reason) => println!("- {}: unknown ({reason})", result.id),
                None if result.is_success() => println!("- {}: passed", result.id),
                None => println!("- {}: failed", result.id),
            }
        }
    }

    unmeasured_or_failed(aggregate.status, "direct Rust validation")
}

fn changed_paths_from_git(root: &std::path::Path, base: &str) -> Result<Vec<String>, String> {
    for range in [format!("{base}...HEAD"), base.to_string()] {
        let output = Command::new("git")
            .args(["diff", "--name-only", &range])
            .current_dir(root)
            .output()
            .map_err(|error| format!("could not read changed files from git: {error}"))?;
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|error| format!("git returned non-UTF-8 changed paths: {error}"))?;
        return Ok(stdout
            .lines()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(ToOwned::to_owned)
            .collect());
    }

    Err(format!(
        "autospec validate could not resolve changed paths from git base {base}"
    ))
}

fn render_shadow_results(path: &PathBuf, json: bool) -> Result<(), String> {
    let document = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read captured validation results {}: {error}",
            path.display()
        )
    })?;
    let aggregate = ValidationReport::from_json(&document)?.aggregate()?;
    if json {
        render_shadow_json(&aggregate);
    } else {
        render_shadow_text(&aggregate);
    }
    unmeasured_or_failed(aggregate.status, "captured validation results")
}

/// Turns a non-passing status into an error, keeping "unknown" out of the success path.
///
/// The predicate is `is_passed`, not `== Failed`. An `== Failed` test lets the new
/// `Unknown` status through as success, which is precisely the class of false pass this
/// exists to stop (#3535): a gate that reports clean because its tools never ran.
fn unmeasured_or_failed(status: ValidationStatus, subject: &str) -> Result<(), String> {
    match status {
        ValidationStatus::Passed => Ok(()),
        ValidationStatus::Failed => Err(format!("{subject} failed required checks")),
        ValidationStatus::Unknown => Err(format!(
            "{subject} could not measure required checks; \
             the report is unknown, not a pass"
        )),
    }
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
        "AutoSpec validation shadow: status={} total={} passed={} failed={} unknown={} required_failed={} required_unknown={} optional_failed={}; no commands were run",
        aggregate.status.as_str(),
        aggregate.total,
        aggregate.passed,
        aggregate.failed,
        aggregate.unknown,
        aggregate.required_failed,
        aggregate.required_unknown,
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
