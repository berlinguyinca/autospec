use super::{
    parse_created_field, parse_remote_fields, plan_view_setup, required_managed_fields,
    resolve_managed_fields, validate_created_field, FieldOwnership, FieldPlanEntry,
    ManagedFieldSpec, ManagedProjectError, ManagedProjectStore, PortfolioShapeReport, RemoteField,
    RemoteProject, ViewCapability,
};
use crate::commands::autonomous::accountability::github::{
    GithubCommand, GithubFailure, GithubTransport,
};
use autospec_core::autonomous::waterfall::sha256_hex;
use autospec_core::managed_project::{
    ManagedProjectIdentity, ManagedProjectPolicy, RelationshipEdge, RelationshipEvidence,
    RelationshipKind, RelationshipState,
};
use std::collections::HashSet;

#[path = "github/parse.rs"]
mod parse;
pub use parse::PortfolioRecoveryCapsule;

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
    let candidates = parse::parse_project_candidates(&output)?;
    let marker = marker_material(store)?;
    let missing_portfolio_state =
        matches!(marker.identity, ManagedProjectIdentity::SpecPortfolio(_))
            && marker.recovery_capsule.is_none();
    let mut matches = Vec::new();
    let mut projects = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let project = view_project(github, &policy.owner, candidate.number)?;
        validate_returned_owner(&project, &policy.owner)?;
        let recovered_capsule = if missing_portfolio_state {
            parse::recoverable_portfolio_capsule(&project.readme, &marker.identity, &policy.owner)?
        } else {
            None
        };
        let exact = if missing_portfolio_state {
            recovered_capsule.is_some()
        } else {
            matches!(
                parse::classify_marker(
                    &project.readme,
                    &marker.identity,
                    &policy.owner,
                    marker.recovery_capsule.as_ref(),
                )?,
                parse::MarkerDisposition::Exact { .. }
            )
        };
        if exact {
            matches.push((project.clone(), recovered_capsule));
        }
        projects.push(project);
    }
    match matches.len() {
        1 => {
            let (project, recovered_capsule) = matches.pop().expect("one verified match");
            if let Some(capsule) = recovered_capsule {
                return recover_missing_portfolio_state(store, github, policy, project, capsule);
            }
            let project = ensure_project_marker(store, github, policy, project, false)?;
            persist_project(store, &project)?;
            ack_create_projection_if_pending(store)?;
            Ok(project)
        }
        count if count > 1 => Err(ManagedProjectError::new(format!(
            "multiple GitHub Projects have the managed marker for {}",
            marker_identity_key(store)
        ))),
        _ if missing_portfolio_state => Err(ManagedProjectError::new(
            "missing local spec portfolio state has no strict marker-bearing Project",
        )),
        _ if has_pending_create(store) => {
            recover_create_unknown(store, github, policy, title, projects)
        }
        _ => create_project(store, github, policy, title),
    }
}

