use std::path::Path;

use super::RuntimeEnvError;

#[derive(Debug)]
enum Token {
    Word { value: String, expansion: bool },
    Separator,
}

enum CommandWord {
    Assignment,
    Compose,
    Docker,
    Other,
}

pub(super) fn compose_authority(command: &str) -> Result<bool, RuntimeEnvError> {
    let tokens = tokenize(command)?;
    let mut command_position = true;
    let mut docker_command = false;
    for token in tokens {
        match token {
            Token::Separator => {
                command_position = true;
                docker_command = false;
            }
            Token::Word { value, expansion } if command_position => {
                match classify_command_word(&value, expansion)? {
                    CommandWord::Assignment => continue,
                    CommandWord::Compose => return Ok(true),
                    CommandWord::Docker => docker_command = true,
                    CommandWord::Other => docker_command = false,
                }
                command_position = false;
            }
            Token::Word { value, .. } if docker_command => {
                if value == "compose" {
                    return Ok(true);
                }
                docker_command = false;
            }
            Token::Word { .. } => {}
        }
    }
    Ok(false)
}

fn classify_command_word(value: &str, expansion: bool) -> Result<CommandWord, RuntimeEnvError> {
    if is_assignment(value) {
        return Ok(CommandWord::Assignment);
    }
    if expansion {
        return Err(RuntimeEnvError::new(
            "RUNTIME_AMBIGUOUS_COMPOSE_AUTHORITY: command-position expansion cannot be classified",
        ));
    }
    Ok(match basename(value) {
        "docker-compose" => CommandWord::Compose,
        "docker" => CommandWord::Docker,
        _ => CommandWord::Other,
    })
}

pub(super) fn join_line_continuations(source: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut continued = String::new();
    for line in source.lines() {
        continued.push_str(if continued.is_empty() {
            line
        } else {
            line.trim_start()
        });
        if continued.ends_with('\\') {
            continued.pop();
        } else {
            lines.push(std::mem::take(&mut continued));
        }
    }
    if !continued.is_empty() {
        lines.push(continued);
    }
    lines
}

fn tokenize(command: &str) -> Result<Vec<Token>, RuntimeEnvError> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut expansion = false;
    let mut chars = command.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\'' => read_quoted(&mut chars, '\'', &mut word, &mut expansion)?,
            '"' => read_quoted(&mut chars, '"', &mut word, &mut expansion)?,
            '\\' if chars.peek() == Some(&'\n') => {
                chars.next();
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    word.push(escaped);
                }
            }
            '$' => {
                expansion = true;
                word.push(character);
            }
            '#' if word.is_empty() => skip_comment(&mut chars, &mut tokens),
            '\n' => {
                flush_word(&mut tokens, &mut word, &mut expansion);
                tokens.push(Token::Separator);
            }
            character if character.is_whitespace() => {
                flush_word(&mut tokens, &mut word, &mut expansion);
            }
            ';' | '|' | '&' | '(' | ')' => {
                flush_word(&mut tokens, &mut word, &mut expansion);
                tokens.push(Token::Separator);
            }
            _ => word.push(character),
        }
    }
    flush_word(&mut tokens, &mut word, &mut expansion);
    Ok(tokens)
}

fn read_quoted<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
    quote: char,
    word: &mut String,
    expansion: &mut bool,
) -> Result<(), RuntimeEnvError> {
    for character in chars.by_ref() {
        if character == quote {
            return Ok(());
        }
        if quote == '"' && character == '$' {
            *expansion = true;
        }
        word.push(character);
    }
    Err(RuntimeEnvError::new(
        "RUNTIME_AMBIGUOUS_COMPOSE_AUTHORITY: unterminated shell quote",
    ))
}

fn skip_comment<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
    tokens: &mut Vec<Token>,
) {
    for character in chars.by_ref() {
        if character == '\n' {
            tokens.push(Token::Separator);
            break;
        }
    }
}

fn flush_word(tokens: &mut Vec<Token>, word: &mut String, expansion: &mut bool) {
    if !word.is_empty() {
        tokens.push(Token::Word {
            value: std::mem::take(word),
            expansion: std::mem::take(expansion),
        });
    }
}

fn is_assignment(value: &str) -> bool {
    value.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

fn basename(value: &str) -> &str {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value)
}
