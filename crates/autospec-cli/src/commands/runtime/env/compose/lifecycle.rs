use std::path::Path;

use autospec_core::runtime_env::{
    write_json_atomic, ComposeIsolation, ComposeOverride, ComposeOwnership, ComposePlan,
    EnvironmentIdentity, ExportProtocol, OwnedVolume, ResolvedExport, ResourceInventory,
    ResourcePlan, RuntimeContext, RuntimeState,
};

use crate::commands::CommandFailure;

use super::super::state::{read_json, StateLayout};
use super::{
    compose_command, docker_command, output_lines, require_success, run, write_override_atomic,
};

const OVERRIDE_FILE: &str = "compose.autospec.override.yaml";
const REMOVE_ORPHANS_FLAG: &str = concat!("-", "-remove-orphans");
const PROTOCOL_FLAG: &str = concat!("-", "-protocol");
const FILTER_FLAG: &str = concat!("-", "-filter");
const FORMAT_FLAG: &str = concat!("-", "-format");

pub(super) fn reject_caller_project_name(
    compose: Option<&ComposePlan>,
) -> Result<(), CommandFailure> {
    if enabled_plan(compose).is_some() && std::env::var_os("COMPOSE_PROJECT_NAME").is_some() {
        return Err(failure(
            "COMPOSE_PROJECT_NAME_CALLER_OVERRIDE: the broker owns the Compose project name",
        ));
    }
    Ok(())
}

pub(super) fn up(
    compose: Option<&ComposePlan>,
    resource_plan: &ResourcePlan,
    resolved_model: Option<&[u8]>,
    context: &RuntimeContext,
    state: &mut RuntimeState,
    layout: &StateLayout,
) -> Result<(), CommandFailure> {
    let Some(plan) = enabled_plan(compose) else {
        return Ok(());
    };
    let model = resolved_model.ok_or_else(|| failure("COMPOSE_MODEL_MISSING"))?;
    let ownership = ownership(&resource_plan.identity, &resource_plan.digest);
    let rendered = ComposeOverride::render_json(plan, model, &ownership)
        .map_err(|error| failure(error.to_string()))?;
    let override_path = layout.environment_dir.join(OVERRIDE_FILE);
    write_override_atomic(&override_path, &rendered)?;
    let output = run(compose_command(
        plan,
        context,
        &override_path,
        &["up", "-d", REMOVE_ORPHANS_FLAG],
    ))?;
    require_success(output, "COMPOSE_UP_FAILED")?;
    let mut inventory = inventory(layout)?;
    inventory.compose_project = Some(plan.project_name.clone());
    persist(layout, &inventory)?;
    discover_inventory(
        plan,
        context,
        &override_path,
        &ownership,
        layout,
        &mut inventory,
    )?;
    resolve_exports(plan, context, &override_path, layout, state, &mut inventory)?;
    Ok(())
}

pub(super) fn down_owned(
    compose: Option<&ComposePlan>,
    resource_plan: &ResourcePlan,
    context: &RuntimeContext,
    layout: &StateLayout,
) -> Result<(), CommandFailure> {
    let Some(plan) = enabled_plan(compose) else {
        return Ok(());
    };
    let mut inventory = inventory(layout)?;
    if inventory.compose_project.as_deref() != Some(plan.project_name.as_str()) {
        return Err(recovery(context, "COMPOSE_PROJECT_OWNERSHIP_MISMATCH"));
    }
    let ownership = ownership(&resource_plan.identity, &resource_plan.digest);
    verify_inventory_ownership(&inventory, &ownership, context)?;
    let override_path = layout.environment_dir.join(OVERRIDE_FILE);
    let output = run(compose_command(
        plan,
        context,
        &override_path,
        &["down", REMOVE_ORPHANS_FLAG],
    ))?;
    require_success(output, "COMPOSE_DOWN_FAILED")?;
    for volume in inventory.deletable_volumes(&plan.preserve_volumes) {
        require_success(
            run(docker_command(&["volume", "rm", &volume]))?,
            "COMPOSE_VOLUME_DELETE_FAILED",
        )?;
    }
    verify_absent(&inventory, &plan.preserve_volumes, context)?;
    inventory.compose_project = None;
    inventory.containers.clear();
    inventory.networks.clear();
    inventory.volumes.clear();
    inventory.exports.clear();
    persist(layout, &inventory)
}