pub fn reconcile_issue<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
    issue_url: &str,
) -> Result<(), ManagedProjectError> {
    let unresolved = journal_issue_projection(store, issue_url)?;
    let identity = bound_identity(store, policy)?;
    let normalized = unresolved_issue_url(&unresolved)?;
    let projection = projection_key(&identity.node_id, &normalized);
    store.enqueue_projection(projection.clone())?;
    store.ack_projection(&unresolved)?;
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

pub fn journal_issue_projection(
    store: &mut ManagedProjectStore,
    issue_url: &str,
) -> Result<String, ManagedProjectError> {
    let normalized = parse::normalize_issue_url(issue_url)?;
    store.record_edge(RelationshipEdge {
        product_key: store.snapshot().product_key.clone(),
        kind: RelationshipKind::Contains,
        source: store.snapshot().product_key.as_str().to_owned(),
        target: normalized.clone(),
        evidence: RelationshipEvidence {
            kind: "autospec-issue-membership".to_owned(),
            location: normalized.clone(),
            discovered_at: "managed-project-sync".to_owned(),
            confidence: 100,
        },
        state: RelationshipState::Active,
    })?;
    let projection = format!("project:item-add:unresolved:{normalized}");
    store.enqueue_projection(projection.clone())?;
    Ok(projection)
}

pub fn tracked_issue_urls(store: &ManagedProjectStore) -> Vec<String> {
    let mut urls = store
        .snapshot()
        .relationships
        .iter()
        .filter(|edge| {
            edge.kind == RelationshipKind::Contains
                && edge.state == RelationshipState::Active
                && edge.evidence.kind == "autospec-issue-membership"
        })
        .map(|edge| edge.target.clone())
        .collect::<Vec<_>>();
    urls.sort();
    urls.dedup();
    urls
}

pub fn normalize_issue_url(issue_url: &str) -> Result<String, ManagedProjectError> {
    parse::normalize_issue_url(issue_url)
}

pub fn retry_pending_projections<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
) -> Result<(), ManagedProjectError> {
    let identity = bound_identity(store, policy)?;
    let prefix = format!("project:item-add:{}:", identity.node_id);
    let unresolved_prefix = "project:item-add:unresolved:";
    let unresolved = store
        .snapshot()
        .pending_projections
        .iter()
        .filter_map(|projection| {
            projection
                .strip_prefix(unresolved_prefix)
                .map(|url| (projection.clone(), url.to_owned()))
        })
        .collect::<Vec<_>>();
    for (unresolved_projection, issue_url) in &unresolved {
        store.ensure_projection_pending(&projection_key(&identity.node_id, issue_url))?;
        store.ack_projection(unresolved_projection)?;
    }
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
                    issue_url: issue_url.clone(),
                },
                "cannot retry managed GitHub Project item addition",
            )?;
        }
        store.ack_projection(&projection)?;
    }
    Ok(())
}

/// Provisions the managed delivery shape of a bound spec-portfolio Project:
/// verifies built-in and creates owned custom fields, reports the view setup,
/// and idempotently adds every bound issue item. Field creates move through the
/// durable `field:<slug>` portfolio operations (intent → sent → acknowledged),
/// item additions through the journaled projection set, so a retryable failure
/// anywhere is resumable and never replayed blindly.
pub fn provision_portfolio_shape<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
) -> Result<PortfolioShapeReport, ManagedProjectError> {
    let ManagedProjectIdentity::SpecPortfolio(_) = store.snapshot().identity() else {
        return Err(ManagedProjectError::new(
            "managed delivery fields are a spec-portfolio projection",
        ));
    };
    if store.portfolio_snapshot().is_none() {
        return Err(ManagedProjectError::new(
            "spec portfolio Project shape provisioning requires the recovery capsule",
        ));
    }
    let identity = bound_identity(store, policy)?;
    let project_url = store
        .snapshot()
        .project_url
        .clone()
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| ManagedProjectError::new("managed project binding has no URL"))?;

    let remote = parse_remote_fields(&execute(
        github,
        GithubCommand::ListProjectFields {
            owner: identity.owner.clone(),
            number: identity.number,
        },
        "cannot list managed GitHub Project fields",
    )?)?;
    let ownership = field_ownership(store)?;
    let plan = resolve_managed_fields(&remote, &ownership)
        .map_err(|error| ManagedProjectError::new(error.to_string()))?;
    for entry in &plan.entries {
        match entry {
            FieldPlanEntry::Create { spec } => {
                ensure_field_operation(store, &ownership, spec)?;
                let created = parse_created_field(&execute(
                    github,
                    GithubCommand::CreateProjectField {
                        owner: identity.owner.clone(),
                        number: identity.number,
                        name: spec.name.to_owned(),
                        data_type: spec.kind.data_type().to_owned(),
                        single_select_options: spec
                            .options
                            .iter()
                            .map(|option| (*option).to_owned())
                            .collect(),
                    },
                    "cannot create managed GitHub Project field",
                )?)?;
                validate_created_field(&created, spec)
                    .map_err(|error| ManagedProjectError::new(error.to_string()))?;
                acknowledge_field_operation(store, &spec.operation_id(), &created)?;
            }
            // A lost create response is adopted only after the remote field
            // verifies against the managed spec; the acknowledgment then records
            // the node IDs the response would have carried.
            FieldPlanEntry::VerifiedOwned {
                name,
                node_id,
                options,
            } => {
                let spec = required_managed_fields()
                    .into_iter()
                    .find(|spec| spec.name == *name)
                    .expect("verified entry names a required field");
                let state = ownership
                    .iter()
                    .find(|(operation_id, _)| operation_id == &spec.operation_id())
                    .map(|(_, state)| *state);
                if state == Some(FieldOwnership::Sent) {
                    let field = RemoteField {
                        node_id: node_id.clone(),
                        name: (*name).to_owned(),
                        data_type: spec.kind.data_type().to_owned(),
                        options: options.clone(),
                    };
                    acknowledge_field_operation(store, &spec.operation_id(), &field)?;
                }
            }
            FieldPlanEntry::VerifiedBuiltIn { .. } => {}
        }
    }

    // The shared transport exposes no Project v2 view mutation (the gh CLI
    // `project` surface has no view create), so view setup is reported as
    // manual with the exact URL and field list; field and item provisioning
    // remain mandatory (spec "Project shape").
    let view_setup = plan_view_setup(ViewCapability::Unsupported, &project_url);

    let items = list_project_items(github, &identity.owner, identity.number)?;
    let mut urls: Vec<String> = store
        .portfolio_item_bindings()
        .iter()
        .filter_map(|binding| binding.get("issue_url").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    urls.sort();
    urls.dedup();
    let mut items_present = 0;
    let mut items_added = 0;
    for issue_url in &urls {
        let normalized = parse::normalize_issue_url(issue_url)?;
        let projection = projection_key(&identity.node_id, &normalized);
        if items.contains(&normalized) {
            store.ensure_projection_pending(&projection)?;
            store.ack_projection(&projection)?;
            items_present += 1;
        } else {
            store.ensure_projection_pending(&projection)?;
            execute(
                github,
                GithubCommand::AddToProject {
                    owner: identity.owner.clone(),
                    project_number: identity.number,
                    issue_url: normalized.clone(),
                },
                "cannot add portfolio issue to managed GitHub Project",
            )?;
            store.ack_projection(&projection)?;
            items_added += 1;
        }
    }

    Ok(PortfolioShapeReport {
        fields: plan.entries.clone(),
        view_setup,
        items_present,
        items_added,
    })
}

