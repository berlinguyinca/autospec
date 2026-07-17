use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};

use autospec_core::runtime_env::{
    EnvironmentLifecycle, ResourcePlan, RuntimeContext, RuntimeManifest, RuntimeState,
};

use crate::commands::CommandFailure;

mod compose;
mod isolation;
mod lifecycle;
mod maven;
mod session;
mod state;
mod worker;

use isolation::{
    bypass_without_planning, invocation_isolation, planning_identity, whole_environment_disabled,
};
use lifecycle::{
    partial_state, provision_locked, teardown_locked, validate_authoritative,
    validate_teardown_lifecycle,
};
use maven::MavenAdapter;
use session::{live_sessions, run_session_command, SessionLease};
use state::{
    layout_for_context, read_authoritative_state, read_runtime_state, write_runtime_state,
    EnvironmentLease, StateLayout,
};

#[cfg(unix)]
static RECEIVED_SIGNAL: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn record_signal(signal: i32) {
    RECEIVED_SIGNAL.store(signal, Ordering::Relaxed);
}

// SECURITY: fixed POSIX signal numbers and an atomic-only handler.
#[cfg(unix)]
extern "C" {
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}

#[derive(Debug, Clone)]
struct Options {
    repo: PathBuf,
    mode: String,
}

#[derive(Debug, Clone, Copy)]
enum ManifestKind {
    Agent,
    Autospec,
}

#[derive(Debug, Clone)]
struct InitOptions {
    repo: PathBuf,
    manifest: ManifestKind,
    force: bool,
}

#[derive(Debug, Clone)]
struct ExecOptions {
    options: Options,
    command: Vec<String>,
}

#[derive(Debug, Clone)]
struct SessionOptions {
    options: Options,
    command: Vec<String>,
    keep_alive: bool,
}

#[derive(Debug, Clone)]
struct DownOptions {
    options: Options,
    purge_maven: bool,
}

pub(super) fn run(args: &[String]) -> Result<(), CommandFailure> {
    let Some((operation, options)) = args.split_first() else {
        return Err(CommandFailure::diagnostic(
            "autospec runtime env requires a subcommand",
        ));
    };
    match operation.as_str() {
        "init" => init(parse_init_options(options)?),
        "up" => up(parse_options(options)?),
        "status" => status(parse_options(options)?),
        "down" => down(parse_down_options(options)?),
        "exec" => exec(parse_exec_options(options)?),
        "session" => session(parse_session_options(options)?),
        "lease-probe" => lease_probe(options),
        "-h" | "--help" | "help" => {
            print_help();
            Ok(())
        }
        _ => Err(CommandFailure::diagnostic(format!(
            "unknown autospec runtime env command: {operation}"
        ))),
    }
}

fn parse_options(args: &[String]) -> Result<Options, CommandFailure> {
    let mut repo = PathBuf::from(".");
    let mut mode = "auto".to_string();
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--repo" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    CommandFailure::diagnostic("autospec runtime env --repo requires a path")
                })?;
                if value.is_empty() || value.starts_with("--") {
                    return Err(CommandFailure::diagnostic(
                        "autospec runtime env --repo requires a path",
                    ));
                }
                repo = PathBuf::from(value);
                index += 2;
            }
            "--mode" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    CommandFailure::diagnostic("autospec runtime env --mode requires a value")
                })?;
                if value.is_empty() {
                    return Err(CommandFailure::diagnostic(
                        "autospec runtime env --mode requires a value",
                    ));
                }
                if value.starts_with("--") {
                    return Err(CommandFailure::diagnostic(
                        "autospec runtime env --mode requires a value",
                    ));
                }
                mode = value.clone();
                index += 2;
            }
            _ if argument.starts_with("--repo=") => {
                let value = argument.trim_start_matches("--repo=");
                if value.is_empty() {
                    return Err(CommandFailure::diagnostic(
                        "autospec runtime env --repo requires a path",
                    ));
                }
                repo = PathBuf::from(value);
                index += 1;
            }
            _ if argument.starts_with("--mode=") => {
                let value = argument.trim_start_matches("--mode=");
                if value.is_empty() {
                    return Err(CommandFailure::diagnostic(
                        "autospec runtime env --mode requires a value",
                    ));
                }
                mode = value.to_string();
                index += 1;
            }
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec runtime env option: {argument}"
                )));
            }
        }
    }
    Ok(Options { repo, mode })
}

