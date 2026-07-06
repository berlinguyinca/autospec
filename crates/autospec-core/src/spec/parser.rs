use super::model::{is_valid_spec_id, SpecId, SpecMetadata, SpecStatus, SpecVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    MissingRequiredField,
    MalformedDependency,
    MalformedVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub field: Option<String>,
    pub line: Option<usize>,
    pub message: String,
}

pub fn parse_spec(source: &str) -> Result<SpecMetadata, ParseError> {
    let title = parse_title(source)?;
    let version = required_section(source, "Version")
        .and_then(first_nonblank)
        .ok_or_else(|| missing("version"))?;
    let objective = required_section(source, "Objective")
        .and_then(first_nonblank)
        .ok_or_else(|| missing("objective"))?;
    let id = infer_id(&version, &title)?;
    let dependencies = parse_dependencies(source)?;
    let acceptance_criteria = parse_acceptance_criteria(source);
    let validation_command = parse_validation_command(source).unwrap_or_default();

    Ok(SpecMetadata {
        id,
        title,
        version: SpecVersion::new(version.clone()).map_err(|_| ParseError {
            kind: ParseErrorKind::MalformedVersion,
            field: Some("version".to_string()),
            line: None,
            message: format!("invalid spec version: {version}"),
        })?,
        status: SpecStatus::Ready,
        objective,
        dependencies,
        acceptance_criteria,
        validation_command,
    })
}

fn parse_title(source: &str) -> Result<String, ParseError> {
    source
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| missing("title"))
}

fn required_section(source: &str, heading: &str) -> Option<Vec<(usize, String)>> {
    section_lines(source, heading).filter(|lines| !lines.is_empty())
}

fn section_lines(source: &str, heading: &str) -> Option<Vec<(usize, String)>> {
    let mut in_section = false;
    let mut lines = Vec::new();
    let target = format!("## {heading}");

    for (index, line) in source.lines().enumerate() {
        if line.trim() == target {
            in_section = true;
            continue;
        }
        if in_section && line.starts_with("## ") {
            break;
        }
        if in_section {
            lines.push((index + 1, line.to_string()));
        }
    }

    in_section.then_some(lines)
}

fn first_nonblank(lines: Vec<(usize, String)>) -> Option<String> {
    lines
        .into_iter()
        .map(|(_, line)| line.trim().to_string())
        .find(|line| !line.is_empty())
}

fn infer_id(version: &str, title: &str) -> Result<SpecId, ParseError> {
    let number = version.strip_prefix('V').ok_or_else(|| ParseError {
        kind: ParseErrorKind::MalformedVersion,
        field: Some("version".to_string()),
        line: None,
        message: format!("invalid spec version: {version}"),
    })?;
    let slug = title
        .chars()
        .filter_map(|character| {
            if character.is_ascii_alphanumeric() {
                Some(character.to_ascii_lowercase())
            } else if character.is_whitespace() || character == '-' {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = slug.strip_suffix("-recovery").unwrap_or(&slug).to_string();
    SpecId::new(format!("v{number}-{slug}")).map_err(|value| ParseError {
        kind: ParseErrorKind::MissingRequiredField,
        field: Some("id".to_string()),
        line: None,
        message: format!("could not infer valid spec id: {value}"),
    })
}

fn parse_dependencies(source: &str) -> Result<Vec<String>, ParseError> {
    let Some(lines) = section_lines(source, "Dependencies") else {
        return Ok(Vec::new());
    };
    let mut dependencies = Vec::new();

    for (line_number, line) in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "None." {
            continue;
        }
        let Some(item) = trimmed.strip_prefix("- ") else {
            continue;
        };
        let dependency = item.trim().trim_matches('`').to_string();
        if !is_valid_spec_id(&dependency) {
            return Err(ParseError {
                kind: ParseErrorKind::MalformedDependency,
                field: Some("dependencies".to_string()),
                line: Some(line_number),
                message: format!("invalid dependency id: {dependency}"),
            });
        }
        dependencies.push(dependency);
    }

    Ok(dependencies)
}

fn parse_acceptance_criteria(source: &str) -> Vec<String> {
    let Some(lines) = section_lines(source, "Acceptance Criteria") else {
        return Vec::new();
    };

    lines
        .into_iter()
        .filter_map(|(_, line)| {
            line.trim()
                .strip_prefix("- [ ] ")
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
        })
        .collect()
}

fn parse_validation_command(source: &str) -> Option<String> {
    let lines = section_lines(source, "Validation Commands")?;
    let mut in_fence = false;
    let mut commands = Vec::new();

    for (_, line) in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence && !trimmed.is_empty() {
            commands.push(trimmed.to_string());
        }
    }

    (!commands.is_empty()).then(|| commands.join(" && "))
}

fn missing(field: &str) -> ParseError {
    ParseError {
        kind: ParseErrorKind::MissingRequiredField,
        field: Some(field.to_string()),
        line: None,
        message: format!("missing required field: {field}"),
    }
}