fn discover_inventory(
    plan: &ComposePlan,
    context: &RuntimeContext,
    override_path: &Path,
    ownership: &ComposeOwnership,
    layout: &StateLayout,
    inventory: &mut ResourceInventory,
) -> Result<(), CommandFailure> {
    inventory.containers = list_owned(&["ps", "-aq"], ownership)?;
    persist(layout, inventory)?;
    inventory.volumes.clear();
    for container in &inventory.containers.clone() {
        for id in container_mounts(container)? {
            if !inventory.volumes.iter().any(|volume| volume.id == id) {
                inventory.volumes.push(OwnedVolume {
                    logical_key: None,
                    id,
                });
                persist(layout, inventory)?;
            }
        }
    }
    inventory.networks = list_owned(&["network", "ls", "-q"], ownership)?;
    persist(layout, inventory)?;
    let volumes = list_owned(&["volume", "ls", "-q"], ownership)?;
    for id in volumes {
        let logical_key = volume_logical_key(&id)?;
        if let Some(volume) = inventory.volumes.iter_mut().find(|volume| volume.id == id) {
            volume.logical_key = logical_key;
        } else {
            inventory.volumes.push(OwnedVolume { logical_key, id });
        }
        persist(layout, inventory)?;
    }
    let _ = (plan, context, override_path);
    Ok(())
}

fn resolve_exports(
    plan: &ComposePlan,
    context: &RuntimeContext,
    override_path: &Path,
    layout: &StateLayout,
    state: &mut RuntimeState,
    inventory: &mut ResourceInventory,
) -> Result<(), CommandFailure> {
    inventory.exports.clear();
    for declaration in &plan.exports {
        let protocol = if declaration.protocol == ExportProtocol::Udp {
            "udp"
        } else {
            "tcp"
        };
        let target = declaration.target.to_string();
        let output = require_success(
            run(compose_command(
                plan,
                context,
                override_path,
                &[
                    "port",
                    PROTOCOL_FLAG,
                    protocol,
                    &declaration.service,
                    &target,
                ],
            ))?,
            "COMPOSE_PORT_FAILED",
        )?;
        let resolved = parse_port(&declaration.env, &output.stdout)?;
        let value = resolved
            .render(declaration)
            .map_err(|error| failure(error.to_string()))?;
        state
            .set_value(&declaration.env, value)
            .map_err(|error| failure(error.to_string()))?;
        inventory.exports.push(resolved);
        persist(layout, inventory)?;
    }
    set_canonical_url(plan, inventory, state)
}

fn set_canonical_url(
    plan: &ComposePlan,
    inventory: &ResourceInventory,
    state: &mut RuntimeState,
) -> Result<(), CommandFailure> {
    let candidates = plan
        .exports
        .iter()
        .enumerate()
        .filter(|(_, export)| {
            matches!(
                export.protocol,
                ExportProtocol::Http | ExportProtocol::Https
            )
        })
        .collect::<Vec<_>>();
    if let [(index, declaration)] = candidates.as_slice() {
        let value = inventory.exports[*index]
            .render(declaration)
            .map_err(|error| failure(error.to_string()))?;
        state
            .set_value("AUTOSPEC_PUBLIC_URL", value.clone())
            .map_err(|error| failure(error.to_string()))?;
        state
            .set_value("AGENT_PUBLIC_URL", value)
            .map_err(|error| failure(error.to_string()))?;
    }
    Ok(())
}