fn parse_down_options(args: &[String]) -> Result<DownOptions, CommandFailure> {
    let mut purge_maven = false;
    let options = args
        .iter()
        .filter(|argument| {
            if argument.as_str() == "--purge-maven" {
                purge_maven = true;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect::<Vec<_>>();
    Ok(DownOptions {
        options: parse_options(&options)?,
        purge_maven,
    })
}

fn parse_init_options(args: &[String]) -> Result<InitOptions, CommandFailure> {
    let mut repo = PathBuf::from(".");
    let mut manifest = ManifestKind::Agent;
    let mut force = false;
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--repo" => {
                let value = option_value(args, index, "--repo", "path")?;
                repo = PathBuf::from(value);
                index += 2;
            }
            "--manifest" => {
                manifest = parse_manifest_kind(option_value(args, index, "--manifest", "value")?)?;
                index += 2;
            }
            "--force" => {
                force = true;
                index += 1;
            }
            _ if argument.starts_with("--repo=") => {
                repo = PathBuf::from(equals_option_value(argument, "--repo", "path")?);
                index += 1;
            }
            _ if argument.starts_with("--manifest=") => {
                manifest =
                    parse_manifest_kind(equals_option_value(argument, "--manifest", "value")?)?;
                index += 1;
            }
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec runtime env init option: {argument}"
                )));
            }
        }
    }
    Ok(InitOptions {
        repo,
        manifest,
        force,
    })
}

fn parse_exec_options(args: &[String]) -> Result<ExecOptions, CommandFailure> {
    let mut options = Options {
        repo: PathBuf::from("."),
        mode: "auto".to_string(),
    };
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "--repo" => {
                options.repo = PathBuf::from(option_value(args, index, "--repo", "path")?);
                index += 2;
            }
            "--mode" => {
                options.mode = option_value(args, index, "--mode", "value")?.to_string();
                index += 2;
            }
            _ if argument.starts_with("--repo=") => {
                options.repo = PathBuf::from(equals_option_value(argument, "--repo", "path")?);
                index += 1;
            }
            _ if argument.starts_with("--mode=") => {
                options.mode = equals_option_value(argument, "--mode", "value")?.to_string();
                index += 1;
            }
            _ if argument.starts_with('-') => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec runtime env exec option: {argument}"
                )));
            }
            _ => break,
        }
    }
    let command = args[index..].to_vec();
    if command.is_empty() {
        return Err(CommandFailure::diagnostic(
            "autospec runtime env exec requires a command after --",
        ));
    }
    Ok(ExecOptions { options, command })
}

fn parse_session_options(args: &[String]) -> Result<SessionOptions, CommandFailure> {
    let mut options = Options {
        repo: PathBuf::from("."),
        mode: "auto".to_string(),
    };
    let mut keep_alive = false;
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "--repo" => {
                options.repo = PathBuf::from(option_value(args, index, "--repo", "path")?);
                index += 2;
            }
            "--mode" => {
                options.mode = option_value(args, index, "--mode", "value")?.to_string();
                index += 2;
            }
            "--keep-alive" => {
                keep_alive = true;
                index += 1;
            }
            _ if argument.starts_with("--repo=") => {
                options.repo = PathBuf::from(equals_option_value(argument, "--repo", "path")?);
                index += 1;
            }
            _ if argument.starts_with("--mode=") => {
                options.mode = equals_option_value(argument, "--mode", "value")?.to_string();
                index += 1;
            }
            _ if argument.starts_with('-') => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec runtime env session option: {argument}"
                )));
            }
            _ => break,
        }
    }
    let command = args[index..].to_vec();
    if command.is_empty() {
        return Err(CommandFailure::diagnostic(
            "autospec runtime env session requires a command after --",
        ));
    }
    Ok(SessionOptions {
        options,
        command,
        keep_alive,
    })
}

fn option_value<'a>(
    args: &'a [String],
    index: usize,
    option: &str,
    noun: &str,
) -> Result<&'a str, CommandFailure> {
    let value = args.get(index + 1).map(String::as_str).ok_or_else(|| {
        CommandFailure::diagnostic(format!("autospec runtime env {option} requires a {noun}"))
    })?;
    if value.is_empty() || value.starts_with("--") {
        return Err(CommandFailure::diagnostic(format!(
            "autospec runtime env {option} requires a {noun}"
        )));
    }
    Ok(value)
}

