use std::fs;
use std::path::Path;
use std::process::Command;

use autospec_core::runtime_env::{
    load_generation_token, read_json, ComposeOwnership, EnvironmentOwner, GcDecision,
    GcInventorySnapshot, GcOwnerSnapshot, GcPolicy, ResourceInventory, ResourcePlan,
};

use crate::commands::CommandFailure;

use super::session::live_sessions;
use super::state::{release_ports, EnvironmentLease, StateLayout};
use super::MavenAdapter;

pub(super) fn collect(root: &Path, requested_repo: Option<&Path>) -> Result<usize, CommandFailure> {
    if !root.exists() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in fs::read_dir(root).map_err(io_failure)? {
        let path = entry.map_err(io_failure)?.path();
        if !path.is_dir() || path.file_name().is_some_and(|name| name == "ports") {
            continue;
        }
        removed += collect_candidate(root, &path, requested_repo)?;
    }
    Ok(removed)
}

fn collect_candidate(
    root: &Path,
    directory: &Path,
    requested_repo: Option<&Path>,
) -> Result<usize, CommandFailure> {
    let Some(_lease) = EnvironmentLease::try_acquire(directory)? else {
        return Ok(0);
    };
    let layout = StateLayout::new(
        root,
        directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
    );
    if !layout.owner.is_file() || !layout.plan.is_file() || !layout.inventory.is_file() {
        return Ok(0);
    }
    let owner: EnvironmentOwner = read_json(&layout.owner).map_err(runtime_failure)?;
    if !matches_requested_repo(&owner, requested_repo) {
        return Ok(0);
    }
    let plan: ResourcePlan = read_json(&layout.plan).map_err(runtime_failure)?;
    let inventory: ResourceInventory = read_json(&layout.inventory).map_err(runtime_failure)?;
    evaluate_and_collect(root, &layout, &owner, &plan, &inventory)
}

fn matches_requested_repo(owner: &EnvironmentOwner, requested: Option<&Path>) -> bool {
    requested.is_none_or(|path| owner.identity.canonical_repo == path)
}

fn evaluate_and_collect(
    root: &Path,
    layout: &StateLayout,
    owner: &EnvironmentOwner,
    plan: &ResourcePlan,
    inventory: &ResourceInventory,
) -> Result<usize, CommandFailure> {
    let worktree_exists = owner.identity.canonical_repo.exists();
    let current_generation = current_generation(&owner.identity.canonical_repo, worktree_exists)?;
    let docker_owner_keys = docker_owner_keys(inventory, owner, plan)?;
    let live_sessions = live_sessions(&layout.environment_dir)?.len();
    let owner_snapshot = GcOwnerSnapshot {
        environment_id: owner.identity.environment_id.clone(),
        owner_key: owner.identity.owner_key.clone(),
        recorded_generation: owner.identity.generation.clone(),
        current_generation,
        worktree_exists,
        locked_session_records: live_sessions,
    };
    let inventory_snapshot = GcInventorySnapshot {
        environment_id: inventory.environment_id.clone(),
        docker_owner_keys,
        live_environment_owners: foreign_live_owners(root, layout, inventory)?,
    };
    match GcPolicy::evaluate(&owner_snapshot, &inventory_snapshot) {
        GcDecision::Delete => delete_owned(root, layout, owner, plan, inventory).map(|()| 1),
        GcDecision::Retain(_) => Ok(0),
        GcDecision::Ambiguous(code) => Err(ambiguous(code, &owner.identity.canonical_repo)),
    }
}

fn foreign_live_owners(
    root: &Path,
    candidate: &StateLayout,
    inventory: &ResourceInventory,
) -> Result<Vec<String>, CommandFailure> {
    let mut owners = Vec::new();
    for entry in fs::read_dir(root).map_err(io_failure)? {
        let directory = entry.map_err(io_failure)?.path();
        if directory == candidate.environment_dir || !directory.join("owner.json").is_file() {
            continue;
        }
        let lease = EnvironmentLease::try_acquire(&directory)?;
        let locked = lease.is_none();
        let owner: EnvironmentOwner =
            read_json(&directory.join("owner.json")).map_err(runtime_failure)?;
        let other: ResourceInventory =
            read_json(&directory.join("inventory.json")).map_err(runtime_failure)?;
        if inventories_overlap(inventory, &other) && (locked || owner_is_live(&directory, &owner)?)
        {
            owners.push(owner.identity.owner_key);
        }
    }
    Ok(owners)
}

fn owner_is_live(directory: &Path, owner: &EnvironmentOwner) -> Result<bool, CommandFailure> {
    if !live_sessions(directory)?.is_empty() {
        return Ok(true);
    }
    let repo = &owner.identity.canonical_repo;
    Ok(repo.exists()
        && load_generation_token(repo)
            .map_err(runtime_failure)?
            .as_deref()
            == owner.identity.generation.as_deref())
}

fn inventories_overlap(left: &ResourceInventory, right: &ResourceInventory) -> bool {
    left.compose_project.is_some() && left.compose_project == right.compose_project
        || shared(&left.containers, &right.containers)
        || shared(&left.networks, &right.networks)
        || left
            .volumes
            .iter()
            .any(|item| right.volumes.iter().any(|other| item.id == other.id))
        || left.maven_local_prefix.is_some() && left.maven_local_prefix == right.maven_local_prefix
        || same_port(left.frontend_port, right.frontend_port)
        || same_port(left.backend_port, right.backend_port)
}

fn shared(left: &[String], right: &[String]) -> bool {
    left.iter().any(|item| right.contains(item))
}

