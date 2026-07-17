use std::collections::BTreeSet;

use autospec_core::runtime_env::{
    write_json_atomic, ComposeOwnership, ComposePlan, OwnedVolume, ResourceInventory,
    RuntimeContext,
};

use crate::commands::CommandFailure;

use super::super::state::StateLayout;
use super::{docker_command, output_lines, require_success, run};

const FILTER_FLAG: &str = concat!("-", "-filter");
const FORMAT_FLAG: &str = concat!("-", "-format");
const NO_TRUNC_FLAG: &str = concat!("-", "-no-trunc");

pub(super) fn reconcile(
    plan: &ComposePlan,
    ownership: &ComposeOwnership,
    layout: &StateLayout,
    inventory: &mut ResourceInventory,
    context: &RuntimeContext,
) -> Result<(), CommandFailure> {
    let recorded_volumes = inventory.volumes.clone();
    let discovered = discover_project_resources(plan)?;
    reject_recorded_foreign(inventory, &discovered, context)?;
    record_project_ids(
        ResourceKind::Container,
        &discovered.containers,
        ownership,
        context,
    )?;
    inventory.containers = discovered.containers;
    persist(layout, inventory)?;
    record_project_ids(
        ResourceKind::Network,
        &discovered.networks,
        ownership,
        context,
    )?;
    inventory.networks = discovered.networks;
    persist(layout, inventory)?;
    let volumes = discover_owned_volumes(
        discovered.named_volumes,
        &discovered.all_volumes,
        ownership,
        context,
        layout,
        inventory,
    )?;
    if detached_anonymous_volume(&recorded_volumes, &volumes, &discovered.all_volumes) {
        return Err(recovery(context, "COMPOSE_OWNERSHIP_MISMATCH"));
    }
    inventory.volumes = volumes;
    persist(layout, inventory)
}

struct DiscoveredResources {
    containers: Vec<String>,
    networks: Vec<String>,
    named_volumes: Vec<String>,
    all_containers: BTreeSet<String>,
    all_networks: BTreeSet<String>,
    all_volumes: BTreeSet<String>,
}

fn discover_project_resources(plan: &ComposePlan) -> Result<DiscoveredResources, CommandFailure> {
    Ok(DiscoveredResources {
        containers: project_ids(ResourceKind::Container, &plan.project_name)?,
        networks: project_ids(ResourceKind::Network, &plan.project_name)?,
        named_volumes: project_ids(ResourceKind::Volume, &plan.project_name)?,
        all_containers: all_ids(ResourceKind::Container)?,
        all_networks: all_ids(ResourceKind::Network)?,
        all_volumes: all_ids(ResourceKind::Volume)?,
    })
}

fn record_project_ids(
    kind: ResourceKind,
    ids: &[String],
    ownership: &ComposeOwnership,
    context: &RuntimeContext,
) -> Result<(), CommandFailure> {
    for id in ids {
        verify_labels(kind, id, ownership, context)?;
    }
    Ok(())
}

fn discover_owned_volumes(
    named: Vec<String>,
    all: &BTreeSet<String>,
    ownership: &ComposeOwnership,
    context: &RuntimeContext,
    layout: &StateLayout,
    inventory: &mut ResourceInventory,
) -> Result<Vec<OwnedVolume>, CommandFailure> {
    let mut volumes = Vec::new();
    for id in named {
        verify_labels(ResourceKind::Volume, &id, ownership, context)?;
        volumes.push(OwnedVolume {
            logical_key: volume_logical_key(&id)?,
            id,
        });
        persist_volumes(layout, inventory, &volumes)?;
    }
    let containers = inventory.containers.clone();
    for container in containers {
        for id in container_mounts(&container)? {
            if all.contains(&id) && !volumes.iter().any(|volume| volume.id == id) {
                volumes.push(OwnedVolume {
                    logical_key: None,
                    id,
                });
                persist_volumes(layout, inventory, &volumes)?;
            }
        }
    }
    Ok(volumes)
}

fn persist_volumes(
    layout: &StateLayout,
    inventory: &mut ResourceInventory,
    volumes: &[OwnedVolume],
) -> Result<(), CommandFailure> {
    inventory.volumes = volumes.to_vec();
    persist(layout, inventory)
}

