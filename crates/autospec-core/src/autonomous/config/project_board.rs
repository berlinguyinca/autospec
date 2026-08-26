use std::collections::{BTreeMap, BTreeSet};

/// Configuration for GitHub Projects board ingestion.
///
/// A board is an external control surface: anyone with board write access
/// could add an item pointing at an arbitrary repository, so a configured
/// `url` without an explicit, non-empty `repo_allowlist` is rejected at
/// parse time. The two candidate lists exist because the boards autospec
/// targets disagree on both field and option names (e.g. p2's `AutoSpec
/// state` vs p1's `Delivery status`), so neither may be a hardcoded literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBoardConfig {
    pub url: Option<String>,
    pub repo_allowlist: Vec<String>,
    pub control_issue: Option<String>,
    pub write_back: bool,
    pub max_parallel_repos: u32,
    pub state_field_candidates: Vec<String>,
    pub state_option_candidates: BTreeMap<String, Vec<String>>,
}

impl Default for ProjectBoardConfig {
    fn default() -> Self {
        Self {
            url: None,
            repo_allowlist: Vec::new(),
            control_issue: None,
            write_back: false,
            max_parallel_repos: 1,
            state_field_candidates: default_state_field_candidates(),
            state_option_candidates: default_state_option_candidates(),
        }
    }
}

fn default_state_field_candidates() -> Vec<String> {
    vec!["AutoSpec state".to_string(), "Delivery status".to_string()]
}

fn default_state_option_candidates() -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    map.insert("Blocked".to_string(), vec!["Blocked".to_string()]);
    map.insert("Ready".to_string(), vec!["Ready".to_string()]);
    map.insert("Done".to_string(), vec!["Done".to_string()]);
    map.insert(
        "Implementation".to_string(),
        vec!["Implementation".to_string(), "In progress".to_string()],
    );
    map.insert(
        "Review".to_string(),
        vec!["Review".to_string(), "In review".to_string()],
    );
    map.insert(
        "Testing".to_string(),
        vec!["Testing".to_string(), "Verify".to_string()],
    );
    map
}