/// Current state of the `field:<slug>` ownership operations, last event wins.
fn field_ownership(
    store: &ManagedProjectStore,
) -> Result<Vec<(String, FieldOwnership)>, ManagedProjectError> {
    let mut ownership = Vec::new();
    for (operation_id, state) in store.portfolio_operation_states() {
        let Some(name) = operation_id.strip_prefix("field:") else {
            continue;
        };
        if name.is_empty() {
            return Err(ManagedProjectError::new(
                "field ownership operation has no field name",
            ));
        }
        let ownership_state = match state.as_str() {
            "intent" => FieldOwnership::Intent,
            "sent" => FieldOwnership::Sent,
            "acknowledged" => FieldOwnership::Acknowledged,
            other => {
                return Err(ManagedProjectError::new(format!(
                    "field ownership operation {operation_id} has invalid state {other}"
                )))
            }
        };
        if let Some((_, existing)) = ownership.iter_mut().find(|(id, _)| *id == operation_id) {
            *existing = ownership_state;
        } else {
            ownership.push((operation_id, ownership_state));
        }
    }
    Ok(ownership)
}

/// Records intent (nothing dispatched) and sent (create dispatched) before the
/// mutation, so a lost response is journaled ownership, never a blind re-create
/// or a human-owned collision.
fn ensure_field_operation(
    store: &mut ManagedProjectStore,
    ownership: &[(String, FieldOwnership)],
    spec: &ManagedFieldSpec,
) -> Result<(), ManagedProjectError> {
    let operation_id = spec.operation_id();
    let state = ownership
        .iter()
        .find(|(id, _)| id == &operation_id)
        .map(|(_, state)| *state);
    if state.is_none() {
        store.transition_portfolio_operation(
            &operation_id,
            "intent",
            serde_json::json!({
                "name": spec.name,
                "data_type": spec.kind.data_type(),
                "options": spec.options,
            }),
        )?;
    }
    if matches!(state, None | Some(FieldOwnership::Intent)) {
        store.transition_portfolio_operation(
            &operation_id,
            "sent",
            serde_json::json!({ "name": spec.name }),
        )?;
    }
    Ok(())
}