fn same_port(left: Option<u16>, right: Option<u16>) -> bool {
    left.is_some() && left == right
}

fn current_generation(repo: &Path, exists: bool) -> Result<Option<String>, CommandFailure> {
    if !exists {
        return Ok(None);
    }
    load_generation_token(repo).map_err(runtime_failure)
}

fn docker_owner_keys(
    inventory: &ResourceInventory,
    owner: &EnvironmentOwner,
    plan: &ResourcePlan,
) -> Result<Vec<String>, CommandFailure> {
    let ownership = ComposeOwnership {
        environment_id: owner.identity.environment_id.clone(),
        owner_key: owner.identity.owner_key.clone(),
        plan_digest: plan.digest.clone(),
    };
    let mut keys = Vec::new();
    for id in &inventory.containers {
        keys.extend(inspect_owner(
            &["inspect", "--format", "{{json .Config.Labels}}", id],
            &ownership,
        )?);
    }
    for id in &inventory.networks {
        keys.extend(inspect_owner(
            &["network", "inspect", "--format", "{{json .Labels}}", id],
            &ownership,
        )?);
    }
    for volume in &inventory.volumes {
        keys.extend(inspect_owner(
            &[
                "volume",
                "inspect",
                "--format",
                "{{json .Labels}}",
                &volume.id,
            ],
            &ownership,
        )?);
    }
    Ok(keys)
}

fn inspect_owner(
    args: &[&str],
    ownership: &ComposeOwnership,
) -> Result<Option<String>, CommandFailure> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(io_failure)?;
    if !output.status.success() {
        if resource_is_absent(&output.stderr) {
            return Ok(None);
        }
        return Err(CommandFailure::diagnostic("RESOURCE_OWNERSHIP_UNVERIFIED"));
    }
    ownership
        .matches_json(&output.stdout)
        .map(|matches| {
            Some(if matches {
                ownership.owner_key.clone()
            } else {
                "RESOURCE_OWNER_MISMATCH".to_string()
            })
        })
        .map_err(|_| CommandFailure::diagnostic("RESOURCE_OWNERSHIP_EVIDENCE_INVALID"))
}

fn delete_owned(
    root: &Path,
    layout: &StateLayout,
    owner: &EnvironmentOwner,
    plan: &ResourcePlan,
    inventory: &ResourceInventory,
) -> Result<(), CommandFailure> {
    let mut remaining = inventory.clone();
    delete_docker_resources(layout, plan, &mut remaining)?;
    purge_maven_prefix(layout, owner, plan, &remaining)?;
    release_ports(root, &owner.identity.environment_id)?;
    fs::remove_dir_all(&layout.environment_dir).map_err(io_failure)
}

fn delete_docker_resources(
    layout: &StateLayout,
    plan: &ResourcePlan,
    inventory: &mut ResourceInventory,
) -> Result<(), CommandFailure> {
    while let Some(id) = inventory.containers.first().cloned() {
        run_delete(&["rm", "-f", "-v", &id])?;
        inventory.containers.remove(0);
        persist_inventory(layout, inventory)?;
    }
    while let Some(id) = inventory.networks.first().cloned() {
        run_delete(&["network", "rm", &id])?;
        inventory.networks.remove(0);
        persist_inventory(layout, inventory)?;
    }
    let preserved = plan
        .compose
        .as_ref()
        .map(|compose| compose.preserve_volumes.as_slice())
        .unwrap_or_default();
    for id in inventory.deletable_volumes(preserved) {
        run_delete(&["volume", "rm", &id])?;
        inventory.volumes.retain(|volume| volume.id != id);
        persist_inventory(layout, inventory)?;
    }
    Ok(())
}

fn run_delete(args: &[&str]) -> Result<(), CommandFailure> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(io_failure)?;
    if output.status.success() || resource_is_absent(&output.stderr) {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic("RESOURCE_DELETE_FAILED"))
    }
}

fn persist_inventory(
    layout: &StateLayout,
    inventory: &ResourceInventory,
) -> Result<(), CommandFailure> {
    autospec_core::runtime_env::write_json_atomic(&layout.inventory, inventory)
        .map_err(runtime_failure)
}

fn resource_is_absent(stderr: &[u8]) -> bool {
    let message = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    message.contains("no such") || message.contains("not found")
}

fn purge_maven_prefix(
    layout: &StateLayout,
    owner: &EnvironmentOwner,
    plan: &ResourcePlan,
    inventory: &ResourceInventory,
) -> Result<(), CommandFailure> {
    if inventory.maven_local_prefix.is_none() {
        return Ok(());
    }
    if !owner.identity.canonical_repo.exists() {
        return Err(CommandFailure::diagnostic(
            "MAVEN_PURGE_WORKTREE_UNAVAILABLE: guarded purge requires the recorded worktree",
        ));
    }
    let context =
        super::context_from_plan(&owner.identity.canonical_repo, &owner.identity.mode, plan)?;
    let state = super::read_state(&context)?;
    MavenAdapter::purge_owned_prefix(plan.maven.as_ref(), &context, &state, layout, inventory)
}

fn ambiguous(code: &str, repo: &Path) -> CommandFailure {
    CommandFailure::diagnostic(format!(
        "{code}: recovery: autospec runtime env gc --repo '{}'",
        repo.display()
    ))
}

fn io_failure(error: std::io::Error) -> CommandFailure {
    CommandFailure::diagnostic(error.to_string())
}

fn runtime_failure(error: autospec_core::runtime_env::RuntimeEnvError) -> CommandFailure {
    CommandFailure::diagnostic(error.to_string())
}