pub(super) fn parse(source: &str) -> Result<ProjectBoardConfig, String> {
    let mut config = ProjectBoardConfig::default();
    let mut in_project_board = false;
    let mut saw_project_board = false;
    let mut project_board_line = 0usize;
    let mut saw_url = false;
    let mut saw_repo_allowlist = false;
    let mut saw_control_issue = false;
    let mut saw_write_back = false;
    let mut saw_max_parallel_repos = false;
    let mut saw_state_field_candidates = false;
    let mut saw_state_option_candidates = false;
    let mut write_back_explicit: Option<bool> = None;
    let mut in_state_option_candidates = false;
    let mut state_option_keys: BTreeSet<String> = BTreeSet::new();

    for (index, raw_line) in source.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(raw_line).trim_end();
        if line.trim().is_empty() {
            continue;
        }

        if raw_line
            .chars()
            .take_while(|character| character.is_whitespace())
            .any(|character| character == '\t')
        {
            return Err(error(
                line_number,
                "tabs are not valid indentation in project_board",
            ));
        }

        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();

        if indent == 0 {
            in_project_board = false;
            in_state_option_candidates = false;
            if trimmed == "project_board:" {
                if saw_project_board {
                    return Err(error(line_number, "duplicate project_board block"));
                }
                saw_project_board = true;
                project_board_line = line_number;
                in_project_board = true;
                continue;
            }
            if declares_project_board(trimmed) {
                return Err(error(line_number, "project_board must be a mapping"));
            }
            continue;
        }

        if !in_project_board {
            continue;
        }

        match indent {
            2 => {
                in_state_option_candidates = false;
                let Some((key, value)) = trimmed.split_once(':') else {
                    return Err(error(
                        line_number,
                        "project_board entry must use key: value syntax",
                    ));
                };
                let key = key.trim();
                let value = value.trim();
                match key {
                    "url" => {
                        if saw_url {
                            return Err(error(line_number, "duplicate project_board.url"));
                        }
                        saw_url = true;
                        config.url = Some(scalar(value, line_number, "url")?);
                    }
                    "repo_allowlist" => {
                        if saw_repo_allowlist {
                            return Err(error(
                                line_number,
                                "duplicate project_board.repo_allowlist",
                            ));
                        }
                        saw_repo_allowlist = true;
                        config.repo_allowlist = inline_list(value, line_number, "repo_allowlist")?;
                    }
                    "control_issue" => {
                        if saw_control_issue {
                            return Err(error(
                                line_number,
                                "duplicate project_board.control_issue",
                            ));
                        }
                        saw_control_issue = true;
                        config.control_issue = Some(scalar(value, line_number, "control_issue")?);
                    }
                    "write_back" => {
                        if saw_write_back {
                            return Err(error(line_number, "duplicate project_board.write_back"));
                        }
                        saw_write_back = true;
                        write_back_explicit =
                            Some(parse_bool(value, line_number, "write_back")?);
                    }
                    "max_parallel_repos" => {
                        if saw_max_parallel_repos {
                            return Err(error(
                                line_number,
                                "duplicate project_board.max_parallel_repos",
                            ));
                        }
                        saw_max_parallel_repos = true;
                        config.max_parallel_repos =
                            parse_u32(value, line_number, "max_parallel_repos")?;
                    }
                    "state_field_candidates" => {
                        if saw_state_field_candidates {
                            return Err(error(
                                line_number,
                                "duplicate project_board.state_field_candidates",
                            ));
                        }
                        saw_state_field_candidates = true;
                        let list = inline_list(value, line_number, "state_field_candidates")?;
                        if list.is_empty() {
                            return Err(error(
                                line_number,
                                "project_board.state_field_candidates must not be an explicitly empty list",
                            ));
                        }
                        config.state_field_candidates = list;
                    }
                    "state_option_candidates" => {
                        if saw_state_option_candidates {
                            return Err(error(
                                line_number,
                                "duplicate project_board.state_option_candidates",
                            ));
                        }
                        if !value.is_empty() {
                            return Err(error(
                                line_number,
                                "project_board.state_option_candidates must be a block mapping",
                            ));
                        }
                        saw_state_option_candidates = true;
                        in_state_option_candidates = true;
                        state_option_keys.clear();
                        config.state_option_candidates = BTreeMap::new();
                    }
                    field => {
                        return Err(error(
                            line_number,
                            &format!("unknown project_board field `{field}`"),
                        ));
                    }
                }
            }
            4 if in_state_option_candidates => {
                let Some((key, value)) = trimmed.split_once(':') else {
                    return Err(error(
                        line_number,
                        "project_board.state_option_candidates entry must use key: value syntax",
                    ));
                };
                let key = key.trim();
                let value = value.trim();
                if key.is_empty() {
                    return Err(error(
                        line_number,
                        "project_board.state_option_candidates key must not be empty",
                    ));
                }
                let key = key.to_string();
                if !state_option_keys.insert(key.clone()) {
                    return Err(error(
                        line_number,
                        &format!("duplicate project_board.state_option_candidates key `{key}`"),
                    ));
                }
                let list = inline_list(value, line_number, "state_option_candidates")?;
                if list.is_empty() {
                    return Err(error(
                        line_number,
                        &format!(
                            "project_board.state_option_candidates.{key} must not be an explicitly empty list"
                        ),
                    ));
                }
                config.state_option_candidates.insert(key, list);
            }
            _ => {
                return Err(error(
                    line_number,
                    "malformed indentation or nested value in project_board",
                ));
            }
        }
    }

    if !saw_project_board {
        return Ok(config);
    }

    config.write_back = write_back_explicit.unwrap_or_else(|| config.url.is_some());

    if config.url.is_some() && config.repo_allowlist.is_empty() {
        return Err(error(
            project_board_line,
            "project_board.repo_allowlist is required when project_board.url is set",
        ));
    }

    Ok(config)
}

