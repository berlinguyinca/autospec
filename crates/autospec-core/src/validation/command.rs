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
    let Some(program_name) = program.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    if !matches!(
        program_name,
        "ash" | "bash" | "csh" | "dash" | "fish" | "ksh" | "sh" | "tcsh" | "zsh"
    ) {
        return false;
    }

    args.iter()
        .filter_map(|argument| argument.to_str())
        .any(is_shell_command_argument)
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
