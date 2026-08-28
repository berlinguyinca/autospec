use super::{ManagedProjectError, ManagedProjectStore, RemoteProject};
use crate::commands::autonomous::accountability::github::{
    GithubCommand, GithubFailure, GithubTransport,
};
use autospec_core::managed_project::ManagedProjectPolicy;
use serde_json::Value;
use std::collections::HashSet;

const MARKER_BEGIN: &str = "<!-- autospec-managed-project:begin -->";
const MARKER_END: &str = "<!-- autospec-managed-project:end -->";
const PROJECT_FETCH_LIMIT: usize = 500;

pub fn verify_managed_marker(
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

pub fn resolve_or_create_project<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
    title: &str,
) -> Result<RemoteProject, ManagedProjectError> {
    validate_policy(policy)?;
    if let Some(number) = store.snapshot().project_number {
        let owner = store
            .snapshot()
            .owner
            .as_deref()
            .ok_or_else(|| ManagedProjectError::new("managed project binding has no owner"))?;
        if owner != policy.owner {
            return Err(ManagedProjectError::new(
                "managed project binding owner conflicts with policy",
            ));
        }
        let project = view_project(github, &policy.owner, number)?;
        validate_project(&project, policy)?;
        if store.snapshot().project_node_id.as_deref() != Some(project.node_id.as_str()) {
            return Err(ManagedProjectError::new(
                "verified remote project node ID conflicts with local binding",
            ));
        }
        return Ok(project);
    }

    let output = execute(
        github,
        GithubCommand::ListProjects {
            owner: policy.owner.clone(),
        },
        "cannot list GitHub Projects",
    )?;
    let numbers = parse_project_numbers(&output)?;
    let mut matches = Vec::new();
    for number in numbers {
        let project = view_project(github, &policy.owner, number)?;
        validate_returned_owner(&project, &policy.owner)?;
        if verify_managed_marker(&project.readme, policy)? {
            matches.push(project);
        }
    }
    match matches.len() {
        1 => {
            let project = matches.pop().expect("one verified match");
            persist_project(store, &project)?;
            Ok(project)
        }
        count if count > 1 => Err(ManagedProjectError::new(format!(
            "multiple GitHub Projects have the managed marker for {}",
            policy.product_key.as_str()
        ))),
        _ => create_project(store, github, policy, title),
    }
}

pub fn reconcile_issue<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
    issue_url: &str,
) -> Result<(), ManagedProjectError> {
    let identity = bound_identity(store, policy)?;
    let normalized = normalize_issue_url(issue_url)?;
    let items = list_project_items(github, &identity.owner, identity.number)?;
    let projection = projection_key(&identity.node_id, &normalized);
    if items.contains(&normalized) {
        if store
            .snapshot()
            .pending_projections
            .iter()
            .any(|pending| pending == &projection)
        {
            store.ack_projection(&projection)?;
        }
        return Ok(());
    }
    store.enqueue_projection(projection.clone())?;
    execute(
        github,
        GithubCommand::AddToProject {
            owner: identity.owner,
            project_number: identity.number,
            issue_url: normalized,
        },
        "cannot add issue to managed GitHub Project",
    )?;
    store.ack_projection(&projection)
}

pub fn retry_pending_projections<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
) -> Result<(), ManagedProjectError> {
    let identity = bound_identity(store, policy)?;
    let prefix = format!("project:item-add:{}:", identity.node_id);
    let pending = store
        .snapshot()
        .pending_projections
        .iter()
        .filter_map(|projection| {
            projection
                .strip_prefix(&prefix)
                .map(|url| (projection.clone(), url.to_owned()))
        })
        .collect::<Vec<_>>();
    if pending.is_empty() {
        return Ok(());
    }
    let items = list_project_items(github, &identity.owner, identity.number)?;
    for (projection, issue_url) in pending {
        let issue_url = normalize_issue_url(&issue_url)?;
        if !items.contains(&issue_url) {
            execute(
                github,
                GithubCommand::AddToProject {
                    owner: identity.owner.clone(),
                    project_number: identity.number,
                    issue_url,
                },
                "cannot retry managed GitHub Project item addition",
            )?;
        }
        store.ack_projection(&projection)?;
    }
    Ok(())
}

fn create_project<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
    title: &str,
) -> Result<RemoteProject, ManagedProjectError> {
    if title.trim().is_empty() {
        return Err(ManagedProjectError::new(
            "managed GitHub Project title must not be empty",
        ));
    }
    let projection = format!("project:create:{}", policy.product_key.as_str());
    store.enqueue_projection(projection.clone())?;
    let created = parse_project(&execute(
        github,
        GithubCommand::CreateProject {
            owner: policy.owner.clone(),
            title: title.to_owned(),
        },
        "cannot create managed GitHub Project",
    )?)?;
    validate_returned_owner(&created, &policy.owner)?;
    let before_edit = view_project(github, &policy.owner, created.number)?;
    if before_edit.node_id != created.node_id {
        return Err(ManagedProjectError::new(
            "created GitHub Project changed identity before marker update",
        ));
    }
    let readme = append_marker(&before_edit.readme, policy)?;
    execute(
        github,
        GithubCommand::EditProjectMarker {
            owner: policy.owner.clone(),
            number: created.number,
            readme,
        },
        "cannot write managed GitHub Project marker",
    )?;
    let verified = view_project(github, &policy.owner, created.number)?;
    if verified.node_id != created.node_id || verified.number != created.number {
        return Err(ManagedProjectError::new(
            "GitHub returned a different Project after marker update",
        ));
    }
    validate_project(&verified, policy)?;
    persist_project(store, &verified)?;
    store.ack_projection(&projection)?;
    Ok(verified)
}