fn declares_project_board(value: &str) -> bool {
    value
        .split_once(':')
        .is_some_and(|(key, _)| key.trim() == "project_board")
        || value == "project_board"
}

fn inline_list(value: &str, line: usize, field: &str) -> Result<Vec<String>, String> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        return Err(error(
            line,
            &format!(
                "project_board.{field} must be an inline list, e.g. [\"a\", \"b\"]"
            ),
        ));
    }
    let inner = value[1..value.len() - 1].trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    split_inline_items(inner)
        .into_iter()
        .map(|raw| unquote_item(&raw, line, field))
        .collect()
}

fn split_inline_items(inner: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for character in inner.chars() {
        match character {
            '\'' | '"' if quote.is_none() => {
                quote = Some(character);
                current.push(character);
            }
            character if Some(character) == quote => {
                quote = None;
                current.push(character);
            }
            ',' if quote.is_none() => {
                items.push(std::mem::take(&mut current));
            }
            character => current.push(character),
        }
    }
    items.push(current);
    items
}

fn unquote_item(raw: &str, line: usize, field: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(error(
            line,
            &format!("project_board.{field} list item must not be empty"),
        ));
    }
    let unquoted = match value.as_bytes().first() {
        Some(b'\'' | b'"') => {
            let quote = value.as_bytes()[0] as char;
            if value.len() < 2 || !value.ends_with(quote) {
                return Err(error(
                    line,
                    &format!("unterminated quoted project_board.{field} list item"),
                ));
            }
            &value[1..value.len() - 1]
        }
        _ => value,
    }
    .trim();
    if unquoted.is_empty() {
        return Err(error(
            line,
            &format!("project_board.{field} list item must not be empty"),
        ));
    }
    Ok(unquoted.to_string())
}

fn scalar(value: &str, line: usize, field: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(error(
            line,
            &format!("project_board.{field} must be a non-empty scalar"),
        ));
    }
    if matches!(value.as_bytes().first(), Some(b'[' | b'{' | b'|' | b'>'))
        || value.starts_with("- ")
        || has_unquoted_mapping_delimiter(value)
    {
        return Err(error(
            line,
            &format!("project_board.{field} must be a scalar value"),
        ));
    }
    let scalar = match value.as_bytes().first() {
        Some(b'\'' | b'"') => {
            let quote = value.as_bytes()[0] as char;
            if value.len() < 2 || !value.ends_with(quote) {
                return Err(error(
                    line,
                    &format!("unterminated quoted project_board.{field}"),
                ));
            }
            &value[1..value.len() - 1]
        }
        _ => value,
    }
    .trim();
    if scalar.is_empty() {
        return Err(error(
            line,
            &format!("project_board.{field} must be a non-empty scalar"),
        ));
    }
    Ok(scalar.to_string())
}

fn parse_bool(value: &str, line: usize, field: &str) -> Result<bool, String> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(error(
            line,
            &format!("project_board.{field} must be `true` or `false`"),
        )),
    }
}

fn parse_u32(value: &str, line: usize, field: &str) -> Result<u32, String> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(
            line,
            &format!("project_board.{field} must be an unsigned decimal integer"),
        ));
    }
    value.parse::<u32>().map_err(|_| {
        error(
            line,
            &format!("project_board.{field} is outside the supported range"),
        )
    })
}

fn has_unquoted_mapping_delimiter(value: &str) -> bool {
    if matches!(value.as_bytes().first(), Some(b'\'' | b'"')) {
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

fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        } else if character == '#'
            && quote.is_none()
            && (index == 0
                || line[..index]
                    .chars()
                    .last()
                    .is_some_and(char::is_whitespace))
        {
            return &line[..index];
        }
    }
    line
}

fn error(line: usize, message: &str) -> String {
    format!("invalid .autospec/autonomous.yml at line {line}: {message}")
}