fn equals_option_value<'a>(
    argument: &'a str,
    option: &str,
    noun: &str,
) -> Result<&'a str, CommandFailure> {
    let value = argument
        .strip_prefix(&format!("{option}="))
        .unwrap_or_default();
    if value.is_empty() {
        return Err(CommandFailure::diagnostic(format!(
            "autospec runtime env {option} requires a {noun}"
        )));
    }
    Ok(value)
}

fn parse_manifest_kind(value: &str) -> Result<ManifestKind, CommandFailure> {
    match value {
        "agent" => Ok(ManifestKind::Agent),
        "autospec" => Ok(ManifestKind::Autospec),
        _ => Err(CommandFailure::diagnostic(format!(
            "agent-env: unknown manifest kind: {value}"
        ))),
    }
}

fn init(options: InitOptions) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.repo)?;
    let target = init_manifest(&repo, options.manifest, options.force)?;
    println!("agent-env: created {}", target.display());
    Ok(())
}

fn up(options: Options) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.repo)?;
    let invocation = invocation_isolation(&repo, &options.mode)?;
    if invocation.whole_environment_disabled {
        println!("AUTOSPEC_ISOLATION_BYPASSED=1");
        return Ok(());
    }
    let plan = invocation
        .plan
        .expect("enabled runtime invocation has a plan");
    let context = context_from_plan(&repo, &options.mode, &plan)?;
    let state = provision(&context, &plan, invocation.bypassed)?;
    print_protocol(&context, &state, invocation.bypassed);
    Ok(())
}

fn status(options: Options) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.repo)?;
    let identity = planning_identity(&repo, &options.mode)?;
    let context = context_from_identity(&repo, &options.mode, &identity)?;
    if !context.environment_dir.exists() {
        return inactive(&context);
    }
    let _lease = EnvironmentLease::acquire(&context.environment_dir)?;
    let layout = layout_for_context(&context);
    let authoritative = match read_authoritative_state(&layout)? {
        Some(state) => state,
        None if !context.env_file.is_file() => return inactive(&context),
        None => return Err(partial_state(&layout)),
    };
    let invocation = invocation_isolation(&repo, &options.mode)?;
    let plan = required_plan(invocation.plan)?;
    validate_authoritative(&authoritative, &plan)?;
    if authoritative.owner.lifecycle != EnvironmentLifecycle::Active {
        return Err(CommandFailure::diagnostic(format!(
            "RUNTIME_LIFECYCLE_MISMATCH: environment is {:?}",
            authoritative.owner.lifecycle
        )));
    }
    if !context.env_file.is_file() {
        return Err(partial_state(&layout));
    }
    let state = read_state(&context)?;
    print_protocol(
        &context,
        &state,
        bypass_without_planning(whole_environment_disabled()?)?,
    );
    Ok(())
}

fn down(options: DownOptions) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.options.repo)?;
    let identity = planning_identity(&repo, &options.options.mode)?;
    let context = context_from_identity(&repo, &options.options.mode, &identity)?;
    if !context.environment_dir.exists() {
        return Ok(());
    }
    let _lease = EnvironmentLease::acquire(&context.environment_dir)?;
    let layout = layout_for_context(&context);
    let authoritative = match read_authoritative_state(&layout)? {
        Some(state) => state,
        None if !context.env_file.is_file() => return Ok(()),
        None => return Err(partial_state(&layout)),
    };
    let invocation = invocation_isolation(&repo, &options.options.mode)?;
    let plan = required_plan(invocation.plan)?;
    validate_authoritative(&authoritative, &plan)?;
    validate_teardown_lifecycle(&authoritative.owner.lifecycle)?;
    let live = live_sessions(&context.environment_dir)?;
    if !live.is_empty() {
        return Err(CommandFailure::diagnostic(format!(
            "RUNTIME_LIVE_SESSIONS: {} live runtime session(s) prevent teardown",
            live.len()
        )));
    }
    let state = if context.env_file.is_file() {
        Some(read_state(&context)?)
    } else {
        None
    };
    if options.purge_maven {
        MavenAdapter::purge_owned_prefix(
            plan.maven.as_ref(),
            &context,
            state.as_ref().ok_or_else(|| partial_state(&layout))?,
            &layout,
            &authoritative.inventory,
        )?;
    }
    teardown_locked(&context, state.as_ref(), &plan)
}

fn exec(options: ExecOptions) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.options.repo)?;
    let invocation = invocation_isolation(&repo, &options.options.mode)?;
    if invocation.whole_environment_disabled {
        return run_direct_command(&options.command, &repo, None, true);
    }
    let plan = invocation
        .plan
        .expect("enabled runtime invocation has a plan");
    let context = context_from_plan(&repo, &options.options.mode, &plan)?;
    let state = provision(&context, &plan, invocation.bypassed)?;
    run_direct_command(
        &options.command,
        &context.repo,
        Some((&context, &state)),
        invocation.bypassed,
    )
}

