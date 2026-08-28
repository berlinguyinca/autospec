use super::{ManagedProjectError, ManagedProjectStore, RemoteProject};
use crate::commands::autonomous::accountability::github::{
    GithubCommand, GithubFailure, GithubTransport,
};
use autospec_core::managed_project::ManagedProjectPolicy;
use std::collections::HashSet;

#[path = "github/parse.rs"]
mod parse;

const PROJECT_FETCH_LIMIT: usize = 500;

pub fn verify_managed_marker(
    readme: &str,
    policy: &ManagedProjectPolicy,
) -> Result<bool, ManagedProjectError> {
    parse::verify_managed_marker(readme, policy)
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
        return resume_bound_project(store, github, policy, number);
    }
    if let Some(provisional) = store.provisional_project().cloned() {
        if provisional.owner != policy.owner {
            return Err(ManagedProjectError::new(
                "provisional project owner conflicts with policy",
            ));
        }
        return resume_created_project(store, github, policy, &provisional);
    }

    let output = execute(
        github,
        GithubCommand::ListProjects {
            owner: policy.owner.clone(),
        },
        "cannot list GitHub Projects",
    )?;
    let numbers = parse::parse_project_numbers(&output)?;
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
            ack_create_projection_if_pending(store, policy)?;
            Ok(project)
        }
        count if count > 1 => Err(ManagedProjectError::new(format!(
            "multiple GitHub Projects have the managed marker for {}",
            policy.product_key.as_str()
        ))),
        _ if has_pending_create(store, policy) => Err(ManagedProjectError::new(
            "pending project creation has no verified project identity",
        )),
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
    let normalized = parse::normalize_issue_url(issue_url)?;
    let projection = projection_key(&identity.node_id, &normalized);
    store.enqueue_projection(projection.clone())?;
    let items = list_project_items(github, &identity.owner, identity.number)?;
    if items.contains(&normalized) {
        store.ack_projection(&projection)?;
        return Ok(());
    }
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
        let issue_url = parse::normalize_issue_url(&issue_url)?;
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
    let projection = create_projection(policy);
    store.enqueue_projection(projection.clone())?;
    let created = parse::parse_project(&execute(
        github,
        GithubCommand::CreateProject {
            owner: policy.owner.clone(),
            title: title.to_owned(),
        },
        "cannot create managed GitHub Project",
    )?)?;
    validate_returned_owner(&created, &policy.owner)?;
    store.record_created_project(&created)?;
    let provisional = store
        .provisional_project()
        .cloned()
        .ok_or_else(|| ManagedProjectError::new("created project identity was not journaled"))?;
    resume_created_project(store, github, policy, &provisional)
}

fn resume_created_project<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
    provisional: &super::ProjectIdentity,
) -> Result<RemoteProject, ManagedProjectError> {
    let before_edit = view_project(github, &policy.owner, provisional.number)?;
    validate_created_identity(&before_edit, provisional)?;
    let verified = if verify_managed_marker(&before_edit.readme, policy)? {
        before_edit
    } else {
        if !has_pending_create(store, policy) {
            return Err(ManagedProjectError::new(
                "provisional Project has no pending create projection",
            ));
        }
        let readme = parse::append_marker(&before_edit.readme, policy)?;
        execute(
            github,
            GithubCommand::EditProjectMarker {
                owner: policy.owner.clone(),
                number: provisional.number,
                readme,
            },
            "cannot write managed GitHub Project marker",
        )?;
        let verified = view_project(github, &policy.owner, provisional.number)?;
        validate_created_identity(&verified, provisional)?;
        validate_project(&verified, policy)?;
        verified
    };
    persist_project(store, &verified)?;
    ack_create_projection_if_pending(store, policy)?;
    Ok(verified)
}

fn validate_created_identity(
    project: &RemoteProject,
    provisional: &super::ProjectIdentity,
) -> Result<(), ManagedProjectError> {
    if project.owner != provisional.owner
        || project.node_id != provisional.node_id
        || project.number != provisional.number
    {
        return Err(ManagedProjectError::new(
            "verified remote project identity conflicts with provisional creation",
        ));
    }
    Ok(())
}

fn resume_bound_project<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
    number: u64,
) -> Result<RemoteProject, ManagedProjectError> {
    let expected_node_id = store
        .snapshot()
        .project_node_id
        .clone()
        .ok_or_else(|| ManagedProjectError::new("managed project binding has no node ID"))?;
    let before_edit = view_project(github, &policy.owner, number)?;
    validate_returned_owner(&before_edit, &policy.owner)?;
    if before_edit.node_id != expected_node_id || before_edit.number != number {
        return Err(ManagedProjectError::new(
            "verified remote project identity conflicts with local binding",
        ));
    }
    if verify_managed_marker(&before_edit.readme, policy)? {
        ack_create_projection_if_pending(store, policy)?;
        return Ok(before_edit);
    }
    if !has_pending_create(store, policy) {
        return Err(ManagedProjectError::new(
            "GitHub Project does not contain the expected managed marker",
        ));
    }
    let readme = parse::append_marker(&before_edit.readme, policy)?;
    execute(
        github,
        GithubCommand::EditProjectMarker {
            owner: policy.owner.clone(),
            number,
            readme,
        },
        "cannot write managed GitHub Project marker",
    )?;
    let verified = view_project(github, &policy.owner, number)?;
    if verified.node_id != expected_node_id || verified.number != number {
        return Err(ManagedProjectError::new(
            "GitHub returned a different Project after marker update",
        ));
    }
    validate_project(&verified, policy)?;
    ack_create_projection_if_pending(store, policy)?;
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
    parse::parse_project(&execute(
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
    parse::parse_project_items(&output)
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
    if has_pending_create(store, policy) {
        return Err(ManagedProjectError::new(
            "managed GitHub Project creation is not verified",
        ));
    }
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

fn create_projection(policy: &ManagedProjectPolicy) -> String {
    format!("project:create:{}", policy.product_key.as_str())
}

fn has_pending_create(store: &ManagedProjectStore, policy: &ManagedProjectPolicy) -> bool {
    let projection = create_projection(policy);
    store
        .snapshot()
        .pending_projections
        .iter()
        .any(|pending| pending == &projection)
}

fn ack_create_projection_if_pending(
    store: &mut ManagedProjectStore,
    policy: &ManagedProjectPolicy,
) -> Result<(), ManagedProjectError> {
    let projection = create_projection(policy);
    if has_pending_create(store, policy) {
        store.ack_projection(&projection)?;
    }
    Ok(())
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
    let message = format!("{context}: {error}");
    match error {
        GithubFailure::Retryable(_)
        | GithubFailure::RetryAfter { .. }
        | GithubFailure::Ambiguous(_) => ManagedProjectError::journaled_projection_pending(message),
        GithubFailure::LocalExecution(_) | GithubFailure::Definitive(_) => {
            ManagedProjectError::new(message)
        }
    }
}
