use std::fs;
use std::path::{Path, PathBuf};

use super::command::ToolCommand;
use super::results::{output_digest, CheckResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalCheck {
    BashSyntax,
    DeriveTrioConsistency,
}

impl ExternalCheck {
    pub fn run(&self, id: &str, required: bool, root: &Path) -> CheckResult {
        match self {
            Self::BashSyntax => run_bash_syntax(id, required, root),
            Self::DeriveTrioConsistency => run_derive_trio_consistency(id, required, root),
        }
    }
}

fn run_bash_syntax(id: &str, required: bool, root: &Path) -> CheckResult {
    let mut targets = Vec::new();
    for skill_dir in trio_directories(root) {
        for script in ["install.sh", "uninstall.sh"] {
            let path = skill_dir.join(script);
            if path.is_file() {
                targets.push(relative_path(root, &path));
            }
        }
    }
    for script in ["install.sh", "uninstall.sh"] {
        let path = root.join(script);
        if path.is_file() {
            targets.push(script.to_string());
        }
    }

    run_bash_syntax_targets(id, required, root, targets)
}

fn run_bash_syntax_targets(
    id: &str,
    required: bool,
    root: &Path,
    targets: Vec<String>,
) -> CheckResult {
    let mut results = Vec::new();
    for target in targets {
        let command = ToolCommand::new("bash", ["-n", target.as_str()])
            .expect("bash syntax validation has no command-string argument");
        let result = command.execute_in(id, required, root);
        let failed = result.is_failure();
        results.push(result);
        if failed {
            break;
        }
    }
    aggregate(id, required, results)
}

fn run_derive_trio_consistency(id: &str, required: bool, root: &Path) -> CheckResult {
    let derive = root.join("scripts/derive-trio.sh");
    if !derive.is_file() {
        return failure(
            id,
            required,
            "scripts/derive-trio.sh: missing (trio-derivation Phase 1 must be merged first)",
        );
    }

    let mut results = Vec::new();
    for skill_dir in trio_directories(root) {
        let relative = relative_path(root, &skill_dir);
        let command = ToolCommand::new("bash", ["scripts/derive-trio.sh", &relative, "--check"])
            .expect("bash -n style validation commands are direct argument vectors");
        let result = command.execute_in(id, required, root);
        let failed = result.is_failure();
        results.push(result);
        if failed {
            break;
        }
    }

    aggregate(id, required, results)
}

fn trio_directories(root: &Path) -> Vec<PathBuf> {
    let skills_root = root.join("skills");
    let Ok(entries) = fs::read_dir(skills_root) else {
        return Vec::new();
    };

    let mut directories = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path.join("SKILL.md").is_file()
                && path.join("opencode/agent.md").is_file()
                && path.join("codex/prompt.md").is_file()
        })
        .collect::<Vec<_>>();
    directories.sort();
    directories
}

fn aggregate(id: &str, required: bool, results: Vec<CheckResult>) -> CheckResult {
    if results.is_empty() {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }

    let failure = results.iter().find(|result| result.is_failure());
    let combined_digests = results
        .iter()
        .flat_map(|result| result.output_digest.bytes().chain(std::iter::once(b'\n')))
        .collect::<Vec<_>>();
    CheckResult {
        id: id.to_string(),
        required,
        exit_code: match failure {
            Some(result) => result.exit_code,
            None => Some(0),
        },
        elapsed_ms: results.iter().map(|result| result.elapsed_ms).sum(),
        spawn_count: results.iter().map(|result| result.spawn_count).sum(),
        stdout_bytes: results.iter().map(|result| result.stdout_bytes).sum(),
        stderr_bytes: results.iter().map(|result| result.stderr_bytes).sum(),
        output_digest: output_digest(&combined_digests, &[]),
    }
}

fn failure(id: &str, required: bool, message: &str) -> CheckResult {
    CheckResult::completed(
        id,
        required,
        1,
        0,
        0,
        0,
        message.len(),
        output_digest(&[], message.as_bytes()),
    )
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("discovered trio directory is inside the validation root")
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_preserves_a_missing_child_tool_failure() {
        let result = aggregate(
            "check",
            true,
            vec![CheckResult {
                id: "child".to_string(),
                required: true,
                exit_code: None,
                elapsed_ms: 0,
                spawn_count: 0,
                stdout_bytes: 0,
                stderr_bytes: 0,
                output_digest: "child".to_string(),
            }],
        );

        assert_eq!(result.exit_code, None);
        assert!(result.is_failure());
    }
}