fn session(options: SessionOptions) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.options.repo)?;
    let whole_environment_disabled = whole_environment_disabled()?;
    if whole_environment_disabled {
        bypass_without_planning(true)?;
        return run_direct_command(&options.command, &repo, None, true);
    }
    if selected_manifest(&repo).is_none() {
        if environment_flag("AUTOSPEC_ENV_AUTO_INIT") {
            init_manifest(&repo, ManifestKind::Agent, false)?;
        } else {
            let bypassed = bypass_without_planning(false)?;
            return run_direct_command(&options.command, &repo, None, bypassed);
        }
    }
    if !worker::consume_session_worker_handoff() {
        return worker::supervise_session_worker(
            &options.command,
            &repo,
            &options.options.mode,
            options.keep_alive,
        );
    }
    let invocation = invocation_isolation(&repo, &options.options.mode)?;
    let plan = invocation
        .plan
        .expect("enabled runtime invocation has a plan");
    let context = context_from_plan(&repo, &options.options.mode, &plan)?;
    let lease = EnvironmentLease::acquire(&context.environment_dir)?;
    let state = provision_locked(&context, &plan, invocation.bypassed)?;
    let harness = options
        .command
        .first()
        .expect("session command is not empty");
    reset_session_signal_handlers();
    let session_lease = SessionLease::register(&context.environment_dir, harness)?;
    drop(lease);
    let keep_alive = options.keep_alive || environment_flag("AUTOSPEC_ENV_KEEP_ALIVE");
    run_session_command(
        &options.command,
        &context,
        &state,
        &plan,
        session_lease,
        keep_alive,
        invocation.bypassed,
    )
}

fn context_from_plan(
    repo: &Path,
    mode: &str,
    plan: &ResourcePlan,
) -> Result<RuntimeContext, CommandFailure> {
    context_from_identity(repo, mode, &plan.identity)
}

fn context_from_identity(
    repo: &Path,
    mode: &str,
    identity: &autospec_core::runtime_env::EnvironmentIdentity,
) -> Result<RuntimeContext, CommandFailure> {
    let manifest = RuntimeManifest::read_from_repo(repo)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let root = state_root()?;
    let layout = StateLayout::new(&root, &identity.environment_id);
    let context = RuntimeContext::new_with_identity(manifest, repo, mode, &root, identity)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    debug_assert_eq!(context.environment_dir, layout.environment_dir);
    Ok(context)
}

fn canonical_repo(repo: &Path) -> Result<PathBuf, CommandFailure> {
    fs::canonicalize(repo).map_err(|error| {
        CommandFailure::diagnostic(format!("repo does not exist: {} ({error})", repo.display()))
    })
}

fn init_manifest(repo: &Path, kind: ManifestKind, force: bool) -> Result<PathBuf, CommandFailure> {
    if let Some(existing) = selected_manifest(repo) {
        if !force {
            return Err(CommandFailure::status(
                format!(
                    "agent-env: runtime manifest already exists: {}",
                    existing.display()
                ),
                4,
            ));
        }
    }
    let target = match kind {
        ManifestKind::Agent => repo.join(".agent-runtime.yml"),
        ManifestKind::Autospec => repo.join(".autospec/runtime.yml"),
    };
    let name = repo
        .file_name()
        .and_then(|name| name.to_str())
        .map(slugify)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "agent_env".to_string());
    let parent = target.parent().expect("runtime manifest target has parent");
    fs::create_dir_all(parent).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not create runtime manifest directory {}: {error}",
            parent.display()
        ))
    })?;
    fs::write(
        &target,
        format!(
            "version: 1\nname: {name}\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\n    down: sh -c 'true'\nports:\n  frontend:\n    env: AGENT_FRONTEND_PORT\n    default: dynamic\npublic_url_env:\n  - AUTOSPEC_PUBLIC_URL\n  - AGENT_PUBLIC_URL\n"
        ),
    )
    .map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not write runtime manifest {}: {error}",
            target.display()
        ))
    })?;
    Ok(target)
}

