use super::*;

pub(super) fn parse_primary_smoke(issue_body: &str) -> Result<DirectCommandPlan, String> {
    let line = first_fenced_command_under(issue_body, "primary smoke test (inner loop)")?;
    parse_direct_command_plan(line)
}

fn first_fenced_command_under<'a>(body: &'a str, heading: &str) -> Result<&'a str, String> {
    let lines = body.lines().collect::<Vec<_>>();
    let heading_index = lines
        .iter()
        .position(|line| normalized_level_three_heading(line).as_deref() == Some(heading))
        .ok_or_else(|| format!("executor issue is missing the {heading} heading"))?;
    let section_end = lines[heading_index + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with("###"))
        .map_or(lines.len(), |offset| heading_index + 1 + offset);
    let section = &lines[heading_index + 1..section_end];
    let fence = section
        .iter()
        .position(|line| line.trim_start().starts_with("```"))
        .ok_or_else(|| format!("executor {heading} requires a fenced command"))?;
    let fence_end = section[fence + 1..]
        .iter()
        .position(|line| line.trim() == "```")
        .map(|offset| fence + 1 + offset)
        .ok_or_else(|| format!("executor {heading} fence is unterminated"))?;
    let commands = section[fence + 1..fence_end]
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    if commands.len() != 1 {
        return Err(format!(
            "executor {heading} requires exactly one non-comment command line"
        ));
    }
    Ok(commands[0])
}

pub(super) fn normalized_level_three_heading(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let hash_count = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if hash_count != 3 {
        return None;
    }
    let content = trimmed.get(hash_count..)?;
    if !content.starts_with(char::is_whitespace) {
        return None;
    }
    Some(
        content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase(),
    )
}

pub(super) fn parse_direct_command_plan(line: &str) -> Result<DirectCommandPlan, String> {
    if line.is_empty()
        || line.len() > MAX_DIRECT_COMMAND_LINE
        || line.contains('\n')
        || line.contains('\r')
        || line.contains('\0')
    {
        return Err("executor direct command line is empty, multiline, or oversized".to_string());
    }
    if line.contains("$(") || line.contains('`') {
        return Err("executor direct command rejects command substitution".to_string());
    }

    let mut segments = Vec::<Vec<String>>::new();
    let mut argv = Vec::<String>::new();
    let mut token = String::new();
    let mut token_started = false;
    let mut quote = None;
    let mut escaped = false;
    let characters = line.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if escaped {
            token.push(character);
            token_started = true;
            escaped = false;
            index += 1;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            token_started = true;
            escaped = true;
            index += 1;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                token.push(character);
            }
            index += 1;
            continue;
        }
        if matches!(character, '\'' | '"') {
            token_started = true;
            quote = Some(character);
            index += 1;
            continue;
        }
        if character == '&' && characters.get(index + 1) == Some(&'&') {
            finish_direct_token(&mut argv, &mut token, &mut token_started)?;
            if argv.is_empty() {
                return Err("executor direct command contains an empty segment".to_string());
            }
            segments.push(std::mem::take(&mut argv));
            index += 2;
            continue;
        }
        if matches!(character, '|' | '<' | '>' | ';' | '&') {
            return Err(format!(
                "executor direct command rejects shell operator {character}"
            ));
        }
        if character.is_whitespace() {
            finish_direct_token(&mut argv, &mut token, &mut token_started)?;
        } else {
            token.push(character);
            token_started = true;
        }
        index += 1;
    }
    if escaped || quote.is_some() {
        return Err("executor direct command has unterminated quoting or escaping".to_string());
    }
    finish_direct_token(&mut argv, &mut token, &mut token_started)?;
    if argv.is_empty() {
        return Err("executor direct command contains an empty segment".to_string());
    }
    segments.push(argv);
    if segments.len() > MAX_DIRECT_COMMAND_SEGMENTS {
        return Err("executor direct command has too many sequential segments".to_string());
    }
    const CONTROL_WORDS: [&str; 42] = [
        ".", "[", "[[", "]", "]]", "{", "}", "break", "case", "cd", "command", "continue",
        "coproc", "do", "done", "elif", "else", "esac", "eval", "exec", "exit", "export", "fi",
        "for", "function", "if", "readonly", "return", "select", "set", "shift", "source", "then",
        "time", "times", "trap", "umask", "unset", "until", "wait", "while", "in",
    ];
    let commands = segments
        .into_iter()
        .map(|argv| {
            if argv.len() > MAX_DIRECT_COMMAND_ARGS {
                return Err("executor direct command has too many arguments".to_string());
            }
            if CONTROL_WORDS.contains(&argv[0].as_str()) {
                return Err(format!(
                    "executor direct command rejects shell control builtin {}",
                    argv[0]
                ));
            }
            Ok(DirectCommand::success(argv))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(DirectCommandPlan { commands })
}

fn finish_direct_token(
    argv: &mut Vec<String>,
    token: &mut String,
    token_started: &mut bool,
) -> Result<(), String> {
    if !*token_started {
        return Ok(());
    }
    if token.len() > MAX_DIRECT_ARGUMENT_LENGTH {
        return Err("executor direct command argument is oversized".to_string());
    }
    argv.push(std::mem::take(token));
    *token_started = false;
    Ok(())
}
