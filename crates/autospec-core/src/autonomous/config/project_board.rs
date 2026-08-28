use std::collections::{BTreeMap, BTreeSet};

use crate::managed_project::{ManagedProjectPolicy, ProductKey, ProjectMode};

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
    pub mode: ProjectMode,
    pub url: Option<String>,
    pub repo_allowlist: Vec<String>,
    pub control_issue: Option<String>,
    pub write_back: bool,
    pub max_parallel_repos: u32,
    pub state_field_candidates: Vec<String>,
    pub state_option_candidates: BTreeMap<String, Vec<String>>,
    pub dep_field_candidates: Vec<String>,
    pub dep_markers: Vec<String>,
    pub item_limit: u32,
    pub ttl_seconds: u32,
    pub label_map: Option<String>,
    pub spend_scope: Option<String>,
    managed_policy: Option<ManagedProjectPolicy>,
}

impl ProjectBoardConfig {
    pub fn managed_policy(&self) -> Option<&ManagedProjectPolicy> {
        self.managed_policy.as_ref()
    }
}

impl Default for ProjectBoardConfig {
    fn default() -> Self {
        Self {
            mode: ProjectMode::External,
            url: None,
            repo_allowlist: Vec::new(),
            control_issue: None,
            write_back: false,
            // Matches the shell's own hardcoded AUTOSPEC_PROJECT_BOARD_PARALLEL
            // default (scripts/project-board-resolve.sh) — nothing read this
            // field before this change, so raising the default from 1 to 2
            // here does not alter any observed behavior.
            max_parallel_repos: 2,
            state_field_candidates: default_state_field_candidates(),
            state_option_candidates: default_state_option_candidates(),
            dep_field_candidates: default_dep_field_candidates(),
            dep_markers: default_dep_markers(),
            item_limit: 500,
            ttl_seconds: 300,
            label_map: None,
            spend_scope: None,
            managed_policy: None,
        }
    }
}

fn default_state_field_candidates() -> Vec<String> {
    vec!["AutoSpec state".to_string(), "Delivery status".to_string()]
}

fn default_dep_field_candidates() -> Vec<String> {
    vec!["Dependencies".to_string(), "Depends on".to_string()]
}

