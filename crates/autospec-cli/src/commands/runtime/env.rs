use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};

#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};
#[cfg(unix)]
use std::time::Duration;

use autospec_core::runtime_env::{RuntimeContext, RuntimeManifest, RuntimeState};

use crate::commands::CommandFailure;

const STATE_ENVIRONMENT_KEYS: [&str; 9] = [
    "AGENT_ENV_ID",
    "AGENT_ENV_MODE",
    "AGENT_ENV_REPO",
    "AGENT_ENV_MANIFEST",
    "AGENT_FRONTEND_PORT",
    "AGENT_BACKEND_PORT",
    "AGENT_PUBLIC_URL",
    "AUTOSPEC_PUBLIC_URL",
    "COMPOSE_PROJECT_NAME",
];

#[cfg(unix)]
extern "C" {
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}

#[cfg(unix)]
static RECEIVED_SIGNAL: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn record_signal(signal: i32) {
    RECEIVED_SIGNAL.store(signal, Ordering::Relaxed);
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

enum SessionWait {
    Exited(ExitStatus),
    Interrupted(i32),
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
        "down" => down(parse_options(options)?),
        "exec" => exec(parse_exec_options(options)?),
        "session" => session(parse_session_options(options)?),
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
    let context = context(&options)?;
    let state = provision(&context)?;
    print_protocol(&context, &state);
    Ok(())
}

fn status(options: Options) -> Result<(), CommandFailure> {
    let context = context(&options)?;
    if !context.env_file.is_file() {
        return Err(CommandFailure::status(
            format!(
                "agent-env: no active environment for {} mode {}",
                context.repo.display(),
                context.mode.name()
            ),
            3,
        ));
    }
    let state = read_state(&context)?;
    print_protocol(&context, &state);
    Ok(())
}

fn down(options: Options) -> Result<(), CommandFailure> {
    let context = context(&options)?;
    let state = if context.env_file.is_file() {
        Some(read_state(&context)?)
    } else {
        None
    };
    teardown(&context, state.as_ref())
}

fn teardown(context: &RuntimeContext, state: Option<&RuntimeState>) -> Result<(), CommandFailure> {
    run_mode_command(context.mode.down(), context, state)?;
    match fs::remove_dir_all(&context.environment_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "could not remove runtime environment {}: {error}",
            context.environment_dir.display()
        ))),
    }
}

fn exec(options: ExecOptions) -> Result<(), CommandFailure> {
    let context = context(&options.options)?;
    let state = provision(&context)?;
    run_direct_command(&options.command, &context.repo, Some((&context, &state)))
}

fn session(options: SessionOptions) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.options.repo)?;
    if environment_flag("AUTOSPEC_ENV_DISABLE") {
        return run_direct_command(&options.command, &repo, None);
    }
    if selected_manifest(&repo).is_none() {
        if environment_flag("AUTOSPEC_ENV_AUTO_INIT") {
            init_manifest(&repo, ManifestKind::Agent, false)?;
        } else {
            return run_direct_command(&options.command, &repo, None);
        }
    }
    let context = context_from_repo(&repo, &options.options.mode)?;
    let state = provision(&context)?;
    let keep_alive = options.keep_alive || environment_flag("AUTOSPEC_ENV_KEEP_ALIVE");
    run_session_command(&options.command, &context, &state, keep_alive)
}

fn context(options: &Options) -> Result<RuntimeContext, CommandFailure> {
    let repo = canonical_repo(&options.repo)?;
    context_from_repo(&repo, &options.mode)
}

