use std::path::Path;

use super::RuntimeEnvError;

#[derive(Debug)]
enum Token {
    Word { value: String, expansion: bool },
    Redirection,
    Separator,
}

enum CommandWord {
    Assignment,
    Compose,
    Docker,
    Shell,
    Wrapper,
    Other,
}

#[derive(Default)]
struct ScanState {
    command_position: bool,
    docker_command: bool,
    docker_option_value: bool,
    shell_option: bool,
    shell_script: bool,
    redirect_target: bool,
}

pub(super) fn compose_authority(command: &str) -> Result<bool, RuntimeEnvError> {
    compose_authority_inner(command, 0)
}

fn compose_authority_inner(command: &str, depth: usize) -> Result<bool, RuntimeEnvError> {
    if depth > 8 {
        return Err(RuntimeEnvError::new(
            "RUNTIME_AMBIGUOUS_COMPOSE_AUTHORITY: shell wrapper recursion is too deep",
        ));
    }
    let mut state = ScanState {
        command_position: true,
        ..ScanState::default()
    };
    for token in tokenize(command)? {
        match token {
            Token::Separator => state.reset_command(),
            Token::Redirection if state.command_position => state.redirect_target = true,
            Token::Redirection => {}
            Token::Word { value, expansion } => {
                if state.consume_word(&value, expansion, depth)? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

impl ScanState {
    fn reset_command(&mut self) {
        *self = Self {
            command_position: true,
            ..Self::default()
        };
    }

    fn consume_word(
        &mut self,
        value: &str,
        expansion: bool,
        depth: usize,
    ) -> Result<bool, RuntimeEnvError> {
        if self.redirect_target {
            self.redirect_target = false;
            return Ok(false);
        }
        if self.shell_script {
            self.shell_script = false;
            return compose_authority_inner(value, depth + 1);
        }
        if self.shell_option {
            self.shell_option = false;
            self.shell_script = value == "-c";
            return Ok(false);
        }
        if self.docker_command {
            return Ok(self.consume_docker_word(value));
        }
        self.consume_command_word(value, expansion)
    }

    fn consume_docker_word(&mut self, value: &str) -> bool {
        if self.docker_option_value {
            self.docker_option_value = false;
            return false;
        }
        if value == "compose" {
            return true;
        }
        if docker_option_needs_value(value) {
            self.docker_option_value = true;
        } else if !value.starts_with('-') {
            self.docker_command = false;
        }
        false
    }

    fn consume_command_word(
        &mut self,
        value: &str,
        expansion: bool,
    ) -> Result<bool, RuntimeEnvError> {
        if !self.command_position {
            return Ok(false);
        }
        match classify_command_word(value, expansion)? {
            CommandWord::Assignment | CommandWord::Wrapper => {}
            CommandWord::Compose => return Ok(true),
            CommandWord::Docker => self.docker_command = true,
            CommandWord::Shell => self.shell_option = true,
            CommandWord::Other => self.command_position = false,
        }
        Ok(false)
    }
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
        "sh" | "bash" | "dash" | "zsh" => CommandWord::Shell,
        "exec" | "env" | "sudo" | "if" | "then" | "elif" | "while" | "until" | "do" | "!" => {
            CommandWord::Wrapper
        }
        _ => CommandWord::Other,
    })
}

fn docker_option_needs_value(value: &str) -> bool {
    matches!(
        value,
        "--config" | "--context" | "--host" | "--log-level" | "-H"
    )
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
            '<' | '>' => {
                flush_word(&mut tokens, &mut word, &mut expansion);
                tokens.push(Token::Redirection);
                if chars.peek() == Some(&character) {
                    chars.next();
                }
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