fn list_owned(
    prefix: &[&str],
    ownership: &ComposeOwnership,
) -> Result<Vec<String>, CommandFailure> {
    let mut command = docker_command(prefix);
    for filter in ownership.label_filters() {
        command.args([FILTER_FLAG, &format!("label={filter}")]);
    }
    let output = require_success(run(command)?, "COMPOSE_INVENTORY_FAILED")?;
    output_lines(&output.stdout)
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

fn verify_inventory_ownership(
    inventory: &ResourceInventory,
    ownership: &ComposeOwnership,
    context: &RuntimeContext,
) -> Result<(), CommandFailure> {
    let mut mounted_volumes = Vec::new();
    for id in &inventory.containers {
        verify_labels(
            &["inspect", FORMAT_FLAG, "{{json .Config.Labels}}", id],
            ownership,
            context,
        )?;
        mounted_volumes.extend(container_mounts(id)?);
    }
    for id in &inventory.networks {
        verify_labels(
            &["network", "inspect", FORMAT_FLAG, "{{json .Labels}}", id],
            ownership,
            context,
        )?;
    }
    for volume in &inventory.volumes {
        let output = require_success(
            run(docker_command(&[
                "volume",
                "inspect",
                FORMAT_FLAG,
                "{{json .Labels}}",
                &volume.id,
            ]))?,
            "COMPOSE_OWNERSHIP_CHECK_FAILED",
        )?;
        let labeled = ownership.matches_json(&output.stdout).unwrap_or(false);
        let attached_anonymous =
            volume.logical_key.is_none() && mounted_volumes.contains(&volume.id);
        if !labeled && !attached_anonymous {
            return Err(recovery(context, "COMPOSE_OWNERSHIP_MISMATCH"));
        }
    }
    Ok(())
}

fn verify_labels(
    args: &[&str],
    ownership: &ComposeOwnership,
    context: &RuntimeContext,
) -> Result<(), CommandFailure> {
    let output = require_success(run(docker_command(args))?, "COMPOSE_OWNERSHIP_CHECK_FAILED")?;
    let matches = ownership
        .matches_json(&output.stdout)
        .map_err(|_| recovery(context, "COMPOSE_OWNERSHIP_EVIDENCE_INVALID"))?;
    if matches {
        Ok(())
    } else {
        Err(recovery(context, "COMPOSE_OWNERSHIP_MISMATCH"))
    }
}

fn verify_absent(
    inventory: &ResourceInventory,
    preserved: &[String],
    context: &RuntimeContext,
) -> Result<(), CommandFailure> {
    for (kind, id) in inventory
        .containers
        .iter()
        .map(|id| ("container", id.as_str()))
        .chain(inventory.networks.iter().map(|id| ("network", id.as_str())))
        .chain(
            inventory
                .deletable_volumes(preserved)
                .iter()
                .map(|id| ("volume", id.as_str())),
        )
    {
        let args = if kind == "container" {
            vec!["inspect", id]
        } else {
            vec![kind, "inspect", id]
        };
        if run(docker_command(&args))?.status.success() {
            return Err(recovery(context, "COMPOSE_RESOURCE_STILL_PRESENT"));
        }
    }
    Ok(())
}

fn parse_port(env: &str, source: &[u8]) -> Result<ResolvedExport, CommandFailure> {
    let value =
        String::from_utf8(source.to_vec()).map_err(|_| failure("COMPOSE_PORT_INVALID_UTF8"))?;
    let (host, port) = value
        .trim()
        .rsplit_once(':')
        .ok_or_else(|| failure("COMPOSE_PORT_INVALID"))?;
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| failure("COMPOSE_PORT_INVALID"))?;
    if host != "127.0.0.1" {
        return Err(failure("COMPOSE_PORT_NOT_LOOPBACK"));
    }
    Ok(ResolvedExport {
        env: env.to_string(),
        host: host.to_string(),
        port,
    })
}

fn inventory(layout: &StateLayout) -> Result<ResourceInventory, CommandFailure> {
    read_json(&layout.inventory).map_err(|error| failure(error.to_string()))
}

fn persist(layout: &StateLayout, inventory: &ResourceInventory) -> Result<(), CommandFailure> {
    write_json_atomic(&layout.inventory, inventory).map_err(|error| failure(error.to_string()))
}

fn enabled_plan(plan: Option<&ComposePlan>) -> Option<&ComposePlan> {
    plan.filter(|plan| plan.isolation == ComposeIsolation::Managed)
}

fn ownership(identity: &EnvironmentIdentity, digest: &str) -> ComposeOwnership {
    ComposeOwnership {
        environment_id: identity.environment_id.clone(),
        owner_key: identity.owner_key.clone(),
        plan_digest: digest.to_string(),
    }
}

fn recovery(context: &RuntimeContext, code: &str) -> CommandFailure {
    failure(format!(
        "{code}: recovery: autospec runtime env down {}repo '{}' {}mode {}",
        concat!("-", "-"),
        context.repo.display(),
        concat!("-", "-"),
        context.mode.name()
    ))
}

fn failure(message: impl Into<String>) -> CommandFailure {
    CommandFailure::diagnostic(message.into())
}