fn context_from_repo(repo: &Path, mode: &str) -> Result<RuntimeContext, CommandFailure> {
    let manifest = RuntimeManifest::read_from_repo(repo)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    RuntimeContext::new(manifest, repo, mode, &state_root()?)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
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
    if let Some(root) = std::env::var_os("AGENT_ENV_STATE_ROOT") {
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

fn state_from_context(context: &RuntimeContext) -> Result<RuntimeState, CommandFailure> {
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

fn provision(context: &RuntimeContext) -> Result<RuntimeState, CommandFailure> {
    if context.env_file.is_file() {
        return read_state(context);
    }
    fs::create_dir_all(&context.environment_dir).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not create runtime environment {}: {error}",
            context.environment_dir.display()
        ))
    })?;
    let state = state_from_context(context)?;
    write_state(context, &state)?;
    let command = context
        .mode
        .command()
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| {
            CommandFailure::status(
                format!(
                    "agent-env: mode '{}' has no command in {}",
                    context.mode.name(),
                    context.manifest.path().display()
                ),
                1,
            )
        })?;
    run_mode_command(Some(command), context, Some(&state))?;
    Ok(state)
}

fn write_state(context: &RuntimeContext, state: &RuntimeState) -> Result<(), CommandFailure> {
    let temporary = context
        .env_file
        .with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, state.render_env_file()).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not write runtime environment {}: {error}",
            temporary.display()
        ))
    })?;
    fs::rename(&temporary, &context.env_file).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not finalize runtime environment {}: {error}",
            context.env_file.display()
        ))
    })
}

fn read_state(context: &RuntimeContext) -> Result<RuntimeState, CommandFailure> {
    let source = fs::read_to_string(&context.env_file).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not read runtime environment {}: {error}",
            context.env_file.display()
        ))
    })?;
    RuntimeState::from_env_file(&source)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn run_direct_command(
    command: &[String],
    repo: &Path,
    runtime: Option<(&RuntimeContext, &RuntimeState)>,
) -> Result<(), CommandFailure> {
    let mut child = spawn_direct_command(command, repo, runtime)?;
    let status = child.wait().map_err(|error| {
        CommandFailure::diagnostic(format!("could not wait for runtime child command: {error}"))
    })?;
    child_status(status)
}

fn spawn_direct_command(
    command: &[String],
    repo: &Path,
    runtime: Option<(&RuntimeContext, &RuntimeState)>,
) -> Result<Child, CommandFailure> {
    let (program, arguments) = command.split_first().ok_or_else(|| {
        CommandFailure::diagnostic("autospec runtime env requires a child command")
    })?;
    let mut child = Command::new(program);
    child.args(arguments).current_dir(repo);
    if let Some((context, state)) = runtime {
        configure_runtime_environment(&mut child, context, state);
    }
    child.spawn().map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not run runtime child command {program}: {error}"
        ))
    })
}

fn run_session_command(
    command: &[String],
    context: &RuntimeContext,
    state: &RuntimeState,
    keep_alive: bool,
) -> Result<(), CommandFailure> {
    #[cfg(unix)]
    install_signal_handlers();

    let session_file = match write_session_record(context, command) {
        Ok(path) => path,
        Err(error) => {
            let _ = teardown(context, Some(state));
            return Err(error);
        }
    };
    let mut child = match spawn_direct_command(command, &context.repo, Some((context, state))) {
        Ok(child) => child,
        Err(error) => {
            let _ = cleanup_session(context, state, &session_file, true);
            return Err(error);
        }
    };
    let result = wait_for_session_child(&mut child);
    let should_teardown = matches!(&result, Ok(SessionWait::Interrupted(_))) || !keep_alive;
    let cleanup = cleanup_session(context, state, &session_file, should_teardown);

    match result {
        Ok(SessionWait::Interrupted(signal)) => {
            cleanup?;
            Err(CommandFailure::status(
                String::new(),
                if signal == 2 { 130 } else { 143 },
            ))
        }
        Ok(SessionWait::Exited(status)) if status.success() => cleanup,
        Ok(SessionWait::Exited(status)) => {
            let _ = cleanup;
            child_status(status)
        }
        Err(error) => {
            let _ = cleanup;
            Err(error)
        }
    }
}