fn default_dep_markers() -> Vec<String> {
    vec!["Blocked by".to_string(), "Depends on".to_string()]
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
    let mut saw_mode = false;
    let mut saw_product_key = false;
    let mut saw_owner = false;
    let mut saw_repository_seeds = false;
    let mut saw_discovery_max_repos = false;
    let mut saw_url = false;
    let mut saw_repo_allowlist = false;
    let mut saw_control_issue = false;
    let mut saw_write_back = false;
    let mut saw_max_parallel_repos = false;
    let mut saw_state_field_candidates = false;
    let mut saw_state_option_candidates = false;
    let mut saw_dep_field_candidates = false;
    let mut saw_dep_markers = false;
    let mut saw_item_limit = false;
    let mut saw_ttl = false;
    let mut saw_label_map = false;
    let mut saw_spend_scope = false;
    let mut write_back_explicit: Option<bool> = None;
    let mut in_state_option_candidates = false;
    let mut state_option_keys: BTreeSet<String> = BTreeSet::new();
    let mut product_key: Option<ProductKey> = None;
    let mut owner: Option<String> = None;
    let mut repository_seeds: Option<Vec<String>> = None;
    let mut discovery_max_repos = 100usize;

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
                    "mode" => {
                        if saw_mode {
                            return Err(error(line_number, "duplicate project_board.mode"));
                        }
                        saw_mode = true;
                        config.mode = match scalar(value, line_number, "mode")?.as_str() {
                            "managed" => ProjectMode::Managed,
                            "external" => ProjectMode::External,
                            _ => {
                                return Err(error(
                                    line_number,
                                    "project_board.mode must be `managed` or `external`",
                                ));
                            }
                        };
                    }
                    "product_key" => {
                        if saw_product_key {
                            return Err(error(line_number, "duplicate project_board.product_key"));
                        }
                        saw_product_key = true;
                        let value = scalar(value, line_number, "product_key")?;
                        product_key = Some(ProductKey::new(value).map_err(|message| {
                            error(
                                line_number,
                                &format!("invalid project_board.product_key: {message}"),
                            )
                        })?);
                    }
                    "owner" => {
                        if saw_owner {
                            return Err(error(line_number, "duplicate project_board.owner"));
                        }
                        saw_owner = true;
                        owner = Some(scalar(value, line_number, "owner")?);
                    }
                    "repository_seeds" => {
                        if saw_repository_seeds {
                            return Err(error(
                                line_number,
                                "duplicate project_board.repository_seeds",
                            ));
                        }
                        saw_repository_seeds = true;
                        repository_seeds =
                            Some(inline_list(value, line_number, "repository_seeds")?);
                    }
                    "discovery_max_repos" => {
                        if saw_discovery_max_repos {
                            return Err(error(
                                line_number,
                                "duplicate project_board.discovery_max_repos",
                            ));
                        }
                        saw_discovery_max_repos = true;
                        discovery_max_repos =
                            parse_usize(value, line_number, "discovery_max_repos")?;
                        if discovery_max_repos == 0 {
                            return Err(error(
                                line_number,
                                "project_board.discovery_max_repos must be greater than zero",
                            ));
                        }
                    }
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
                        write_back_explicit = Some(parse_bool(value, line_number, "write_back")?);
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
                    "dep_field_candidates" => {
                        if saw_dep_field_candidates {
                            return Err(error(
                                line_number,
                                "duplicate project_board.dep_field_candidates",
                            ));
                        }
                        saw_dep_field_candidates = true;
                        let list = inline_list(value, line_number, "dep_field_candidates")?;
                        if list.is_empty() {
                            return Err(error(
                                line_number,
                                "project_board.dep_field_candidates must not be an explicitly empty list",
                            ));
                        }
                        config.dep_field_candidates = list;
                    }
                    "dep_markers" => {
                        if saw_dep_markers {
                            return Err(error(line_number, "duplicate project_board.dep_markers"));
                        }
                        saw_dep_markers = true;
                        let list = inline_list(value, line_number, "dep_markers")?;
                        if list.is_empty() {
                            return Err(error(
                                line_number,
                                "project_board.dep_markers must not be an explicitly empty list",
                            ));
                        }
                        config.dep_markers = list;
                    }
                    "item_limit" => {
                        if saw_item_limit {
                            return Err(error(line_number, "duplicate project_board.item_limit"));
                        }
                        saw_item_limit = true;
                        config.item_limit = parse_u32(value, line_number, "item_limit")?;
                    }
                    "ttl" => {
                        if saw_ttl {
                            return Err(error(line_number, "duplicate project_board.ttl"));
                        }
                        saw_ttl = true;
                        config.ttl_seconds = parse_u32(value, line_number, "ttl")?;
                    }
                    "label_map" => {
                        if saw_label_map {
                            return Err(error(line_number, "duplicate project_board.label_map"));
                        }
                        saw_label_map = true;
                        config.label_map = Some(scalar(value, line_number, "label_map")?);
                    }
                    "spend_scope" => {
                        if saw_spend_scope {
                            return Err(error(line_number, "duplicate project_board.spend_scope"));
                        }
                        saw_spend_scope = true;
                        let scope = scalar(value, line_number, "spend_scope")?;
                        validate_spend_scope(&scope, line_number)?;
                        config.spend_scope = Some(scope);
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

    if config.mode == ProjectMode::Managed {
        let product_key = product_key.ok_or_else(|| {
            error(
                project_board_line,
                "project_board.product_key is required in managed mode",
            )
        })?;
        let owner = owner.ok_or_else(|| {
            error(
                project_board_line,
                "project_board.owner is required in managed mode",
            )
        })?;
        let repository_seeds = repository_seeds
            .filter(|seeds| !seeds.is_empty())
            .ok_or_else(|| {
                error(
                    project_board_line,
                    "project_board.repository_seeds must not be empty in managed mode",
                )
            })?;
        config.managed_policy = Some(ManagedProjectPolicy {
            product_key,
            owner,
            repository_seeds,
            repo_allowlist: config.repo_allowlist.clone(),
            discovery_max_repos,
        });
    }

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
            &format!("project_board.{field} must be an inline list, e.g. [\"a\", \"b\"]"),
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

fn parse_usize(value: &str, line: usize, field: &str) -> Result<usize, String> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(
            line,
            &format!("project_board.{field} must be an unsigned decimal integer"),
        ));
    }
    value.parse::<usize>().map_err(|_| {
        error(
            line,
            &format!("project_board.{field} is outside the supported range"),
        )
    })
}

