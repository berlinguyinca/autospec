use std::process::Command;

use autospec_core::validation::affected::AffectedSet;

#[derive(Debug)]
struct Options {
    paths: Vec<String>,
    json: bool,
}

pub fn run(args: &[String]) -> Result<(), String> {
    if std::env::var("AUTOSPEC_VALIDATE_FROM_SHELL")
        .ok()
        .as_deref()
        == Some("1")
    {
        return run_legacy_shell(args);
    }

    let options = parse_options(args)?;
    let affected = AffectedSet::from_paths(&options.paths);
    if options.json {
        render_json(&affected);
    } else {
        render_text(&affected);
    }
    Ok(())
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut paths = Vec::new();
    let mut json = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--path" => {
                index += 1;
                let path = args
                    .get(index)
                    .filter(|path| !path.is_empty() && !path.starts_with("--"))
                    .ok_or_else(|| "autospec validate --path requires a path".to_string())?;
                paths.push(path.clone());
            }
            option => {
                return Err(format!(
                    "unknown autospec validate option: {option}; use bash scripts/validate.sh for execution"
                ));
            }
        }
        index += 1;
    }

    Ok(Options { paths, json })
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