fn detached_anonymous_volume(
    recorded: &[OwnedVolume],
    discovered: &[OwnedVolume],
    all: &BTreeSet<String>,
) -> bool {
    recorded.iter().any(|volume| {
        volume.logical_key.is_none()
            && all.contains(&volume.id)
            && !discovered.iter().any(|current| current.id == volume.id)
    })
}

pub(super) fn remove_exact_owned(
    plan: &ComposePlan,
    ownership: &ComposeOwnership,
    layout: &StateLayout,
    inventory: &mut ResourceInventory,
    context: &RuntimeContext,
) -> Result<(), CommandFailure> {
    reconcile(plan, ownership, layout, inventory, context)?;
    let containers = inventory.containers.clone();
    for id in containers {
        remove_or_confirm_absent(
            ResourceKind::Container,
            &id,
            &["rm", "-f", "-v", &id],
            "COMPOSE_CONTAINER_DELETE_FAILED",
        )?;
        inventory.containers.retain(|candidate| candidate != &id);
        let existing = all_ids(ResourceKind::Volume)?;
        inventory
            .volumes
            .retain(|volume| existing.contains(&volume.id));
        persist(layout, inventory)?;
    }
    let networks = inventory.networks.clone();
    for id in networks {
        remove_or_confirm_absent(
            ResourceKind::Network,
            &id,
            &["network", "rm", &id],
            "COMPOSE_NETWORK_DELETE_FAILED",
        )?;
        inventory.networks.retain(|candidate| candidate != &id);
        persist(layout, inventory)?;
    }
    for id in inventory.deletable_volumes(&plan.preserve_volumes) {
        remove_or_confirm_absent(
            ResourceKind::Volume,
            &id,
            &["volume", "rm", &id],
            "COMPOSE_VOLUME_DELETE_FAILED",
        )?;
        inventory.volumes.retain(|volume| volume.id != id);
        persist(layout, inventory)?;
    }
    verify_deleted(inventory, &plan.preserve_volumes, context)?;
    Ok(())
}

fn remove_or_confirm_absent(
    kind: ResourceKind,
    id: &str,
    args: &[&str],
    code: &str,
) -> Result<(), CommandFailure> {
    let output = run(docker_command(args))?;
    if output.status.success() {
        return Ok(());
    }
    let original = require_success(output, code).expect_err("nonzero output is an error");
    if all_ids(kind)?.contains(id) {
        Err(original)
    } else {
        Ok(())
    }
}

fn verify_deleted(
    inventory: &ResourceInventory,
    preserved: &[String],
    context: &RuntimeContext,
) -> Result<(), CommandFailure> {
    let containers = all_ids(ResourceKind::Container)?;
    let networks = all_ids(ResourceKind::Network)?;
    let volumes = all_ids(ResourceKind::Volume)?;
    let present = inventory
        .containers
        .iter()
        .any(|id| containers.contains(id))
        || inventory.networks.iter().any(|id| networks.contains(id))
        || inventory
            .deletable_volumes(preserved)
            .iter()
            .any(|id| volumes.contains(id));
    if present {
        Err(recovery(context, "COMPOSE_RESOURCE_STILL_PRESENT"))
    } else {
        Ok(())
    }
}

fn reject_recorded_foreign(
    inventory: &ResourceInventory,
    discovered: &DiscoveredResources,
    context: &RuntimeContext,
) -> Result<(), CommandFailure> {
    let foreign =
        inventory.containers.iter().any(|id| {
            discovered.all_containers.contains(id) && !discovered.containers.contains(id)
        }) || inventory
            .networks
            .iter()
            .any(|id| discovered.all_networks.contains(id) && !discovered.networks.contains(id))
            || inventory.volumes.iter().any(|volume| {
                volume.logical_key.is_some()
                    && discovered.all_volumes.contains(&volume.id)
                    && !discovered.named_volumes.contains(&volume.id)
            });
    if foreign {
        Err(recovery(context, "COMPOSE_OWNERSHIP_MISMATCH"))
    } else {
        Ok(())
    }
}

fn project_ids(kind: ResourceKind, project: &str) -> Result<Vec<String>, CommandFailure> {
    let mut command = docker_command(kind.list_args());
    command.args([
        FILTER_FLAG,
        &format!("label=com.docker.compose.project={project}"),
    ]);
    let output = require_success(run(command)?, "COMPOSE_INVENTORY_FAILED")?;
    output_lines(&output.stdout)
}