fn selected_manifest(repo: &Path) -> Option<PathBuf> {
    let autospec = repo.join(".autospec/runtime.yml");
    if autospec.is_file() {
        return Some(autospec);
    }
    let agent = repo.join(".agent-runtime.yml");
    agent.is_file().then_some(agent)
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_underscore = false;
    for character in value.chars() {
        let next = if character.is_ascii_uppercase() {
            character.to_ascii_lowercase()
        } else if character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '_'
        {
            character
        } else {
            '_'
        };
        if next == '_' && previous_was_underscore {
            continue;
        }
        slug.push(next);
        previous_was_underscore = next == '_';
    }
    slug.trim_matches('_').to_string()
}

fn state_root() -> Result<PathBuf, CommandFailure> {
    if let Some(root) = std::env::var_os("AGENT_ENV_STATE_ROOT").filter(|root| !root.is_empty()) {
        return Ok(PathBuf::from(root));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        CommandFailure::diagnostic("HOME is required for runtime environment state")
    })?;
    Ok(PathBuf::from(home).join(".autospec/envs"))
}

fn free_port() -> Result<u16, CommandFailure> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| {
        CommandFailure::diagnostic(format!("could not allocate runtime port: {error}"))
    })?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not read runtime port: {error}"))
        })
}

pub(super) fn state_from_context(context: &RuntimeContext) -> Result<RuntimeState, CommandFailure> {
    let frontend_override = caller_override("AGENT_FRONTEND_PORT");
    let backend_override = caller_override("AGENT_BACKEND_PORT");
    let mut state = RuntimeState::from_context(
        context,
        if frontend_override.is_some() {
            0
        } else {
            free_port()?
        },
        if backend_override.is_some() {
            0
        } else {
            free_port()?
        },
    );
    if let Some(value) = frontend_override {
        replace_state_value(&mut state, "AGENT_FRONTEND_PORT", value)?;
    }
    if let Some(value) = backend_override {
        replace_state_value(&mut state, "AGENT_BACKEND_PORT", value)?;
    }

    let public_url = caller_override("AGENT_PUBLIC_URL").unwrap_or_else(|| {
        format!(
            "http://127.0.0.1:{}",
            state
                .value("AGENT_FRONTEND_PORT")
                .expect("new runtime state includes the frontend port")
        )
    });
    replace_state_value(&mut state, "AGENT_PUBLIC_URL", public_url.clone())?;
    replace_state_value(
        &mut state,
        "AUTOSPEC_PUBLIC_URL",
        caller_override("AUTOSPEC_PUBLIC_URL").unwrap_or(public_url),
    )?;
    if let Some(value) = caller_override("COMPOSE_PROJECT_NAME") {
        replace_state_value(&mut state, "COMPOSE_PROJECT_NAME", value)?;
    }
    Ok(state)
}

