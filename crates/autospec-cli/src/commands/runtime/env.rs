use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

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

#[derive(Debug, Clone)]
struct Options {
    repo: PathBuf,
    mode: String,
}

pub(super) fn run(args: &[String]) -> Result<(), CommandFailure> {
    let Some((operation, options)) = args.split_first() else {
        return Err(CommandFailure::diagnostic(
            "autospec runtime env requires a subcommand",
        ));
    };
    match operation.as_str() {
        "up" => up(parse_options(options)?),
        "status" => status(parse_options(options)?),
        "down" => down(parse_options(options)?),
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

fn up(options: Options) -> Result<(), CommandFailure> {
    let context = context(&options)?;
    if context.env_file.is_file() {
        let state = read_state(&context)?;
        print_protocol(&context, &state);
        return Ok(());
    }

    fs::create_dir_all(&context.environment_dir).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not create runtime environment {}: {error}",
            context.environment_dir.display()
        ))
    })?;
    let state = RuntimeState::from_context(&context, free_port()?, free_port()?);
    write_state(&context, &state)?;
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
    run_mode_command(Some(command), &context, Some(&state))?;
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
    run_mode_command(context.mode.down(), &context, state.as_ref())?;
    match fs::remove_dir_all(&context.environment_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "could not remove runtime environment {}: {error}",
            context.environment_dir.display()
        ))),
    }
}

fn context(options: &Options) -> Result<RuntimeContext, CommandFailure> {
    let repo = fs::canonicalize(&options.repo).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "repo does not exist: {} ({error})",
            options.repo.display()
        ))
    })?;
    let manifest = RuntimeManifest::read_from_repo(&repo)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    RuntimeContext::new(manifest, &repo, &options.mode, &state_root()?)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
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
    let status = process.status().map_err(|error| {
        CommandFailure::diagnostic(format!("could not run runtime manifest command: {error}"))
    })?;
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
        "autospec runtime env\n\nUSAGE:\n    autospec runtime env up [--repo PATH] [--mode MODE]\n    autospec runtime env status [--repo PATH] [--mode MODE]\n    autospec runtime env down [--repo PATH] [--mode MODE]"
    );
}
