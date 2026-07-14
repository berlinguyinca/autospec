use std::fs;
use std::path::PathBuf;

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
    let plan = ValidationPlan::build(&catalog, options)?;
    let report = ValidationRunner::run_plan(&plan, &root);
    let aggregate = report.aggregate()?;

    if options.json {
        println!("{}", report.to_json()?);
    } else {
        println!(
            "AutoSpec validation: status={} total={} passed={} failed={} required_failed={} optional_failed={}",
            aggregate.status.as_str(),
            aggregate.total,
            aggregate.passed,
            aggregate.failed,
            aggregate.required_failed,
            aggregate.optional_failed
        );
        for result in &report.results {
            let status = if result.is_success() {
                "passed"
            } else {
                "failed"
            };
            println!("- {}: {status}", result.id);
        }
    }

    if aggregate.status == ValidationStatus::Failed {
        return Err("direct Rust validation failed required checks".to_string());
    }
    Ok(())
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
    if aggregate.status == ValidationStatus::Failed {
        return Err("captured validation results failed required checks".to_string());
    }
    Ok(())
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