fn caller_override(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

fn replace_state_value(
    state: &mut RuntimeState,
    key: &str,
    value: String,
) -> Result<(), CommandFailure> {
    state
        .replace_existing_value(key, value)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn provision(
    context: &RuntimeContext,
    plan: &ResourcePlan,
    bypassed: bool,
) -> Result<RuntimeState, CommandFailure> {
    let _lease = EnvironmentLease::acquire(&context.environment_dir)?;
    provision_locked(context, plan, bypassed)
}

fn inactive(context: &RuntimeContext) -> Result<(), CommandFailure> {
    Err(CommandFailure::status(
        format!(
            "agent-env: no active environment for {} mode {}",
            context.repo.display(),
            context.mode.name()
        ),
        3,
    ))
}

fn required_plan(plan: Option<ResourcePlan>) -> Result<ResourcePlan, CommandFailure> {
    plan.ok_or_else(|| {
        CommandFailure::diagnostic(
            "RUNTIME_PLAN_MISMATCH: disabled runtime cannot authenticate persisted state",
        )
    })
}

pub(super) fn missing_mode_command(context: &RuntimeContext) -> CommandFailure {
    CommandFailure::status(
        format!(
            "agent-env: mode '{}' has no command in {}",
            context.mode.name(),
            context.manifest.path().display()
        ),
        1,
    )
}

pub(super) fn write_state(
    context: &RuntimeContext,
    state: &RuntimeState,
) -> Result<(), CommandFailure> {
    write_runtime_state(context, state)
}

pub(super) fn read_state(context: &RuntimeContext) -> Result<RuntimeState, CommandFailure> {
    read_runtime_state(context)
}

fn lease_probe(args: &[String]) -> Result<(), CommandFailure> {
    let [environment_dir, ready_path, release_path] = args else {
        return Err(CommandFailure::diagnostic(
            "autospec runtime env lease-probe requires ENVIRONMENT READY RELEASE",
        ));
    };
    let _lease = EnvironmentLease::acquire(Path::new(environment_dir))?;
    fs::write(ready_path, "ready\n").map_err(|error| {
        CommandFailure::diagnostic(format!("could not mark runtime lease probe ready: {error}"))
    })?;
    while !Path::new(release_path).exists() {
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn run_direct_command(
    command: &[String],
    repo: &Path,
    runtime: Option<(&RuntimeContext, &RuntimeState)>,
    bypassed: bool,
) -> Result<(), CommandFailure> {
    let mut child = worker::spawn_direct_command(command, repo, runtime, bypassed, false)?;
    let status = child.wait().map_err(|error| {
        CommandFailure::diagnostic(format!("could not wait for runtime child command: {error}"))
    })?;
    child_status(status)
}

fn environment_flag(key: &str) -> bool {
    matches!(std::env::var(key).as_deref(), Ok("1"))
}

pub(super) fn run_mode_command(
    command: Option<&str>,
    context: &RuntimeContext,
    state: Option<&RuntimeState>,
    bypassed: bool,
) -> Result<(), CommandFailure> {
    let Some(command) = command else {
        return Ok(());
    };
    let mut process = Command::new("sh");
    process.args(["-c", command]).current_dir(&context.repo);
    worker::scrub_external_environment(&mut process);
    if let Some(state) = state {
        worker::configure_runtime_environment(&mut process, context, state, bypassed);
    }
    let status = process.status().map_err(|error| {
        CommandFailure::diagnostic(format!("could not run runtime manifest command: {error}"))
    })?;
    child_status(status)
}

fn child_status(status: std::process::ExitStatus) -> Result<(), CommandFailure> {
    if status.success() {
        Ok(())
    } else {
        #[cfg(unix)]
        let code = {
            use std::os::unix::process::ExitStatusExt;
            status
                .code()
                .or_else(|| status.signal().map(|signal| 128 + signal))
                .unwrap_or(1)
        };
        #[cfg(not(unix))]
        let code = status.code().unwrap_or(1);
        Err(CommandFailure::status(String::new(), code))
    }
}

pub(super) fn reset_session_signal_handlers() {
    #[cfg(unix)]
    install_signal_handlers();
}

pub(super) fn received_session_signal() -> i32 {
    #[cfg(unix)]
    return RECEIVED_SIGNAL.load(Ordering::Relaxed);
    #[cfg(not(unix))]
    0
}

#[cfg(unix)]
fn install_signal_handlers() {
    RECEIVED_SIGNAL.store(0, Ordering::Relaxed);
    // SAFETY: SIGINT/SIGTERM are fixed and the handler only writes an atomic.
    unsafe {
        signal(2, record_signal);
        signal(15, record_signal);
    }
}

fn print_protocol(context: &RuntimeContext, state: &RuntimeState, bypassed: bool) {
    print_state_value(state, "AGENT_ENV_ID");
    print_state_value(state, "AGENT_ENV_MODE");
    print_state_value(state, "AGENT_ENV_REPO");
    println!("AGENT_ENV_FILE={}", context.env_file.display());
    print_state_value(state, "AGENT_FRONTEND_PORT");
    print_state_value(state, "AGENT_BACKEND_PORT");
    print_state_value(state, "AGENT_PUBLIC_URL");
    print_state_value(state, "AUTOSPEC_PUBLIC_URL");
    print_state_value(state, "COMPOSE_PROJECT_NAME");
    for (key, _) in context.mode.env() {
        print_state_value(state, key);
    }
    if bypassed {
        println!("AUTOSPEC_ISOLATION_BYPASSED=1");
    }
}

fn print_state_value(state: &RuntimeState, key: &str) {
    if let Some(value) = state.value(key) {
        println!("{key}={value}");
    }
}

fn print_help() {
    println!(
        "autospec runtime env\n\nUSAGE:\n    autospec runtime env init [--repo PATH] [--manifest agent|autospec] [--force]\n    autospec runtime env up [--repo PATH] [--mode MODE]\n    autospec runtime env status [--repo PATH] [--mode MODE]\n    autospec runtime env down [--repo PATH] [--mode MODE] [--purge-maven]\n    autospec runtime env exec [--repo PATH] [--mode MODE] -- COMMAND [ARGS...]\n    autospec runtime env session [--repo PATH] [--mode MODE] [--keep-alive] -- COMMAND [ARGS...]"
    );
}
