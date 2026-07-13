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

pub(crate) struct CapturedCheckResult {
    pub result: CheckResult,
    pub stdout: Vec<u8>,
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
        if is_launcher_executable(&program) {
            return Err("validation tool commands must not use launcher executables".to_string());
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

    pub fn working_directory_for(&self, root: &Path) -> PathBuf {
        root.to_path_buf()
    }

    pub fn execute(&self, id: impl Into<String>, required: bool) -> CheckResult {
        self.execute_in(id, required, &self.working_directory())
    }

    pub fn execute_in(&self, id: impl Into<String>, required: bool, root: &Path) -> CheckResult {
        self.execute_in_capturing(id, required, root).result
    }

    pub(crate) fn execute_in_capturing(
        &self,
        id: impl Into<String>,
        required: bool,
        root: &Path,
    ) -> CapturedCheckResult {
        let id = id.into();
        let started = Instant::now();
        let output = Command::new(&self.program)
            .args(&self.args)
            .current_dir(self.working_directory_for(root))
            .output();
        let elapsed_ms = started.elapsed().as_millis();

        match output {
            Ok(output) => {
                let stdout = output.stdout;
                let stderr = output.stderr;
                CapturedCheckResult {
                    result: CheckResult {
                        id,
                        required,
                        exit_code: output.status.code(),
                        elapsed_ms,
                        spawn_count: 1,
                        stdout_bytes: stdout.len(),
                        stderr_bytes: stderr.len(),
                        output_digest: output_digest(&stdout, &stderr),
                    },
                    stdout,
                }
            }
            Err(_) => CapturedCheckResult {
                result: CheckResult {
                    id,
                    required,
                    exit_code: None,
                    elapsed_ms,
                    spawn_count: 0,
                    stdout_bytes: 0,
                    stderr_bytes: 0,
                    output_digest: output_digest(&[], &[]),
                },
                stdout: Vec::new(),
            },
        }
    }
}

fn invokes_shell_command_string(program: &Path, args: &[OsString]) -> bool {
    is_shell_interpreter(program)
        && args
            .iter()
            .filter_map(|argument| argument.to_str())
            .any(is_shell_command_argument)
}

fn is_shell_interpreter(program: &Path) -> bool {
    matches!(
        program.file_name().and_then(OsStr::to_str),
        Some("ash" | "bash" | "csh" | "dash" | "fish" | "ksh" | "sh" | "tcsh" | "zsh")
    )
}

fn is_launcher_executable(program: &Path) -> bool {
    program.file_name().and_then(OsStr::to_str) == Some("env")
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