fn validate_project(
    project: &RemoteProject,
    policy: &ManagedProjectPolicy,
) -> Result<(), ManagedProjectError> {
    validate_returned_owner(project, &policy.owner)?;
    if !verify_managed_marker(&project.readme, policy)? {
        return Err(ManagedProjectError::new(
            "GitHub Project does not contain the expected managed marker",
        ));
    }
    Ok(())
}

fn validate_returned_owner(
    project: &RemoteProject,
    expected: &str,
) -> Result<(), ManagedProjectError> {
    if project.owner != expected {
        return Err(ManagedProjectError::new(format!(
            "GitHub Project owner {} does not match approved owner {expected}",
            project.owner
        )));
    }
    Ok(())
}

fn persist_project(
    store: &mut ManagedProjectStore,
    project: &RemoteProject,
) -> Result<(), ManagedProjectError> {
    store.record_project(
        &project.owner,
        &project.node_id,
        project.number,
        &project.url,
        &project.title,
    )
}

fn view_project<T: GithubTransport>(
    github: &mut T,
    owner: &str,
    number: u64,
) -> Result<RemoteProject, ManagedProjectError> {
    parse_project(&execute(
        github,
        GithubCommand::ViewProject {
            owner: owner.to_owned(),
            number,
        },
        "cannot verify GitHub Project",
    )?)
}

fn list_project_items<T: GithubTransport>(
    github: &mut T,
    owner: &str,
    number: u64,
) -> Result<HashSet<String>, ManagedProjectError> {
    let output = execute(
        github,
        GithubCommand::ListProjectItems {
            owner: owner.to_owned(),
            number,
        },
        "cannot list managed GitHub Project items",
    )?;
    let value: Value = serde_json::from_str(&output).map_err(json_error)?;
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| ManagedProjectError::new("GitHub Project item list has no items array"))?;
    if items.len() >= PROJECT_FETCH_LIMIT {
        return Err(ManagedProjectError::new(
            "GitHub Project item list may be truncated at the transport limit",
        ));
    }
    items
        .iter()
        .filter_map(|item| {
            item.pointer("/content/url")
                .or_else(|| item.get("contentUrl"))
                .or_else(|| item.get("url"))
                .and_then(Value::as_str)
        })
        .map(normalize_issue_url)
        .collect()
}

fn parse_project_numbers(output: &str) -> Result<Vec<u64>, ManagedProjectError> {
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

fn parse_project(output: &str) -> Result<RemoteProject, ManagedProjectError> {
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
        readme: value
            .get("readme")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

fn append_marker(
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
        Ok(format!("{}\n\n{marker}", readme.trim_end()))
    }
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

fn validate_policy(policy: &ManagedProjectPolicy) -> Result<(), ManagedProjectError> {
    if policy.owner.trim().is_empty() {
        return Err(ManagedProjectError::new(
            "managed GitHub Project policy has no owner",
        ));
    }
    Ok(())
}

struct BoundIdentity {
    owner: String,
    node_id: String,
    number: u64,
}

fn bound_identity(
    store: &ManagedProjectStore,
    policy: &ManagedProjectPolicy,
) -> Result<BoundIdentity, ManagedProjectError> {
    let binding = store.snapshot();
    let owner = binding
        .owner
        .clone()
        .ok_or_else(|| ManagedProjectError::new("managed GitHub Project is not resolved"))?;
    if owner != policy.owner {
        return Err(ManagedProjectError::new(
            "managed project binding owner conflicts with policy",
        ));
    }
    Ok(BoundIdentity {
        owner,
        node_id: binding
            .project_node_id
            .clone()
            .ok_or_else(|| ManagedProjectError::new("managed project binding has no node ID"))?,
        number: binding
            .project_number
            .ok_or_else(|| ManagedProjectError::new("managed project binding has no number"))?,
    })
}

fn projection_key(node_id: &str, issue_url: &str) -> String {
    format!("project:item-add:{node_id}:{issue_url}")
}

fn normalize_issue_url(url: &str) -> Result<String, ManagedProjectError> {
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
    if parts.len() != 4 || parts[2] != "issues" || parts[3].parse::<u64>().is_err() {
        return Err(ManagedProjectError::new(
            "issue URL must identify one GitHub issue",
        ));
    }
    Ok(normalized)
}

fn execute<T: GithubTransport>(
    github: &mut T,
    command: GithubCommand,
    context: &str,
) -> Result<String, ManagedProjectError> {
    github
        .execute(command)
        .map_err(|error| transport_error(context, error))
}

fn transport_error(context: &str, error: GithubFailure) -> ManagedProjectError {
    ManagedProjectError::new(format!("{context}: {error}"))
}

fn json_error(error: serde_json::Error) -> ManagedProjectError {
    ManagedProjectError::new(format!("invalid GitHub Project response: {error}"))
}
