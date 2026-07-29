use std::fs;
use std::path::Path;
use std::time::Duration;

use autospec_core::runtime_env::{
    wait_for_loopback_bind, ComposeIsolation, EnvironmentLifecycle, EnvironmentOwner,
    MavenIsolation, MavenPlan, ResourceInventory, ResourcePlan, RuntimeContext, RuntimeState,
};

use crate::commands::CommandFailure;

use super::compose::ComposeAdapter;
use super::maven::MavenAdapter;
use super::session::live_sessions;
use super::state::{
    initialize_authoritative_state, layout_for_context, read_authoritative_state, release_ports,
    write_lifecycle, PortReservations, StateLayout,
};

const INITIAL_OVERRIDE_KEYS: [&str; 5] = [
    "AGENT_FRONTEND_PORT",
    "AGENT_BACKEND_PORT",
    "AGENT_PUBLIC_URL",
    "AUTOSPEC_PUBLIC_URL",
    "COMPOSE_PROJECT_NAME",
];

const DIRECT_BIND_TIMEOUT: Duration = Duration::from_secs(2);
const DIRECT_LAUNCH_ATTEMPTS: usize = 5;

pub(super) fn provision_locked(
    context: &RuntimeContext,
    plan: &ResourcePlan,
    bypassed: bool,
) -> Result<RuntimeState, CommandFailure> {
    let layout = layout_for_context(context);
    ComposeAdapter::reject_caller_project_name(plan.compose.as_ref())?;
    if let Some(authoritative) = read_authoritative_state(&layout)? {
        validate_authoritative(&authoritative, plan)?;
        if active_state_matches(&authoritative, context) {
            let cached = super::read_state(context)?;
            return validate_cached_state(context, plan, &authoritative.inventory, &cached);
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
    let managed_compose = desired
        .compose
        .as_ref()
        .is_some_and(|compose| compose.isolation == ComposeIsolation::Managed);
    if inventory_has_teardown_blockers(&authoritative.inventory) && !managed_compose {
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
    if let Err(error) =
        ComposeAdapter::down_owned(desired.compose.as_ref(), desired, context, &layout)
    {
        return cleanup_failed(&layout, Some(&mut owner), error);
    }
    let retained_inventory: ResourceInventory = match super::state::read_json(&layout.inventory) {
        Ok(inventory) => inventory,
        Err(error) => {
            return cleanup_failed(
                &layout,
                Some(&mut owner),
                CommandFailure::diagnostic(error.to_string()),
            )
        }
    };
    if retained_inventory.maven_local_prefix.is_some() {
        write_lifecycle(&layout, &mut owner, EnvironmentLifecycle::Active)?;
        return Ok(());
    }
    let state_root = layout
        .environment_dir
        .parent()
        .expect("runtime environment has a state root");
    release_ports(state_root, &context.environment_id)?;
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

pub(super) fn validate_cached_state(
    context: &RuntimeContext,
    plan: &ResourcePlan,
    inventory: &ResourceInventory,
    cached: &RuntimeState,
) -> Result<RuntimeState, CommandFailure> {
    cached
        .validate_child_environment(context)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let frontend = required_inventory_port(inventory.frontend_port, "AGENT_FRONTEND_PORT")?;
    let backend = required_inventory_port(inventory.backend_port, "AGENT_BACKEND_PORT")?;
    let mut expected = RuntimeState::from_context(context, frontend, backend);
    restore_initial_overrides(inventory, &mut expected)?;
    restore_maven_state(plan, inventory, context, &mut expected)?;
    restore_compose_state(plan, inventory, &mut expected)?;
    for (key, expected_value) in expected.values() {
        if cached.value(key) != Some(expected_value) {
            return Err(CommandFailure::diagnostic(format!(
                "RUNTIME_CHILD_ENV_VALUE_MISMATCH: {key}"
            )));
        }
    }
    for (key, cached_value) in cached.values() {
        if expected.value(key) != Some(cached_value) {
            return Err(CommandFailure::diagnostic(format!(
                "RUNTIME_CHILD_ENV_VALUE_MISMATCH: {key}"
            )));
        }
    }
    Ok(expected)
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
        | EnvironmentLifecycle::Provisioning
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
    let command = context
        .mode
        .command()
        .filter(|command| !command.trim().is_empty());
    if command.is_none() && plan.maven.is_none() && plan.compose.is_none() {
        return Err(super::missing_mode_command(context));
    }
    write_lifecycle(layout, &mut owner, EnvironmentLifecycle::Provisioning)?;
    if plan.maven.is_none() && plan.compose.is_none() {
        return provision_direct(context, layout, plan, command, bypassed, owner);
    }
    provision_resources(context, layout, plan, command, bypassed, owner)
}

fn provision_resources(
    context: &RuntimeContext,
    layout: &StateLayout,
    plan: &ResourcePlan,
    command: Option<&str>,
    bypassed: bool,
    mut owner: EnvironmentOwner,
) -> Result<RuntimeState, CommandFailure> {
    ComposeAdapter::validate_canonical_export(plan.compose.as_ref())?;
    let resolved = ComposeAdapter::validate_resolved_model(plan.compose.as_ref(), context)?;
    let (mut state, mut ports) = super::state_from_context(context)?;
    let result = (|| {
        record_allocated_ports(layout, &state)?;
        configure_resources(context, layout, plan, resolved.as_deref(), &mut state)?;
        super::write_state(context, &state)?;
        ports.release_for_launch();
        super::run_mode_command(command, context, Some(&state), bypassed)?;
        write_lifecycle(layout, &mut owner, EnvironmentLifecycle::Active)?;
        Ok(state)
    })();
    match result {
        Ok(state) => Ok(state),
        Err(error) => release_after_failure(layout, &mut owner, ports, error),
    }
}

fn configure_resources(
    context: &RuntimeContext,
    layout: &StateLayout,
    plan: &ResourcePlan,
    resolved_compose: Option<&[u8]>,
    state: &mut RuntimeState,
) -> Result<(), CommandFailure> {
    MavenAdapter::configure(plan.maven.as_ref(), context, state, layout)?;
    record_maven_arguments(layout, plan, state)?;
    ComposeAdapter::up(
        plan.compose.as_ref(),
        plan,
        resolved_compose,
        context,
        state,
        layout,
    )
}

fn provision_direct(
    context: &RuntimeContext,
    layout: &StateLayout,
    _plan: &ResourcePlan,
    command: Option<&str>,
    bypassed: bool,
    mut owner: EnvironmentOwner,
) -> Result<RuntimeState, CommandFailure> {
    for attempt in 0..DIRECT_LAUNCH_ATTEMPTS {
        let (state, mut ports) = super::state_from_context(context)?;
        if let Err(error) = record_allocated_ports(layout, &state)
            .and_then(|()| super::write_state(context, &state))
        {
            return release_after_failure(layout, &mut owner, ports, error);
        }
        let (frontend, backend) = ports.release_for_launch();
        match super::run_mode_command(command, context, Some(&state), bypassed) {
            Ok(()) if wait_for_loopback_bind(frontend, DIRECT_BIND_TIMEOUT) => {
                return activate_direct(layout, &mut owner, ports, state);
            }
            Ok(()) if attempt + 1 < DIRECT_LAUNCH_ATTEMPTS => {
                ports.release_claims()?;
            }
            Ok(()) => {
                let error = CommandFailure::diagnostic(
                    "PORT_BIND_HEALTH_RETRIES_EXHAUSTED: frontend port did not bind after 5 attempts",
                );
                return release_after_failure(layout, &mut owner, ports, error);
            }
            Err(_error)
                if bind_collision(frontend, backend) && attempt + 1 < DIRECT_LAUNCH_ATTEMPTS =>
            {
                ports.release_claims()?;
            }
            Err(error) => return release_after_failure(layout, &mut owner, ports, error),
        }
    }
    unreachable!("bounded direct-server attempts always return")
}

fn bind_collision(frontend: u16, backend: u16) -> bool {
    [frontend, backend]
        .into_iter()
        .any(|port| wait_for_loopback_bind(port, DIRECT_BIND_TIMEOUT))
}

fn activate_direct(
    layout: &StateLayout,
    owner: &mut EnvironmentOwner,
    ports: PortReservations,
    state: RuntimeState,
) -> Result<RuntimeState, CommandFailure> {
    match write_lifecycle(layout, owner, EnvironmentLifecycle::Active) {
        Ok(()) => Ok(state),
        Err(error) => release_after_failure(layout, owner, ports, error),
    }
}

fn release_after_failure<T>(
    layout: &StateLayout,
    owner: &mut EnvironmentOwner,
    ports: PortReservations,
    error: CommandFailure,
) -> Result<T, CommandFailure> {
    ports.release_claims()?;
    cleanup_failed(layout, Some(owner), error)
}

fn required_inventory_port(value: Option<u16>, key: &str) -> Result<u16, CommandFailure> {
    value
        .filter(|port| *port > 0)
        .ok_or_else(|| CommandFailure::diagnostic(format!("RUNTIME_INVENTORY_MISSING: {key}")))
}

fn restore_maven_state(
    plan: &ResourcePlan,
    inventory: &ResourceInventory,
    context: &RuntimeContext,
    state: &mut RuntimeState,
) -> Result<(), CommandFailure> {
    if !plan
        .maven
        .as_ref()
        .is_some_and(|plan| plan.isolation == MavenIsolation::SplitLocal)
    {
        return Ok(());
    }
    let stored = inventory
        .maven_arguments
        .as_deref()
        .ok_or_else(|| CommandFailure::diagnostic("RUNTIME_INVENTORY_MISSING: MAVEN_ARGS"))?;
    let canonical = MavenPlan::arguments(stored, &context.environment_id)
        .map_err(|error| CommandFailure::diagnostic(format!("{}: {}", error.code, error.evidence)))?
        .render()
        .map_err(|error| {
            CommandFailure::diagnostic(format!("{}: {}", error.code, error.evidence))
        })?;
    if canonical != stored {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_INVENTORY_MISMATCH: MAVEN_ARGS is not canonical",
        ));
    }
    state
        .set_value("MAVEN_ARGS", canonical)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn restore_compose_state(
    plan: &ResourcePlan,
    inventory: &ResourceInventory,
    state: &mut RuntimeState,
) -> Result<(), CommandFailure> {
    let Some(compose) = plan
        .compose
        .as_ref()
        .filter(|plan| plan.isolation == ComposeIsolation::Managed)
    else {
        return Ok(());
    };
    if inventory.compose_project.as_deref() != Some(compose.project_name.as_str()) {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_INVENTORY_MISMATCH: COMPOSE_PROJECT_NAME",
        ));
    }
    if inventory.exports.len() != compose.exports.len() {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_INVENTORY_MISMATCH: Compose exports",
        ));
    }
    restore_declared_exports(compose, inventory, state)?;
    restore_canonical_url(compose, inventory, state)
}