fn write_session_record(
    context: &RuntimeContext,
    command: &[String],
) -> Result<PathBuf, CommandFailure> {
    let sessions = context.environment_dir.join("sessions");
    fs::create_dir_all(&sessions).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not create runtime session directory {}: {error}",
            sessions.display()
        ))
    })?;
    let record = sessions.join(std::process::id().to_string());
    fs::write(
        &record,
        format!(
            "pid={}\ncommand={}\n",
            std::process::id(),
            command.join(" ")
        ),
    )
    .map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not write runtime session record {}: {error}",
            record.display()
        ))
    })?;
    Ok(record)
}

fn cleanup_session(
    context: &RuntimeContext,
    state: &RuntimeState,
    session_file: &Path,
    should_teardown: bool,
) -> Result<(), CommandFailure> {
    match fs::remove_file(session_file) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not remove runtime session record {}: {error}",
                session_file.display()
            )));
        }
    }
    if should_teardown {
        teardown(context, Some(state))?;
    }
    Ok(())
}

fn environment_flag(key: &str) -> bool {
    matches!(std::env::var(key).as_deref(), Ok("1"))
}

#[cfg(unix)]
fn install_signal_handlers() {
    RECEIVED_SIGNAL.store(0, Ordering::Relaxed);
    unsafe {
        signal(2, record_signal);
        signal(15, record_signal);
    }
}

#[cfg(unix)]
fn wait_for_session_child(child: &mut Child) -> Result<SessionWait, CommandFailure> {
    loop {
        let signal = RECEIVED_SIGNAL.load(Ordering::Relaxed);
        if signal == 2 || signal == 15 {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(SessionWait::Interrupted(signal));
        }
        if let Some(status) = child.try_wait().map_err(|error| {
            CommandFailure::diagnostic(format!("could not wait for runtime child command: {error}"))
        })? {
            return Ok(SessionWait::Exited(status));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(unix))]
fn wait_for_session_child(child: &mut Child) -> Result<SessionWait, CommandFailure> {
    child.wait().map(SessionWait::Exited).map_err(|error| {
        CommandFailure::diagnostic(format!("could not wait for runtime child command: {error}"))
    })
}

fn run_mode_command(
    command: Option<&str>,
    context: &RuntimeContext,
    state: Option<&RuntimeState>,
) -> Result<(), CommandFailure> {
    let Some(command) = command else {
        return Ok(());
    };
    let mut process = Command::new("sh");
    process.args(["-c", command]).current_dir(&context.repo);
    if let Some(state) = state {
        configure_runtime_environment(&mut process, context, state);
    }
    let status = process.status().map_err(|error| {
        CommandFailure::diagnostic(format!("could not run runtime manifest command: {error}"))
    })?;
    child_status(status)
}

fn configure_runtime_environment(
    process: &mut Command,
    context: &RuntimeContext,
    state: &RuntimeState,
) {
    for key in STATE_ENVIRONMENT_KEYS {
        if let Some(value) = state.value(key) {
            process.env(key, value);
        }
    }
    for (key, _) in context.mode.env() {
        if let Some(value) = state.value(key) {
            process.env(key, value);
        }
    }
}

fn child_status(status: std::process::ExitStatus) -> Result<(), CommandFailure> {
    if status.success() {
        Ok(())
    } else {
        Err(CommandFailure::status(
            String::new(),
            status.code().unwrap_or(1),
        ))
    }
}

fn print_protocol(context: &RuntimeContext, state: &RuntimeState) {
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
}

fn print_state_value(state: &RuntimeState, key: &str) {
    if let Some(value) = state.value(key) {
        println!("{key}={value}");
    }
}

fn print_help() {
    println!(
        "autospec runtime env\n\nUSAGE:\n    autospec runtime env init [--repo PATH] [--manifest agent|autospec] [--force]\n    autospec runtime env up [--repo PATH] [--mode MODE]\n    autospec runtime env status [--repo PATH] [--mode MODE]\n    autospec runtime env down [--repo PATH] [--mode MODE]\n    autospec runtime env exec [--repo PATH] [--mode MODE] -- COMMAND [ARGS...]\n    autospec runtime env session [--repo PATH] [--mode MODE] [--keep-alive] -- COMMAND [ARGS...]"
    );
}
