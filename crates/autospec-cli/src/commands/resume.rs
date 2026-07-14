use autospec_core::execution::ExecutionQueue;

pub fn run(args: &[String]) -> Result<(), String> {
    let json = parse_options(args)?;
    let queue = ExecutionQueue::load_latest_incomplete(".")?
        .ok_or_else(|| "autospec resume found no incomplete run".to_string())?;
    let entry = queue
        .next_incomplete()
        .ok_or_else(|| "autospec resume found no incomplete queue entry".to_string())?;

    if json {
        println!(
            "{{\"command\":\"resume\",\"status\":\"ready\",\"run_id\":\"{}\",\"spec_id\":\"{}\",\"entry_status\":\"{}\"}}",
            escape_json(&queue.run_id),
            escape_json(&entry.spec_id),
            entry.status.as_str(),
        );
    } else {
        println!(
            "AutoSpec resume: local run {} is ready at {} ({})",
            queue.run_id,
            entry.spec_id,
            entry.status.as_str(),
        );
    }
    Ok(())
}

fn parse_options(args: &[String]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [json] if json == "--json" => Ok(true),
        [option, ..] => Err(format!("unknown autospec resume option: {option}")),
    }
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
