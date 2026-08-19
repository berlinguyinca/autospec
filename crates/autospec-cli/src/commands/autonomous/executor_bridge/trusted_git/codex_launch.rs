use super::*;

const MAX_SHEBANG_BYTES: u64 = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::commands::autonomous::executor_bridge) enum TrustedCodexLaunch {
    Native {
        program: TrustedExecutable,
    },
    NodeScript {
        node: TrustedExecutable,
        script: TrustedExecutable,
    },
}

impl TrustedCodexLaunch {
    pub(in crate::commands::autonomous::executor_bridge) fn program(&self) -> &Path {
        match self {
            Self::Native { program } => program.path(),
            Self::NodeScript { node, .. } => node.path(),
        }
    }

    pub(in crate::commands::autonomous::executor_bridge) fn prefix_args(&self) -> Vec<PathBuf> {
        match self {
            Self::Native { .. } => Vec::new(),
            Self::NodeScript { script, .. } => vec![script.path().to_path_buf()],
        }
    }

    fn executables(&self) -> Vec<&TrustedExecutable> {
        match self {
            Self::Native { program } => vec![program],
            Self::NodeScript { node, script } => vec![node, script],
        }
    }

    pub(in crate::commands::autonomous::executor_bridge) fn revalidate(
        &self,
    ) -> Result<(), String> {
        for executable in self.executables() {
            executable.revalidate()?;
        }
        Ok(())
    }

    pub(in crate::commands::autonomous::executor_bridge) fn shell_words(&self) -> String {
        self.executables()
            .into_iter()
            .map(|executable| posix_shell_quote(executable.path().to_string_lossy().as_ref()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn first_line(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| format!("open contained Git hook Codex launcher: {error}"))?
        .take(MAX_SHEBANG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read contained Git hook Codex launcher: {error}"))?;
    if bytes.starts_with(b"#!")
        && bytes.len() as u64 > MAX_SHEBANG_BYTES
        && !bytes[..MAX_SHEBANG_BYTES as usize].contains(&b'\n')
    {
        return Err("contained Git hook Codex launcher shebang exceeds 4096 bytes".to_string());
    }
    Ok(bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default()
        .to_vec())
}

pub(in crate::commands::autonomous::executor_bridge) fn trusted_codex_launch_from(
    binding: &TrustedWorktreeGit,
    environment: &BTreeMap<String, OsString>,
) -> Result<TrustedCodexLaunch, String> {
    let script = TrustedExecutable::resolve(
        Path::new("codex"),
        environment,
        &binding.worktree,
        "contained Git hook Codex launcher",
    )?;
    let line = first_line(script.path())?;
    if !line.starts_with(b"#!") {
        return Ok(TrustedCodexLaunch::Native { program: script });
    }
    let interpreter = if line == b"#!/usr/bin/env node" {
        PathBuf::from("node")
    } else {
        let value = std::str::from_utf8(&line[2..])
            .map_err(|_| "contained Git hook Codex launcher shebang must be UTF-8".to_string())?;
        let path = PathBuf::from(value);
        if !path.is_absolute()
            || value.bytes().any(|byte| byte.is_ascii_whitespace())
            || path.file_name() != Some(OsStr::new("node"))
        {
            return Err(
                "contained Git hook Codex launcher shebang must be exactly /usr/bin/env node or an absolute node path"
                    .to_string(),
            );
        }
        path
    };
    let node = TrustedExecutable::resolve(
        &interpreter,
        environment,
        &binding.worktree,
        "contained Git hook Node interpreter",
    )?;
    Ok(TrustedCodexLaunch::NodeScript { node, script })
}

fn toml_path_entry(path: &Path, permission: &str) -> Result<String, String> {
    let path = path
        .to_str()
        .ok_or_else(|| "contained Git hook path must be UTF-8".to_string())?
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    Ok(format!("\"{path}\"=\"{permission}\""))
}

pub(in crate::commands::autonomous::executor_bridge) fn contained_hook_profile_paths(
    launch: &TrustedCodexLaunch,
) -> Result<Vec<String>, String> {
    launch
        .executables()
        .into_iter()
        .map(|executable| toml_path_entry(executable.path(), "read"))
        .collect()
}

pub(in crate::commands::autonomous::executor_bridge) fn contained_hook_profile(
    binding: &TrustedWorktreeGit,
    bundle: &Path,
    launch: &TrustedCodexLaunch,
    linter: &Path,
    autospec: &Path,
) -> Result<String, String> {
    let mut entries = vec![
        "\":minimal\"=\"read\"".to_string(),
        "\":workspace_roots\"={}".to_string(),
        toml_path_entry(&binding.worktree, "write")?,
        toml_path_entry(&binding.worktree.join(".git"), "read")?,
        toml_path_entry(
            &binding.worktree.join(".autospec/executor-closeout.md"),
            "read",
        )?,
        toml_path_entry(&binding.worktree.join(".autospec/local-git"), "read")?,
        toml_path_entry(
            &binding.worktree.join(".autospec/original-git-pointer"),
            "read",
        )?,
        toml_path_entry(&binding.common_dir, "read")?,
        toml_path_entry(&binding.git_dir, "read")?,
        toml_path_entry(&binding.hooks_dir, "read")?,
        toml_path_entry(linter, "read")?,
        toml_path_entry(bundle, "read")?,
        toml_path_entry(&bundle.join("tmp"), "write")?,
        toml_path_entry(autospec, "read")?,
    ];
    entries.extend(contained_hook_profile_paths(launch)?);
    for path in [
        "~/.aws",
        "~/.azure",
        "~/.cargo/credentials",
        "~/.cargo/credentials.toml",
        "~/.codex/archived_sessions",
        "~/.codex/auth.json",
        "~/.codex/config.toml",
        "~/.codex/history.jsonl",
        "~/.codex/sessions",
        "~/.codex/shell_snapshots",
        "~/.config/containers",
        "~/.config/gcloud",
        "~/.config/gh",
        "~/.config/pip",
        "~/.docker",
        "~/.git-credentials",
        "~/.gnupg",
        "~/.gradle",
        "~/.kube",
        "~/.m2",
        "~/.netrc",
        "~/.npmrc",
        "~/.pypirc",
        "~/.ssh",
        "~/.terraform.d",
        "~/.vault-token",
    ] {
        entries.push(format!("\"{path}\"=\"deny\""));
    }
    Ok(format!(
        "permissions.autospec-git-hook.filesystem={{{}}}",
        entries.join(",")
    ))
}