fn restore_declared_exports(
    compose: &autospec_core::runtime_env::ComposePlan,
    inventory: &ResourceInventory,
    state: &mut RuntimeState,
) -> Result<(), CommandFailure> {
    for declaration in &compose.exports {
        let resolved = resolved_export(inventory, &declaration.env)?;
        let value = resolved
            .render(declaration)
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
        state
            .set_value(&declaration.env, value)
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    }
    Ok(())
}

fn restore_canonical_url(
    compose: &autospec_core::runtime_env::ComposePlan,
    inventory: &ResourceInventory,
    state: &mut RuntimeState,
) -> Result<(), CommandFailure> {
    let Some(index) = compose
        .canonical_url_export_index()
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?
    else {
        return Ok(());
    };
    let declaration = &compose.exports[index];
    let value = resolved_export(inventory, &declaration.env)?
        .canonical_url(declaration)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    state
        .set_value("AUTOSPEC_PUBLIC_URL", value.clone())
        .and_then(|_| state.set_value("AGENT_PUBLIC_URL", value))
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn resolved_export<'a>(
    inventory: &'a ResourceInventory,
    env: &str,
) -> Result<&'a autospec_core::runtime_env::ResolvedExport, CommandFailure> {
    let matches = inventory
        .exports
        .iter()
        .filter(|resolved| resolved.env == env)
        .collect::<Vec<_>>();
    let [resolved] = matches.as_slice() else {
        return Err(CommandFailure::diagnostic(format!(
            "RUNTIME_INVENTORY_MISMATCH: {env}"
        )));
    };
    Ok(resolved)
}

