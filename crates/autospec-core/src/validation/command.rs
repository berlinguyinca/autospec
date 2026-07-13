use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use super::results::{output_digest, CheckResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCommand {
    program: PathBuf,
    args: Vec<OsString>,
}

impl ToolCommand {
    pub fn new<P, I, A>(program: P, args: I) -> Result<Self, String>
    where
        P: Into<PathBuf>,
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        let program = program.into();
        let args = args.into_iter().map(Into::into).collect::<Vec<_>>();

        if program.as_os_str().is_empty() {
            return Err("validation tool command program must not be empty".to_string());
        }
        if invokes_shell_command_string(&program, &args) {
            return Err(
                "validation tool commands must not execute shell command strings".to_string(),
            );
        }

        Ok(Self { program, args })
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    pub fn working_directory(&self) -> PathBuf {
        repository_root()
    }

    pub fn execute(&self, id: impl Into<String>, required: bool) -> CheckResult {
        let id = id.into();
        let started = Instant::now();
        let output = Command::new(&self.program)
            .args(&self.args)
            .current_dir(self.working_directory())
            .output();
        let elapsed_ms = started.elapsed().as_millis();

        match output {
            Ok(output) => CheckResult {
                id,
                required,
                exit_code: output.status.code(),
                elapsed_ms,
                spawn_count: 1,
                stdout_bytes: output.stdout.len(),
                stderr_bytes: output.stderr.len(),
                output_digest: output_digest(&output.stdout, &output.stderr),
            },
            Err(_) => CheckResult {
                id,
                required,
                exit_code: None,
                elapsed_ms,
                spawn_count: 0,
                stdout_bytes: 0,
                stderr_bytes: 0,
                output_digest: output_digest(&[], &[]),
            },
        }
    }
}

fn invokes_shell_command_string(program: &Path, args: &[OsString]) -> bool {
    (is_shell_interpreter(program)
        && args
            .iter()
            .filter_map(|argument| argument.to_str())
            .any(is_shell_command_argument))
        || (is_environment_launcher(program) && environment_launcher_invokes_shell(args))
}

fn is_shell_interpreter(program: &Path) -> bool {
    matches!(
        program.file_name().and_then(OsStr::to_str),
        Some("ash" | "bash" | "csh" | "dash" | "fish" | "ksh" | "sh" | "tcsh" | "zsh")
    )
}

fn is_environment_launcher(program: &Path) -> bool {
    program.file_name().and_then(OsStr::to_str) == Some("env")
}

fn environment_launcher_invokes_shell(args: &[OsString]) -> bool {
    match environment_launcher_command(args) {
        EnvironmentLauncherCommand::Command(program, command_args) => {
            invokes_shell_command_string(Path::new(program), command_args)
        }
        EnvironmentLauncherCommand::SplitString => true,
        EnvironmentLauncherCommand::None => false,
    }
}

enum EnvironmentLauncherCommand<'a> {
    Command(&'a OsStr, &'a [OsString]),
    SplitString,
    None,
}

fn environment_launcher_command(args: &[OsString]) -> EnvironmentLauncherCommand<'_> {
    let mut index = 0;

    while let Some(argument) = args.get(index) {
        let Some(argument_text) = argument.to_str() else {
            return EnvironmentLauncherCommand::None;
        };

        if argument_text == "--" {
            return args
                .get(index + 1)
                .map(|program| {
                    EnvironmentLauncherCommand::Command(program.as_os_str(), &args[index + 2..])
                })
                .unwrap_or(EnvironmentLauncherCommand::None);
        }
        if is_environment_split_string_option(argument_text) {
            return EnvironmentLauncherCommand::SplitString;
        }
        if argument_text.starts_with('-') {
            index += usize::from(environment_option_consumes_next_argument(argument_text));
            index += 1;
            continue;
        }
        if argument_text.contains('=') {
            index += 1;
            continue;
        }
        return EnvironmentLauncherCommand::Command(argument.as_os_str(), &args[index + 1..]);
    }

    EnvironmentLauncherCommand::None
}

fn is_environment_split_string_option(argument: &str) -> bool {
    argument == "-S"
        || argument.starts_with("-S")
        || argument == "--split-string"
        || argument.starts_with("--split-string=")
        || argument
            .strip_prefix('-')
            .is_some_and(|options| !options.starts_with('-') && options.contains('S'))
}

fn environment_option_consumes_next_argument(argument: &str) -> bool {
    matches!(
        argument,
        "-a" | "-C" | "-P" | "-u" | "--argv0" | "--chdir" | "--unset"
    ) || argument.strip_prefix('-').is_some_and(|options| {
        !options.starts_with('-') && matches!(options.chars().last(), Some('a' | 'C' | 'P' | 'u'))
    })
}

fn is_shell_command_argument(argument: &str) -> bool {
    argument == "-c"
        || argument == "--command"
        || argument.starts_with("--command=")
        || (argument.starts_with('-') && !argument.starts_with("--") && argument[1..].contains('c'))
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("autospec-core manifest is two directories below the repository root")
        .to_path_buf()
}
