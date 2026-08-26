use std::collections::BTreeSet;

use crate::autonomous::waterfall::sha256_hex;

mod project_board;
mod tier4;

pub use project_board::ProjectBoardConfig;
pub use tier4::{Tier4Config, Tier4SourceDescriptor};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutonomousConfig {
    pub main_health: MainHealthConfig,
    pub tier4: Tier4Config,
    pub project_board: ProjectBoardConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MainHealthConfig {
    pub branch: Option<String>,
    pub ignore_checks: BTreeSet<String>,
}

impl MainHealthConfig {
    pub fn effective_policy_digest(&self, resolved_branch: &str) -> Result<String, String> {
        if resolved_branch.trim().is_empty() {
            return Err("effective main-health branch must not be empty".to_string());
        }

        let mut identity = "autospec-main-health-policy-v1\n".to_string();
        append_identity_field(&mut identity, "branch", resolved_branch);
        for check in &self.ignore_checks {
            append_identity_field(&mut identity, "ignore_check", check);
        }
        Ok(format!(
            "autospec-main-health-policy-v1:{}",
            sha256_hex(identity.as_bytes())
        ))
    }
}

impl AutonomousConfig {
    /// Parses the intentionally small repository-owned autonomous schema.
    ///
    /// This is not a generic YAML parser: only the `main_health` and `tier4`
    /// blocks are interpreted. Unrelated top-level policy is ignored so future,
    /// separate Rust migrations do not make either typed policy reject it.
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut config = Self::default();
        let mut in_main_health = false;
        let mut saw_main_health = false;
        let mut saw_branch = false;
        let mut saw_ignore_checks = false;
        let mut list_open = false;
        let mut ignoring_unrelated_block = false;

        for (index, raw_line) in source.lines().enumerate() {
            let line_number = index + 1;
            let line = strip_comment(raw_line).trim_end();
            if line.trim().is_empty() {
                continue;
            }

            let leading_whitespace = raw_line
                .chars()
                .take_while(|character| character.is_whitespace())
                .collect::<String>();
            if leading_whitespace.contains('\t') && in_main_health {
                return Err(error(
                    line_number,
                    "tabs are not valid indentation in main_health",
                ));
            }

            let indent = line.len() - line.trim_start().len();
            let trimmed = line.trim_start();
            if indent != 0 && !ignoring_unrelated_block && declares_main_health(trimmed) {
                return Err(error(
                    line_number,
                    "main_health must be a top-level mapping",
                ));
            }
            if indent == 0 {
                in_main_health = false;
                list_open = false;

                let Some((key, value)) = trimmed.split_once(':') else {
                    if trimmed == "main_health" {
                        return Err(error(line_number, "main_health must be a mapping"));
                    }
                    ignoring_unrelated_block = true;
                    continue;
                };
                if key.trim() != "main_health" {
                    ignoring_unrelated_block = true;
                    continue;
                }
                if saw_main_health {
                    return Err(error(line_number, "duplicate main_health block"));
                }
                if !value.trim().is_empty() {
                    return Err(error(line_number, "main_health must be a mapping"));
                }
                saw_main_health = true;
                in_main_health = true;
                ignoring_unrelated_block = false;
                continue;
            }

            if !in_main_health {
                continue;
            }

            match indent {
                2 => {
                    list_open = false;
                    let Some((key, value)) = trimmed.split_once(':') else {
                        return Err(error(
                            line_number,
                            "main_health entry must use key: value syntax",
                        ));
                    };
                    match key.trim() {
                        "branch" => {
                            if saw_branch {
                                return Err(error(line_number, "duplicate main_health.branch"));
                            }
                            saw_branch = true;
                            config.main_health.branch = Some(parse_scalar(value, line_number)?);
                        }
                        "ignore_checks" => {
                            if saw_ignore_checks {
                                return Err(error(
                                    line_number,
                                    "duplicate main_health.ignore_checks",
                                ));
                            }
                            if !value.trim().is_empty() {
                                return Err(error(
                                    line_number,
                                    "main_health.ignore_checks must be a block list",
                                ));
                            }
                            saw_ignore_checks = true;
                            list_open = true;
                        }
                        field => {
                            return Err(error(
                                line_number,
                                &format!("unknown main_health field `{field}`"),
                            ));
                        }
                    }
                }
                4 if list_open => {
                    let Some(value) = trimmed.strip_prefix('-') else {
                        return Err(error(
                            line_number,
                            "main_health.ignore_checks entries must start with -",
                        ));
                    };
                    if value.is_empty() || !value.starts_with(char::is_whitespace) {
                        return Err(error(
                            line_number,
                            "main_health.ignore_checks entries must be scalar values",
                        ));
                    }
                    let value = value.trim();
                    if value.starts_with('-') || has_unquoted_mapping_delimiter(value) {
                        return Err(error(
                            line_number,
                            "main_health.ignore_checks entries must be scalar values",
                        ));
                    }
                    let value = parse_scalar(value, line_number)?;
                    if !config.main_health.ignore_checks.insert(value.clone()) {
                        return Err(error(
                            line_number,
                            &format!("duplicate main_health.ignore_checks value `{value}`"),
                        ));
                    }
                }
                _ => {
                    return Err(error(
                        line_number,
                        "malformed indentation or nested value in main_health",
                    ));
                }
            }
        }

        config.tier4 = tier4::parse(source)?;
        config.project_board = project_board::parse(source)?;
        Ok(config)
    }
}

fn declares_main_health(value: &str) -> bool {
    value
        .split_once(':')
        .is_some_and(|(key, _)| key.trim() == "main_health")
        || value == "main_health"
}

fn has_unquoted_mapping_delimiter(value: &str) -> bool {
    if matches!(value.as_bytes().first(), Some(b'\'' | b'\"')) {
        return false;
    }
    value.char_indices().any(|(index, character)| {
        character == ':'
            && value[index + character.len_utf8()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
    })
}

fn parse_scalar(value: &str, line_number: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(error(
            line_number,
            "main_health scalar values must not be empty",
        ));
    }
    if matches!(value.as_bytes().first(), Some(b'[' | b'{' | b'|' | b'>')) {
        return Err(error(
            line_number,
            "main_health values must be scalar strings, not collections or blocks",
        ));
    }

    if value.starts_with("- ") || has_unquoted_mapping_delimiter(value) {
        return Err(error(
            line_number,
            "main_health values must be scalar strings, not nested collections or mappings",
        ));
    }

    let unquoted = match value.as_bytes().first() {
        Some(b'\'' | b'\"') => {
            let quote = value.as_bytes()[0] as char;
            if value.len() < 2 || !value.ends_with(quote) {
                return Err(error(line_number, "unterminated quoted main_health value"));
            }
            &value[1..value.len() - 1]
        }
        _ => value,
    };
    let scalar = unquoted.trim();
    if scalar.is_empty() {
        return Err(error(
            line_number,
            "main_health scalar values must not be empty",
        ));
    }
    Ok(scalar.to_string())
}

fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('\"') {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '\"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        } else if character == '#' && quote.is_none() {
            return &line[..index];
        }
    }
    line
}

fn error(line_number: usize, message: &str) -> String {
    format!("invalid .autospec/autonomous.yml at line {line_number}: {message}")
}

fn append_identity_field(document: &mut String, name: &str, value: &str) {
    document.push_str(&format!("{name}:{}:{value}\n", value.len()));
}