fn record_allocated_ports(
    layout: &StateLayout,
    state: &RuntimeState,
) -> Result<(), CommandFailure> {
    let mut inventory: ResourceInventory = super::state::read_json(&layout.inventory)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    inventory.frontend_port = Some(parse_state_port(state, "AGENT_FRONTEND_PORT")?);
    inventory.backend_port = Some(parse_state_port(state, "AGENT_BACKEND_PORT")?);
    inventory.initial_overrides = captured_initial_overrides(state)?;
    super::state::write_json_atomic(&layout.inventory, &inventory)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn captured_initial_overrides(
    state: &RuntimeState,
) -> Result<Vec<(String, String)>, CommandFailure> {
    let caller_set = |key: &str| std::env::var(key).is_ok_and(|value| !value.is_empty());
    let public_override = caller_set("AGENT_PUBLIC_URL") || caller_set("AUTOSPEC_PUBLIC_URL");
    INITIAL_OVERRIDE_KEYS
        .into_iter()
        .filter(|key| caller_set(key) || (*key == "AUTOSPEC_PUBLIC_URL" && public_override))
        .map(|key| {
            state
                .value(key)
                .map(|value| (key.to_string(), value.to_string()))
                .ok_or_else(|| CommandFailure::diagnostic(format!("RUNTIME_STATE_INVALID: {key}")))
        })
        .collect()
}

fn restore_initial_overrides(
    inventory: &ResourceInventory,
    state: &mut RuntimeState,
) -> Result<(), CommandFailure> {
    let mut seen = std::collections::BTreeSet::new();
    for (key, value) in &inventory.initial_overrides {
        if !INITIAL_OVERRIDE_KEYS.contains(&key.as_str()) || !seen.insert(key) {
            return Err(CommandFailure::diagnostic(format!(
                "RUNTIME_INVENTORY_MISMATCH: initial override {key}"
            )));
        }
        state
            .replace_existing_value(key, value.clone())
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    }
    Ok(())
}

fn record_maven_arguments(
    layout: &StateLayout,
    plan: &ResourcePlan,
    state: &RuntimeState,
) -> Result<(), CommandFailure> {
    if !plan
        .maven
        .as_ref()
        .is_some_and(|plan| plan.isolation == MavenIsolation::SplitLocal)
    {
        return Ok(());
    }
    let mut inventory: ResourceInventory = super::state::read_json(&layout.inventory)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    inventory.maven_arguments = Some(
        state
            .value("MAVEN_ARGS")
            .ok_or_else(|| CommandFailure::diagnostic("RUNTIME_STATE_INVALID: MAVEN_ARGS"))?
            .to_string(),
    );
    super::state::write_json_atomic(&layout.inventory, &inventory)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn parse_state_port(state: &RuntimeState, key: &str) -> Result<u16, CommandFailure> {
    state
        .value(key)
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port > 0)
        .ok_or_else(|| CommandFailure::diagnostic(format!("RUNTIME_STATE_INVALID: {key}")))
}

fn inventory_has_teardown_blockers(inventory: &ResourceInventory) -> bool {
    inventory.compose_project.is_some()
        || !inventory.containers.is_empty()
        || !inventory.networks.is_empty()
        || !inventory.volumes.is_empty()
        || !inventory.exports.is_empty()
}

fn inventory_is_empty(inventory: &ResourceInventory) -> bool {
    !inventory_has_teardown_blockers(inventory) && inventory.maven_local_prefix.is_none()
}

fn cleanup_failed<T>(
    layout: &StateLayout,
    owner: Option<&mut EnvironmentOwner>,
    error: CommandFailure,
) -> Result<T, CommandFailure> {
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
