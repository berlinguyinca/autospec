use std::path::Path;

use autospec_core::runtime_env::{
    write_json_atomic, ComposeIsolation, ComposeOverride, ComposeOwnership, ComposePlan,
    EnvironmentIdentity, ExportProtocol, ResolvedExport, ResourceInventory, ResourcePlan,
    RuntimeContext, RuntimeState,
};

use crate::commands::CommandFailure;

use super::super::state::{read_json, StateLayout};
use super::ownership;
use super::{compose_command, require_success, run, write_override_atomic};

const OVERRIDE_FILE: &str = "compose.autospec.override.yaml";
const REMOVE_ORPHANS_FLAG: &str = concat!("-", "-remove-orphans");
const PROTOCOL_FLAG: &str = concat!("-", "-protocol");

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
    let mut inventory = inventory(layout)?;
    inventory.compose_project = Some(plan.project_name.clone());
    persist(layout, &inventory)?;
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
    if !output.status.success() {
        let original = require_success(output, "COMPOSE_UP_FAILED")
            .expect_err("nonzero Compose up is an error");
        let _ = ownership::reconcile(plan, &ownership, layout, &mut inventory, context);
        return Err(original);
    }
    ownership::reconcile(plan, &ownership, layout, &mut inventory, context)?;
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
        return Err(ownership::recovery(
            context,
            "COMPOSE_PROJECT_OWNERSHIP_MISMATCH",
        ));
    }
    let ownership = ownership(&resource_plan.identity, &resource_plan.digest);
    ownership::remove_exact_owned(plan, &ownership, layout, &mut inventory, context)?;
    inventory.compose_project = None;
    inventory.containers.clear();
    inventory.networks.clear();
    inventory.volumes.clear();
    inventory.exports.clear();
    persist(layout, &inventory)
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
    let index = plan
        .canonical_url_export_index()
        .map_err(|error| failure(error.to_string()))?;
    let declaration = &plan.exports[index];
    let value = inventory.exports[index]
        .render(declaration)
        .map_err(|error| failure(error.to_string()))?;
    state
        .set_value("AUTOSPEC_PUBLIC_URL", value.clone())
        .map_err(|error| failure(error.to_string()))?;
    state
        .set_value("AGENT_PUBLIC_URL", value)
        .map_err(|error| failure(error.to_string()))?;
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

fn failure(message: impl Into<String>) -> CommandFailure {
    CommandFailure::diagnostic(message.into())
}

#[cfg(test)]
mod tests {
    use super::parse_port;

    #[test]
    fn compose_port_parser_fails_closed_on_noncanonical_output() {
        for value in [
            "0.0.0.0:49152\n",
            "[::]:49152\n",
            "[::1]:49152\n",
            "127.0.0.1:49152\n127.0.0.1:49153\n",
            "0.0.0.0:49152\n[::]:49152\n",
        ] {
            assert!(
                parse_port("WEB_URL", value.as_bytes()).is_err(),
                "{value:?}"
            );
        }
        assert_eq!(
            parse_port("WEB_URL", b"127.0.0.1:49152\n").unwrap().port,
            49152
        );
    }
}
