use std::fs;
use std::path::Path;

use autospec_core::runtime_env::{
    EnvironmentLifecycle, EnvironmentOwner, ResourceInventory, ResourcePlan, RuntimeContext,
    RuntimeState,
};

use crate::commands::CommandFailure;

use super::session::live_sessions;
use super::state::{
    initialize_authoritative_state, layout_for_context, read_authoritative_state, write_lifecycle,
    StateLayout,
};

pub(super) fn provision_locked(
    context: &RuntimeContext,
    plan: &ResourcePlan,
    bypassed: bool,
) -> Result<RuntimeState, CommandFailure> {
    let layout = layout_for_context(context);
    if let Some(authoritative) = read_authoritative_state(&layout)? {
        validate_authoritative(&authoritative, plan)?;
        if active_state_matches(&authoritative, context) {
            return super::read_state(context);
        }
        reconcile_provisioning(context, &layout, authoritative)?;
    } else if context.env_file.is_file() {
        return Err(partial_state(&layout));
    }
    provision_fresh(context, &layout, plan, bypassed)
}

pub(super) fn teardown_locked(
    context: &RuntimeContext,
    state: Option<&RuntimeState>,
    desired: &ResourcePlan,
) -> Result<(), CommandFailure> {
    let layout = layout_for_context(context);
    let authoritative = read_authoritative_state(&layout)?.ok_or_else(|| partial_state(&layout))?;
    validate_authoritative(&authoritative, desired)?;
    validate_teardown_lifecycle(&authoritative.owner.lifecycle)?;
    let mut owner = authoritative.owner;
    if !inventory_is_empty(&authoritative.inventory) {
        write_lifecycle(&layout, &mut owner, EnvironmentLifecycle::CleanupFailed)?;
        return Err(CommandFailure::diagnostic(
            "RUNTIME_INVENTORY_NOT_EMPTY: refusing cleanup of recorded resources",
        ));
    }
    write_lifecycle(&layout, &mut owner, EnvironmentLifecycle::TearingDown)?;
    if let Some(state) = state {
        if let Err(error) =
            super::run_mode_command(context.mode.down(), context, Some(state), false)
        {
            return cleanup_failed(&layout, Some(&mut owner), error);
        }
    }
    for path in [&layout.env, &layout.inventory, &layout.plan] {
        if let Err(error) = remove_file_if_present(path) {
            return cleanup_failed(&layout, Some(&mut owner), error);
        }
    }
    if let Err(error) = remove_directory_if_present(&layout.sessions) {
        return cleanup_failed(&layout, Some(&mut owner), error);
    }
    if let Err(error) = remove_file_if_present(&layout.owner) {
        return cleanup_failed(&layout, Some(&mut owner), error);
    }
    Ok(())
}

pub(super) fn validate_authoritative(
    state: &super::state::AuthoritativeState,
    desired: &ResourcePlan,
) -> Result<(), CommandFailure> {
    if state.owner.schema_version != 1
        || state.plan.schema_version != 1
        || state.inventory.schema_version != 1
    {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_SCHEMA_MISMATCH: authoritative runtime state requires schema version 1",
        ));
    }
    if state.owner.identity != desired.identity
        || state.inventory.environment_id != desired.identity.environment_id
    {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_OWNER_MISMATCH: persisted runtime identity does not match this environment",
        ));
    }
    if state.owner.manifest_digest != desired.digest || state.plan != *desired {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_PLAN_MISMATCH: persisted runtime plan does not match the requested plan",
        ));
    }
    Ok(())
}

pub(super) fn partial_state(layout: &StateLayout) -> CommandFailure {
    CommandFailure::diagnostic(format!(
        "RUNTIME_PARTIAL_STATE: incomplete runtime state under {}",
        layout.environment_dir.display()
    ))
}

pub(super) fn validate_teardown_lifecycle(
    lifecycle: &EnvironmentLifecycle,
) -> Result<(), CommandFailure> {
    match lifecycle {
        EnvironmentLifecycle::Active
        | EnvironmentLifecycle::TearingDown
        | EnvironmentLifecycle::CleanupFailed => Ok(()),
        other => Err(CommandFailure::diagnostic(format!(
            "RUNTIME_LIFECYCLE_MISMATCH: refusing to tear down {other:?} state"
        ))),
    }
}

fn active_state_matches(
    state: &super::state::AuthoritativeState,
    context: &RuntimeContext,
) -> bool {
    state.owner.lifecycle == EnvironmentLifecycle::Active && context.env_file.is_file()
}

fn reconcile_provisioning(
    context: &RuntimeContext,
    layout: &StateLayout,
    state: super::state::AuthoritativeState,
) -> Result<(), CommandFailure> {
    if state.owner.lifecycle != EnvironmentLifecycle::Provisioning {
        return Err(CommandFailure::diagnostic(format!(
            "RUNTIME_LIFECYCLE_MISMATCH: refusing to reconcile {:?} state",
            state.owner.lifecycle
        )));
    }
    if !inventory_is_empty(&state.inventory) {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_INVENTORY_NOT_EMPTY: refusing to reconcile recorded resources",
        ));
    }
    if !live_sessions(&context.environment_dir)?.is_empty() {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_LIVE_SESSIONS: cannot reconcile partial state with live sessions",
        ));
    }
    let mut owner = state.owner;
    for path in [&context.env_file, &layout.inventory, &layout.plan] {
        if let Err(error) = remove_file_if_present(path) {
            return cleanup_failed(layout, Some(&mut owner), error);
        }
    }
    if let Err(error) = remove_file_if_present(&layout.owner) {
        return cleanup_failed(layout, Some(&mut owner), error);
    }
    Ok(())
}

fn provision_fresh(
    context: &RuntimeContext,
    layout: &StateLayout,
    plan: &ResourcePlan,
    bypassed: bool,
) -> Result<RuntimeState, CommandFailure> {
    let mut owner = initialize_authoritative_state(layout, plan)?;
    let state = super::state_from_context(context)?;
    super::write_state(context, &state)?;
    let command = context
        .mode
        .command()
        .filter(|command| !command.trim().is_empty());
    if command.is_none() && plan.maven.is_none() && plan.compose.is_none() {
        return Err(super::missing_mode_command(context));
    }
    write_lifecycle(layout, &mut owner, EnvironmentLifecycle::Provisioning)?;
    super::run_mode_command(command, context, Some(&state), bypassed)?;
    write_lifecycle(layout, &mut owner, EnvironmentLifecycle::Active)?;
    Ok(state)
}

fn inventory_is_empty(inventory: &ResourceInventory) -> bool {
    inventory.compose_project.is_none()
        && inventory.containers.is_empty()
        && inventory.networks.is_empty()
        && inventory.volumes.is_empty()
        && inventory.exports.is_empty()
        && inventory.maven_local_prefix.is_none()
}

fn cleanup_failed(
    layout: &StateLayout,
    owner: Option<&mut EnvironmentOwner>,
    error: CommandFailure,
) -> Result<(), CommandFailure> {
    if let Some(owner) = owner {
        write_lifecycle(layout, owner, EnvironmentLifecycle::CleanupFailed)?;
    }
    Err(error)
}

fn remove_file_if_present(path: &Path) -> Result<(), CommandFailure> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "could not remove runtime state {}: {error}",
            path.display()
        ))),
    }
}

fn remove_directory_if_present(path: &Path) -> Result<(), CommandFailure> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "could not remove runtime state {}: {error}",
            path.display()
        ))),
    }
}