fn all_ids(kind: ResourceKind) -> Result<BTreeSet<String>, CommandFailure> {
    let output = require_success(
        run(docker_command(kind.list_args()))?,
        "COMPOSE_INVENTORY_FAILED",
    )?;
    Ok(output_lines(&output.stdout)?.into_iter().collect())
}

fn verify_labels(
    kind: ResourceKind,
    id: &str,
    ownership: &ComposeOwnership,
    context: &RuntimeContext,
) -> Result<(), CommandFailure> {
    let args = kind.inspect_label_args(id);
    let output = require_success(
        run(docker_command(&args))?,
        "COMPOSE_OWNERSHIP_CHECK_FAILED",
    )?;
    let matches = ownership
        .matches_json(&output.stdout)
        .map_err(|_| recovery(context, "COMPOSE_OWNERSHIP_EVIDENCE_INVALID"))?;
    if matches {
        Ok(())
    } else {
        Err(recovery(context, "COMPOSE_OWNERSHIP_MISMATCH"))
    }
}

fn volume_logical_key(id: &str) -> Result<Option<String>, CommandFailure> {
    let output = require_success(
        run(docker_command(&[
            "volume",
            "inspect",
            FORMAT_FLAG,
            "{{ index .Labels \"com.docker.compose.volume\" }}",
            id,
        ]))?,
        "COMPOSE_INVENTORY_FAILED",
    )?;
    let value =
        String::from_utf8(output.stdout).map_err(|_| failure("COMPOSE_OUTPUT_INVALID_UTF8"))?;
    Ok((!value.trim().is_empty()).then(|| value.trim().to_string()))
}

fn container_mounts(id: &str) -> Result<Vec<String>, CommandFailure> {
    let output = require_success(
        run(docker_command(&[
            "inspect",
            FORMAT_FLAG,
            "{{range .Mounts}}{{if eq .Type \"volume\"}}{{println .Name}}{{end}}{{end}}",
            id,
        ]))?,
        "COMPOSE_INVENTORY_FAILED",
    )?;
    output_lines(&output.stdout)
}

fn persist(layout: &StateLayout, inventory: &ResourceInventory) -> Result<(), CommandFailure> {
    write_json_atomic(&layout.inventory, inventory).map_err(|error| failure(error.to_string()))
}

pub(super) fn recovery(context: &RuntimeContext, code: &str) -> CommandFailure {
    failure(format!(
        "{code}: recovery: {}",
        recovery_command(&context.repo, context.mode.name())
    ))
}

fn recovery_command(repo: &std::path::Path, mode: &str) -> String {
    format!(
        "autospec runtime env down {}repo {} {}mode {}",
        concat!("-", "-"),
        autospec_core::runtime_env::shell_quote(&repo.display().to_string()),
        concat!("-", "-"),
        autospec_core::runtime_env::shell_quote(mode)
    )
}

#[derive(Clone, Copy)]
enum ResourceKind {
    Container,
    Network,
    Volume,
}

impl ResourceKind {
    fn list_args(self) -> &'static [&'static str] {
        match self {
            Self::Container => &["ps", "-aq", NO_TRUNC_FLAG],
            Self::Network => &["network", "ls", "-q", NO_TRUNC_FLAG],
            Self::Volume => &["volume", "ls", "-q"],
        }
    }

    fn inspect_label_args(self, id: &str) -> Vec<&str> {
        match self {
            Self::Container => vec!["inspect", FORMAT_FLAG, "{{json .Config.Labels}}", id],
            Self::Network => vec!["network", "inspect", FORMAT_FLAG, "{{json .Labels}}", id],
            Self::Volume => vec!["volume", "inspect", FORMAT_FLAG, "{{json .Labels}}", id],
        }
    }
}

fn failure(message: impl Into<String>) -> CommandFailure {
    CommandFailure::diagnostic(message.into())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::recovery_command;

    #[test]
    fn recovery_quotes_repository_and_mode_metacharacters() {
        assert_eq!(
            recovery_command(Path::new("/tmp/repo '$(touch nope)\nnext"), "dev;\nexit 9"),
            format!(
                "autospec runtime env down {}repo '/tmp/repo '\\''$(touch nope)\nnext' {}mode 'dev;\nexit 9'",
                concat!("-", "-"),
                concat!("-", "-")
            )
        );
    }
}