// Mirrors scripts/autonomous-spend-ledger.sh's validate_scope(): a
// spend_scope becomes a ledger directory name verbatim, so it is validated
// against an allowlist charset (never a denylist) at parse time — the same
// gate the shell enforces at runtime, moved earlier so a bad value is
// rejected before it ever reaches the shell.
const SPEND_SCOPE_MAX_LEN: usize = 200;

fn validate_spend_scope(value: &str, line: usize) -> Result<(), String> {
    if value.is_empty() {
        return Err(error(line, "project_board.spend_scope must not be empty"));
    }
    if value == "." || value == ".." {
        return Err(error(
            line,
            "project_board.spend_scope must not be '.' or '..'",
        ));
    }
    if value.starts_with('-') {
        return Err(error(
            line,
            "project_board.spend_scope must not start with '-'",
        ));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(error(
            line,
            "project_board.spend_scope must contain only [A-Za-z0-9._-]",
        ));
    }
    if value.len() > SPEND_SCOPE_MAX_LEN {
        return Err(error(
            line,
            &format!("project_board.spend_scope must be {SPEND_SCOPE_MAX_LEN} characters or fewer"),
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use crate::{autonomous::config::AutonomousConfig, managed_project::ProjectMode};

    #[test]
    fn project_board_parses_managed_policy() {
        let config = AutonomousConfig::parse(
            r#"
project_board:
  mode: managed
  product_key: autospec
  owner: berlinguyinca
  repo_allowlist: ["berlinguyinca/autospec", "berlinguyinca/autospec-*" ]
  repository_seeds: ["berlinguyinca/autospec"]
  discovery_max_repos: 25
  write_back: true
"#,
        )
        .unwrap();

        let policy = config.project_board.managed_policy().unwrap();
        assert_eq!(config.project_board.mode, ProjectMode::Managed);
        assert_eq!(policy.product_key.as_str(), "autospec");
        assert_eq!(policy.owner, "berlinguyinca");
        assert_eq!(policy.repository_seeds, ["berlinguyinca/autospec"]);
        assert_eq!(policy.discovery_max_repos, 25);
    }

    #[test]
    fn project_board_managed_mode_rejects_missing_or_invalid_identity_fields() {
        let invalid = [
            (
                "missing product key",
                "project_board:\n  mode: managed\n  owner: berlinguyinca\n  repository_seeds: [\"berlinguyinca/autospec\"]\n",
            ),
            (
                "missing owner",
                "project_board:\n  mode: managed\n  product_key: autospec\n  repository_seeds: [\"berlinguyinca/autospec\"]\n",
            ),
            (
                "empty seeds",
                "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repository_seeds: []\n",
            ),
            (
                "invalid product key",
                "project_board:\n  mode: managed\n  product_key: ../autospec\n  owner: berlinguyinca\n  repository_seeds: [\"berlinguyinca/autospec\"]\n",
            ),
            (
                "zero discovery limit",
                "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repository_seeds: [\"berlinguyinca/autospec\"]\n  discovery_max_repos: 0\n",
            ),
        ];

        for (name, source) in invalid {
            let error = AutonomousConfig::parse(source).expect_err(name);
            assert!(error.contains("line "), "{name}: {error}");
        }
    }

    #[test]
    fn project_board_url_only_configuration_defaults_to_external_mode() {
        let config = AutonomousConfig::parse(
            "project_board:\n  url: https://github.com/orgs/InferWeave/projects/2\n  repo_allowlist: [\"InferWeave/*\"]\n",
        )
        .unwrap();

        assert_eq!(config.project_board.mode, ProjectMode::External);
        assert!(config.project_board.managed_policy().is_none());
    }
}
