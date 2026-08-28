use super::{ManagedProjectError, ManagedProjectPolicy, RemoteProject, PROJECT_FETCH_LIMIT};
use serde_json::Value;
use std::collections::HashSet;

const MARKER_BEGIN: &str = "<!-- autospec-managed-project:begin -->";
const MARKER_END: &str = "<!-- autospec-managed-project:end -->";

pub(super) fn verify_managed_marker(
    readme: &str,
    policy: &ManagedProjectPolicy,
) -> Result<bool, ManagedProjectError> {
    let Some(marker) = parse_marker(readme)? else {
        return Ok(false);
    };
    if marker.product_key == policy.product_key.as_str() && marker.owner != policy.owner {
        return Err(ManagedProjectError::new(format!(
            "managed GitHub Project marker owner {} conflicts with approved owner {}",
            marker.owner, policy.owner
        )));
    }
    Ok(marker.product_key == policy.product_key.as_str() && marker.owner == policy.owner)
}

pub(super) fn append_marker(
    readme: &str,
    policy: &ManagedProjectPolicy,
) -> Result<String, ManagedProjectError> {
    if parse_marker(readme)?.is_some() {
        return Err(ManagedProjectError::new(
            "new GitHub Project already contains a managed marker",
        ));
    }
    let marker = format!(
        "{MARKER_BEGIN}\nschema: 1\nproduct-key: {}\nowner: {}\n{MARKER_END}",
        policy.product_key.as_str(),
        policy.owner
    );
    if readme.is_empty() {
        Ok(marker)
    } else {
        Ok(format!("{readme}\n\n{marker}"))
    }
}

pub(super) fn parse_project_numbers(output: &str) -> Result<Vec<u64>, ManagedProjectError> {
    let value: Value = serde_json::from_str(output).map_err(json_error)?;
    let projects = value
        .get("projects")
        .and_then(Value::as_array)
        .ok_or_else(|| ManagedProjectError::new("GitHub Project list has no projects array"))?;
    if projects.len() >= PROJECT_FETCH_LIMIT {
        return Err(ManagedProjectError::new(
            "GitHub Project discovery may be truncated at the transport limit",
        ));
    }
    projects
        .iter()
        .map(|project| {
            project
                .get("number")
                .and_then(Value::as_u64)
                .filter(|number| *number > 0)
                .ok_or_else(|| ManagedProjectError::new("GitHub Project has invalid number"))
        })
        .collect()
}

pub(super) fn parse_project(output: &str) -> Result<RemoteProject, ManagedProjectError> {
    let value: Value = serde_json::from_str(output).map_err(json_error)?;
    let string = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| ManagedProjectError::new(format!("GitHub Project has invalid {field}")))
    };
    let owner = value
        .pointer("/owner/login")
        .or_else(|| value.get("owner"))
        .and_then(Value::as_str)
        .filter(|owner| !owner.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ManagedProjectError::new("GitHub Project has invalid owner"))?;
    let readme = value
        .get("readme")
        .and_then(Value::as_str)
        .ok_or_else(|| ManagedProjectError::new("GitHub Project has invalid readme"))?
        .to_owned();
    Ok(RemoteProject {
        node_id: string("id")?,
        number: value
            .get("number")
            .and_then(Value::as_u64)
            .filter(|number| *number > 0)
            .ok_or_else(|| ManagedProjectError::new("GitHub Project has invalid number"))?,
        url: string("url")?,
        title: string("title")?,
        owner,
        readme,
    })
}

pub(super) fn parse_project_items(output: &str) -> Result<HashSet<String>, ManagedProjectError> {
    let value: Value = serde_json::from_str(output).map_err(json_error)?;
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| ManagedProjectError::new("GitHub Project item list has no items array"))?;
    if items.len() >= PROJECT_FETCH_LIMIT {
        return Err(ManagedProjectError::new(
            "GitHub Project item list may be truncated at the transport limit",
        ));
    }
    let mut issues = HashSet::new();
    for item in items {
        let content = item
            .get("content")
            .and_then(Value::as_object)
            .ok_or_else(|| ManagedProjectError::new("GitHub Project item has invalid content"))?;
        let item_type = content
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| ManagedProjectError::new("GitHub Project item has invalid type"))?;
        match item_type {
            "Issue" => {
                let url = content
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ManagedProjectError::new("GitHub issue item has invalid URL"))?;
                issues.insert(normalize_issue_url(url)?);
            }
            "PullRequest" | "DraftIssue" | "RedactedItem" => {}
            _ => {
                return Err(ManagedProjectError::new(format!(
                    "GitHub Project item has unknown type {item_type}"
                )))
            }
        }
    }
    Ok(issues)
}

pub(super) fn normalize_issue_url(url: &str) -> Result<String, ManagedProjectError> {
    let normalized = url
        .trim()
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_ascii_lowercase();
    let path = normalized
        .strip_prefix("https://github.com/")
        .ok_or_else(|| ManagedProjectError::new("issue URL must use https://github.com"))?;
    let parts = path.split('/').collect::<Vec<_>>();
    let number = parts
        .get(3)
        .filter(|_| parts.len() == 4 && !parts[0].is_empty() && !parts[1].is_empty())
        .filter(|_| parts[2] == "issues")
        .and_then(|number| number.parse::<u64>().ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| ManagedProjectError::new("issue URL must identify one GitHub issue"))?;
    Ok(format!(
        "https://github.com/{}/{}/issues/{number}",
        parts[0], parts[1]
    ))
}

struct Marker<'a> {
    product_key: &'a str,
    owner: &'a str,
}

fn parse_marker(readme: &str) -> Result<Option<Marker<'_>>, ManagedProjectError> {
    let starts = readme.match_indices(MARKER_BEGIN).collect::<Vec<_>>();
    let ends = readme.match_indices(MARKER_END).collect::<Vec<_>>();
    if starts.is_empty() && ends.is_empty() {
        return Ok(None);
    }
    if starts.len() != 1 || ends.len() != 1 || starts[0].0 >= ends[0].0 {
        return Err(ManagedProjectError::new(
            "GitHub Project managed marker must contain exactly one complete block",
        ));
    }
    let payload_start = starts[0].0 + MARKER_BEGIN.len();
    let payload = readme[payload_start..ends[0].0].trim_matches(['\r', '\n']);
    let lines = payload.lines().collect::<Vec<_>>();
    if lines.len() != 3 || lines[0] != "schema: 1" {
        return Err(ManagedProjectError::new(
            "GitHub Project managed marker has unsupported schema or shape",
        ));
    }
    let product_key = lines[1].strip_prefix("product-key: ").ok_or_else(|| {
        ManagedProjectError::new("GitHub Project managed marker has invalid product key")
    })?;
    let owner = lines[2].strip_prefix("owner: ").ok_or_else(|| {
        ManagedProjectError::new("GitHub Project managed marker has invalid owner")
    })?;
    if product_key.is_empty() || owner.is_empty() {
        return Err(ManagedProjectError::new(
            "GitHub Project managed marker identity must not be empty",
        ));
    }
    Ok(Some(Marker { product_key, owner }))
}

fn json_error(error: serde_json::Error) -> ManagedProjectError {
    ManagedProjectError::new(format!("invalid GitHub Project response: {error}"))
}
