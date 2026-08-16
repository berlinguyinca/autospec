use std::cell::Cell;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Instant;

use super::results::{output_digest, CheckResult};

thread_local! {
    static FAST_VALIDATION_MODE: Cell<bool> = const { Cell::new(false) };
}

/// Restores the validation execution mode when an external batch finishes.
pub(crate) struct FastValidationModeGuard {
    previous: bool,
}

pub(crate) fn enter_fast_validation_mode(fast: bool) -> FastValidationModeGuard {
    let previous = FAST_VALIDATION_MODE.with(|mode| mode.replace(fast));
    FastValidationModeGuard { previous }
}

impl Drop for FastValidationModeGuard {
    fn drop(&mut self) {
        FAST_VALIDATION_MODE.with(|mode| mode.set(self.previous));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCommand {
    program: PathBuf,
    args: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    removed_environment: Vec<OsString>,
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

        Ok(Self {
            program,
            args,
            environment: Vec::new(),
            removed_environment: Vec::new(),
        })
    }

    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        let key = key.into();
        self.removed_environment.retain(|removed| removed != &key);
        self.environment.push((key, value.into()));
        self
    }

    pub fn with_isolated_pytest(self, pythonpath: impl Into<OsString>) -> Self {
        self.with_env("PYTHONPATH", pythonpath)
            .with_env("PYTHONDONTWRITEBYTECODE", "1")
            .with_env("PYTEST_DISABLE_PLUGIN_AUTOLOAD", "1")
    }

    pub fn without_env(mut self, key: impl Into<OsString>) -> Self {
        let key = key.into();
        self.environment
            .retain(|(configured, _)| configured != &key);
        self.removed_environment.push(key);
        self
    }

    /// The environment overrides this command will apply, so a caller that must *prove*
    /// an override is present can assert on it without spawning the process.
    pub fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
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
        if should_skip_bats_in_fast_mode(&self.program) {
            return skipped_result(id, required);
        }
        let started = Instant::now();
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .envs(self.environment.iter().map(|(key, value)| (key, value)));
        for key in &self.removed_environment {
            command.env_remove(key);
        }
        let output = command
            .current_dir(self.working_directory_for(root))
            .output();

        captured_result(id, required, started, output)
    }

    pub(crate) fn execute_in_with_stdin_capturing(
        &self,
        id: impl Into<String>,
        required: bool,
        root: &Path,
        stdin_bytes: &[u8],
    ) -> CapturedCheckResult {
        let id = id.into();
        if should_skip_bats_in_fast_mode(&self.program) {
            return skipped_result(id, required);
        }
        let started = Instant::now();
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .envs(self.environment.iter().map(|(key, value)| (key, value)));
        for key in &self.removed_environment {
            command.env_remove(key);
        }
        command
            .current_dir(self.working_directory_for(root))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = match command.spawn() {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(stdin_bytes);
                }
                child.wait_with_output()
            }
            Err(error) => Err(error),
        };

        captured_result(id, required, started, output)
    }
}

fn should_skip_bats_in_fast_mode(program: &Path) -> bool {
    FAST_VALIDATION_MODE.with(Cell::get)
        && program.file_name().and_then(OsStr::to_str) == Some("bats")
}

fn skipped_result(id: String, required: bool) -> CapturedCheckResult {
    CapturedCheckResult {
        result: CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[])),
        stdout: Vec::new(),
    }
}

fn captured_result(
    id: String,
    required: bool,
    started: Instant,
    output: std::io::Result<Output>,
) -> CapturedCheckResult {
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

/// A Bats invocation with the refine lens pinned to the offline path.
///
/// `scripts/refine-prompt.sh` resolves its lens mode to `auto` by default, and `auto` is
/// LLM-*first*: it dispatches the model each round and only falls back to the template
/// lens when no dispatcher is reachable. With `claude` on PATH — which is the normal
/// state on a developer machine and on any host running autospec — a generic fixture
/// therefore spawned a real, billable model process, and `autospec validate` spent most
/// of an hour waiting on them (#2568).
///
/// Pinning `deterministic` here cannot disable the tests that genuinely exercise the LLM
/// path: `_resolve_lens_mode` gives `--lens-mode` precedence over this environment
/// variable, so a suite that passes the flag still gets the LLM lens.
///
/// Every Bats command validate runs goes through this function. That is the point — the
/// bug was one unset variable across ten construction sites, and a single site missed on
/// the next edit reintroduces it silently, as a slow run rather than a failure.
pub(crate) fn bats_command(suite: &str) -> ToolCommand {
    ToolCommand::new("bats", [suite])
        .expect("Bats validation has a static suite path")
        .with_env("AUTOSPEC_REFINE_LENS_MODE", "deterministic")
}

pub(crate) fn security_artifact_commands() -> Vec<ToolCommand> {
    vec![
        ToolCommand::new(
            "python3",
            ["scripts/validate-security-artifact.py", "--help"],
        )
        .expect("security artifact validator help uses direct arguments"),
        ToolCommand::new(
            "python3",
            [
                "scripts/validate-security-artifact.py",
                "tests/fixtures/security-artifact/valid.yml",
            ],
        )
        .expect("security artifact fixture validation uses direct arguments"),
        bats_command("tests/security-artifact-validator.bats"),
        bats_command("tests/unit/test_security_profile_skill_contract.bats"),
        bats_command("tests/unit/test_autospec_run_security_prerequisites.bats"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn every_validate_bats_command_pins_the_lens_offline() {
        let command = bats_command("tests/refine/test_refine_path_security.bats");
        assert_eq!(command.program(), std::path::Path::new("bats"));
        assert_eq!(
            command.args(),
            [OsStr::new("tests/refine/test_refine_path_security.bats")]
        );
        let pinned = command
            .environment()
            .iter()
            .find(|(key, _)| key == OsStr::new("AUTOSPEC_REFINE_LENS_MODE"))
            .map(|(_, value)| value.clone());
        assert_eq!(
            pinned.as_deref(),
            Some(OsStr::new("deterministic")),
            "validate would dispatch a real model: refine-prompt.sh defaults to auto, \
             which is LLM-first whenever a dispatcher is on PATH"
        );
    }
}