fn acknowledge_field_operation(
    store: &mut ManagedProjectStore,
    operation_id: &str,
    field: &RemoteField,
) -> Result<(), ManagedProjectError> {
    store.transition_portfolio_operation(
        operation_id,
        "acknowledged",
        serde_json::json!({
            "name": field.name,
            "node_id": field.node_id,
            "options": field.options.iter().map(|option| serde_json::json!({
                "name": option.name,
                "node_id": option.node_id,
            })).collect::<Vec<_>>(),
        }),
    )
}

fn unresolved_issue_url(projection: &str) -> Result<String, ManagedProjectError> {
    projection
        .strip_prefix("project:item-add:unresolved:")
        .map(str::to_owned)
        .ok_or_else(|| ManagedProjectError::new("invalid unresolved issue projection key"))
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
    let create_title = exact_nonce_title(store, title)?.unwrap_or_else(|| title.to_owned());
    validate_create_title(&create_title)?;
    let projection = create_projection(store);
    store.ensure_projection_pending(&projection)?;
    if let Some(title_projection) = create_title_projection(store, &create_title)? {
        store.ensure_projection_pending(&title_projection)?;
    }
    let created = parse::parse_project(&execute(
        github,
        GithubCommand::CreateProject {
            owner: policy.owner.clone(),
            title: create_title,
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
    let verified = ensure_project_marker(store, github, policy, before_edit, true)?;
    validate_created_identity(&verified, provisional)?;
    persist_project(store, &verified)?;
    ack_create_projection_if_pending(store)?;
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
    let verified = ensure_project_marker(
        store,
        github,
        policy,
        before_edit,
        has_pending_create(store),
    )?;
    if verified.node_id != expected_node_id || verified.number != number {
        return Err(ManagedProjectError::new(
            "GitHub returned a different Project after marker update",
        ));
    }
    persist_project(store, &verified)?;
    ack_create_projection_if_pending(store)?;
    Ok(verified)
}

fn validate_project(
    store: &ManagedProjectStore,
    project: &RemoteProject,
    policy: &ManagedProjectPolicy,
) -> Result<(), ManagedProjectError> {
    validate_returned_owner(project, &policy.owner)?;
    let marker = marker_material(store)?;
    if !matches!(
        parse::classify_marker(
            &project.readme,
            &marker.identity,
            &policy.owner,
            marker.recovery_capsule.as_ref(),
        )?,
        parse::MarkerDisposition::Exact { legacy: false }
    ) {
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
    if has_pending_create(store) {
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

fn create_projection(store: &ManagedProjectStore) -> String {
    format!("project:create:{}", marker_identity_key(store))
}

fn create_title_projection(
    store: &ManagedProjectStore,
    title: &str,
) -> Result<Option<String>, ManagedProjectError> {
    match store.snapshot().identity() {
        ManagedProjectIdentity::Product { .. } => Ok(None),
        ManagedProjectIdentity::SpecPortfolio(_) => {
            validate_create_title(title)?;
            Ok(Some(format!(
                "project:create-title:{}:{}",
                marker_identity_key(store),
                encode_title(title)
            )))
        }
    }
}

fn has_pending_create(store: &ManagedProjectStore) -> bool {
    let projection = create_projection(store);
    store
        .snapshot()
        .pending_projections
        .iter()
        .any(|pending| pending == &projection)
}

fn ack_create_projection_if_pending(
    store: &mut ManagedProjectStore,
) -> Result<(), ManagedProjectError> {
    let pending = pending_create_projections(store)?;
    for projection in pending {
        store.ack_projection(&projection)?;
    }
    Ok(())
}

struct MarkerMaterial {
    identity: ManagedProjectIdentity,
    recovery_capsule: Option<PortfolioRecoveryCapsule>,
}

fn marker_material(store: &ManagedProjectStore) -> Result<MarkerMaterial, ManagedProjectError> {
    let identity = store.snapshot().identity().clone();
    let recovery_capsule = match &identity {
        ManagedProjectIdentity::Product { .. } => None,
        ManagedProjectIdentity::SpecPortfolio(expected) => {
            let Some(capsule) = store
                .portfolio_snapshot()
                .and_then(|snapshot| snapshot.get("recovery_capsule"))
            else {
                return Ok(MarkerMaterial {
                    identity,
                    recovery_capsule: None,
                });
            };
            let capsule = PortfolioRecoveryCapsule::from_value(capsule)?;
            if capsule.portfolio_id() != expected.portfolio_id().as_str() {
                return Err(ManagedProjectError::new(
                    "spec portfolio recovery capsule identity conflicts with its store",
                ));
            }
            Some(capsule)
        }
    };
    Ok(MarkerMaterial {
        identity,
        recovery_capsule,
    })
}

fn marker_identity_key(store: &ManagedProjectStore) -> String {
    match store.snapshot().identity() {
        ManagedProjectIdentity::Product { product_key } => product_key.to_string(),
        ManagedProjectIdentity::SpecPortfolio(identity) => {
            format!("portfolio.{}", identity.portfolio_id())
        }
    }
}

fn exact_nonce_title(
    store: &ManagedProjectStore,
    title: &str,
) -> Result<Option<String>, ManagedProjectError> {
    let ManagedProjectIdentity::SpecPortfolio(_) = store.snapshot().identity() else {
        return Ok(None);
    };
    let marker = marker_material(store)?;
    let nonce = marker
        .recovery_capsule
        .as_ref()
        .map(PortfolioRecoveryCapsule::create_nonce)
        .ok_or_else(|| {
            ManagedProjectError::new("portfolio recovery capsule has no create nonce")
        })?;
    Ok(Some(format!("{title} [autospec:{nonce}]")))
}

fn recover_create_unknown<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
    title: &str,
    projects: Vec<RemoteProject>,
) -> Result<RemoteProject, ManagedProjectError> {
    let persisted_title = pending_create_title(store)?;
    let Some(expected_title) = persisted_title.or(exact_nonce_title(store, title)?) else {
        return Err(ManagedProjectError::new(
            "pending project creation has no verified project identity",
        ));
    };
    let mut candidates = projects
        .into_iter()
        .filter(|project| project.title == expected_title)
        .collect::<Vec<_>>();
    match candidates.len() {
        0 => Err(ManagedProjectError::journaled_projection_pending(
            "create_unknown: pending spec Project is not yet visible by its exact nonce title",
        )),
        1 => {
            let project = candidates.pop().expect("one exact nonce-title candidate");
            store.record_created_project(&project)?;
            let provisional = store
                .provisional_project()
                .cloned()
                .ok_or_else(|| ManagedProjectError::new("recovered project was not journaled"))?;
            resume_created_project(store, github, policy, &provisional)
        }
        count => Err(ManagedProjectError::new(format!(
            "create_unknown: {count} Projects match the exact nonce title"
        ))),
    }
}

fn ensure_project_marker<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
    project: RemoteProject,
    allow_missing: bool,
) -> Result<RemoteProject, ManagedProjectError> {
    let marker = marker_material(store)?;
    let disposition = parse::classify_marker(
        &project.readme,
        &marker.identity,
        &policy.owner,
        marker.recovery_capsule.as_ref(),
    )?;
    match disposition {
        parse::MarkerDisposition::Exact { legacy: false } => {
            ack_marker_projection_if_pending(store, &project)?;
            return Ok(project);
        }
        parse::MarkerDisposition::Missing if !allow_missing => {
            return Err(ManagedProjectError::new(
                "GitHub Project does not contain the expected managed marker",
            ))
        }
        parse::MarkerDisposition::Other => {
            return Err(ManagedProjectError::new(
                "GitHub Project contains a different managed marker",
            ))
        }
        parse::MarkerDisposition::Exact { legacy: true }
            if matches!(marker.identity, ManagedProjectIdentity::SpecPortfolio(_)) =>
        {
            return Err(ManagedProjectError::new(
                "spec portfolio marker cannot use the legacy product schema",
            ))
        }
        parse::MarkerDisposition::Exact { legacy: true } | parse::MarkerDisposition::Missing => {}
    }
    let readme = parse::upsert_marker(
        &project.readme,
        &marker.identity,
        &policy.owner,
        marker.recovery_capsule.as_ref(),
    )?;
    let projection = marker_projection(&project.node_id, &readme);
    store.ensure_projection_pending(&projection)?;
    execute(
        github,
        GithubCommand::EditProjectMarker {
            owner: policy.owner.clone(),
            number: project.number,
            readme,
        },
        "cannot write managed GitHub Project marker",
    )?;
    let verified = view_project(github, &policy.owner, project.number)?;
    if verified.node_id != project.node_id || verified.number != project.number {
        return Err(ManagedProjectError::new(
            "GitHub returned a different Project after marker update",
        ));
    }
    validate_project(store, &verified, policy)?;
    store.ack_projection(&projection)?;
    Ok(verified)
}

fn marker_projection(node_id: &str, readme: &str) -> String {
    format!("project:marker:{node_id}:{}", sha256_hex(readme.as_bytes()))
}

fn ack_marker_projection_if_pending(
    store: &mut ManagedProjectStore,
    project: &RemoteProject,
) -> Result<(), ManagedProjectError> {
    let projection = marker_projection(&project.node_id, &project.readme);
    if store
        .snapshot()
        .pending_projections
        .iter()
        .any(|pending| pending == &projection)
    {
        store.ack_projection(&projection)?;
    }
    Ok(())
}

fn recover_missing_portfolio_state<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &ManagedProjectPolicy,
    project: RemoteProject,
    capsule: PortfolioRecoveryCapsule,
) -> Result<RemoteProject, ManagedProjectError> {
    let ManagedProjectIdentity::SpecPortfolio(identity) = store.snapshot().identity().clone()
    else {
        return Err(ManagedProjectError::new(
            "recovery requires a spec portfolio store",
        ));
    };
    let snapshot = serde_json::json!({
        "schema": "autospec.portfolio-snapshot.v1",
        "portfolio_id": identity.portfolio_id(),
        "owner": policy.owner,
        "project_number": project.number,
        "project_node_id": project.node_id,
        "project_url": project.url,
        "source_spec": identity.source(),
        "plan_digest": capsule.plan_digest(),
        "lease_generation": 0,
        "state": "recovered",
        "projection_high_watermark": 0,
        "recovery_capsule": capsule.to_value()?,
    });
    store.record_portfolio_snapshot(snapshot)?;
    persist_project(store, &project)?;
    let verified = view_project(github, &policy.owner, project.number)?;
    validate_project(store, &verified, policy)?;
    persist_project(store, &verified)?;
    Ok(verified)
}

fn pending_create_projections(
    store: &ManagedProjectStore,
) -> Result<Vec<String>, ManagedProjectError> {
    let create = create_projection(store);
    let title_prefix = format!("project:create-title:{}:", marker_identity_key(store));
    let pending = store
        .snapshot()
        .pending_projections
        .iter()
        .filter(|projection| projection.as_str() == create || projection.starts_with(&title_prefix))
        .cloned()
        .collect::<Vec<_>>();
    if pending
        .iter()
        .filter(|projection| projection.starts_with(&title_prefix))
        .count()
        > 1
    {
        return Err(ManagedProjectError::new(
            "managed Project has multiple pending create-title intents",
        ));
    }
    Ok(pending)
}

fn pending_create_title(
    store: &ManagedProjectStore,
) -> Result<Option<String>, ManagedProjectError> {
    let prefix = format!("project:create-title:{}:", marker_identity_key(store));
    let Some(projection) = pending_create_projections(store)?
        .into_iter()
        .find(|projection| projection.starts_with(&prefix))
    else {
        return Ok(None);
    };
    let encoded = projection
        .strip_prefix(&prefix)
        .expect("matched title prefix");
    decode_title(encoded).map(Some)
}

fn validate_create_title(title: &str) -> Result<(), ManagedProjectError> {
    if title.is_empty() || title.len() > 256 || title.chars().any(char::is_control) {
        return Err(ManagedProjectError::new(
            "managed GitHub Project title is outside the safe byte or character bound",
        ));
    }
    Ok(())
}

fn encode_title(title: &str) -> String {
    title
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn decode_title(encoded: &str) -> Result<String, ManagedProjectError> {
    if encoded.is_empty() || !encoded.len().is_multiple_of(2) || encoded.len() > 512 {
        return Err(ManagedProjectError::new(
            "invalid durable Project create title",
        ));
    }
    let bytes = encoded
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|value| u8::from_str_radix(value, 16).ok())
                .ok_or_else(|| ManagedProjectError::new("invalid durable Project create title"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let title = String::from_utf8(bytes)
        .map_err(|_| ManagedProjectError::new("invalid durable Project create title"))?;
    validate_create_title(&title)?;
    Ok(title)
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
