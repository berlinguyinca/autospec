use autospec_core::runtime_policy::classify_path;

mod audit;

pub fn run(args: &[String]) -> Result<(), String> {
    match args {
        [command, path] if command == "classify" => {
            print_classification(path, false);
            Ok(())
        }
        [command, path, flag] if command == "classify" && flag == "--json" => {
            print_classification(path, true);
            Ok(())
        }
        [command, rest @ ..] if command == "audit" => audit::run(rest),
        [flag] if flag == "--help" || flag == "-h" => {
            print_help();
            Ok(())
        }
        [] => Err("autospec runtime requires a subcommand".to_string()),
        [command, ..] => Err(format!("unknown autospec runtime command: {command}")),
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
        "autospec runtime\n\nUSAGE:\n    autospec runtime classify <PATH> [--json]\n    autospec runtime audit [--root PATH] [--json]\n\nCOMMANDS:\n    classify       Classify a repository path by runtime ownership policy\n    audit          List platform files grouped by runtime migration class"
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
