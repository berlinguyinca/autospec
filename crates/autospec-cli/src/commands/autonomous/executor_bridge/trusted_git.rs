// executor_bridge/trusted_git.rs — binding to the worktree's git so the executor cannot
// subvert it.
//
// One concern, extracted whole: the worktree binding, the hook bundle that neutralises
// repo-supplied hooks for the duration of a run, resolution of the trusted codex executable
// and linter, the signing attestation, and the rejection of external filters. Every item here
// exists to make git operations trustworthy while untrusted code is running beside them, and
// nothing else in the bridge is about that.
//
// The cfg-gated pairs (two TrustedHookBundle definitions, two TrustedHookContext) travel
// together because they are the same abstraction under different platform support.

use super::*;

mod signing_program;
use signing_program::resolve_signing_program;

#[derive(Debug)]
pub(super) struct TrustedWorktreeGit {
    pub(super) active_hooks: Vec<PathBuf>,
    pub(super) common_dir: PathBuf,
    pub(super) git_dir: PathBuf,
    pub(super) hooks_dir: PathBuf,
    pub(super) worktree: PathBuf,
}

impl TrustedWorktreeGit {
    pub(super) fn command(&self) -> Command {
        let mut command = Command::new("git");
        command
            .args(["-c", "core.fsmonitor=false"])
            .arg("--git-dir")
            .arg(&self.git_dir)
            .arg("--work-tree")
            .arg(&self.worktree);
        command
    }
}

pub(super) fn trusted_worktree_git(
    state: &PersistedInvocation,
) -> Result<TrustedWorktreeGit, String> {
    trusted_worktree_git_paths(&state.identity.repository_path, &state.identity.worktree)
}

pub(super) fn trusted_worktree_git_paths(
    repository_path: &Path,
    worktree_path: &Path,
) -> Result<TrustedWorktreeGit, String> {
    let worktree = fs::canonicalize(worktree_path)
        .map_err(|error| format!("canonicalize executor worktree: {error}"))?;
    let git_pointer = worktree.join(".git");
    let metadata = fs::symlink_metadata(&git_pointer)
        .map_err(|error| format!("inspect executor worktree Git metadata: {error}"))?;
    let test_primary = cfg!(test)
        && metadata.file_type().is_dir()
        && fs::canonicalize(repository_path)
            .map_err(|error| format!("canonicalize executor test repository: {error}"))?
            == worktree;
    if !test_primary && (!metadata.file_type().is_file() || metadata.file_type().is_symlink()) {
        return Err("executor worktree Git metadata is not a trusted gitdir file".to_string());
    }
    let git_dir = if test_primary {
        fs::canonicalize(&git_pointer)
            .map_err(|error| format!("canonicalize executor test gitdir: {error}"))?
    } else {
        let pointer = fs::read_to_string(&git_pointer)
            .map_err(|error| format!("read executor worktree gitdir: {error}"))?;
        let pointer = pointer
            .trim()
            .strip_prefix("gitdir: ")
            .ok_or_else(|| "executor worktree Git metadata has no gitdir pointer".to_string())?;
        if pointer.is_empty() || pointer.contains('\n') || pointer.contains('\r') {
            return Err("executor worktree Git metadata has an invalid gitdir pointer".to_string());
        }
        let candidate = PathBuf::from(pointer);
        let candidate = if candidate.is_absolute() {
            candidate
        } else {
            worktree.join(candidate)
        };
        fs::canonicalize(candidate)
            .map_err(|error| format!("canonicalize executor worktree gitdir: {error}"))?
    };

    let common = PathBuf::from(git_stdout(
        repository_path,
        &["rev-parse", "--git-common-dir"],
    )?);
    let common = if common.is_absolute() {
        common
    } else {
        repository_path.join(common)
    };
    let common = fs::canonicalize(common)
        .map_err(|error| format!("canonicalize executor common gitdir: {error}"))?;
    if !test_primary {
        let registrations = fs::canonicalize(common.join("worktrees"))
            .map_err(|error| format!("canonicalize executor worktree registrations: {error}"))?;
        if git_dir.parent() != Some(registrations.as_path()) {
            return Err(
                "executor worktree gitdir is not registered by the target repository".to_string(),
            );
        }
        let registered_pointer = fs::read_to_string(git_dir.join("gitdir"))
            .map_err(|error| format!("read registered executor worktree pointer: {error}"))?;
        let registered_pointer = PathBuf::from(registered_pointer.trim());
        let registered_pointer = fs::canonicalize(&registered_pointer).map_err(|error| {
            format!("canonicalize registered executor worktree pointer: {error}")
        })?;
        let expected_pointer = fs::canonicalize(&git_pointer)
            .map_err(|error| format!("canonicalize executor worktree pointer: {error}"))?;
        if registered_pointer != expected_pointer {
            return Err(
                "executor worktree gitdir is registered to a different worktree".to_string(),
            );
        }
    }

    let mut binding = TrustedWorktreeGit {
        active_hooks: Vec::new(),
        common_dir: common,
        git_dir,
        hooks_dir: PathBuf::new(),
        worktree,
    };
    let hooks = binding
        .command()
        .args(["rev-parse", "--git-path", "hooks"])
        .output()
        .map_err(|error| format!("resolve executor Git hook directory: {error}"))?;
    if !hooks.status.success() {
        return Err(format!(
            "resolve executor Git hook directory: {}",
            String::from_utf8_lossy(&hooks.stderr).trim()
        ));
    }
    let hooks = PathBuf::from(String::from_utf8_lossy(&hooks.stdout).trim());
    let hooks = if hooks.is_absolute() {
        hooks
    } else {
        binding.worktree.join(hooks)
    };
    let hooks = fs::canonicalize(hooks)
        .map_err(|error| format!("canonicalize executor Git hook directory: {error}"))?;
    if !test_primary && hooks.starts_with(&binding.worktree) {
        return Err(
            "executor Git hook directory is writable by the sandboxed implementer".to_string(),
        );
    }
    for entry in fs::read_dir(&hooks)
        .map_err(|error| format!("inventory executor Git hook directory: {error}"))?
    {
        let hook = entry.map_err(|error| format!("inventory executor Git hook entry: {error}"))?;
        let file_type = hook
            .file_type()
            .map_err(|error| format!("inspect executor Git hook entry: {error}"))?;
        if file_type.is_symlink() {
            return Err("executor Git hook must not be a symlink".to_string());
        }
        #[cfg(unix)]
        if !hook.file_name().to_string_lossy().ends_with(".sample")
            && hook
                .metadata()
                .map_err(|error| format!("inspect executor Git hook mode: {error}"))?
                .permissions()
                .mode()
                & 0o111
                != 0
        {
            let name = hook.file_name();
            if name.to_str() != Some("pre-commit") {
                return Err(format!(
                    "executor commit cannot contain unsupported active Git hook {}",
                    name.to_string_lossy()
                ));
            }
            let metadata = hook
                .metadata()
                .map_err(|error| format!("inspect executor Git hook: {error}"))?;
            if !metadata.file_type().is_file() || metadata.nlink() != 1 {
                return Err("executor Git hook must be a singly linked regular file".to_string());
            }
            binding.active_hooks.push(
                fs::canonicalize(hook.path())
                    .map_err(|error| format!("canonicalize executor Git hook: {error}"))?,
            );
        }
    }
    binding.hooks_dir = hooks;
    Ok(binding)
}

#[cfg(unix)]
pub(super) static HOOK_BUNDLE_NONCE: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
pub(super) struct TrustedHookBundle {
    pub(super) path: PathBuf,
    pub(super) temporary: bool,
}

#[cfg(unix)]
pub(super) struct TrustedHookContext {
    pub(super) environment: BTreeMap<String, OsString>,
    pub(super) autospec: PathBuf,
}

#[cfg(unix)]
impl TrustedHookContext {
    pub(super) fn current() -> Result<Self, String> {
        Ok(Self {
            environment: std::env::vars_os()
                .filter_map(|(key, value)| key.into_string().ok().map(|key| (key, value)))
                .collect(),
            autospec: std::env::current_exe()
                .map_err(|error| format!("resolve contained hook Autospec binary: {error}"))?,
        })
    }
}

#[cfg(unix)]
impl Drop for TrustedHookBundle {
    fn drop(&mut self) {
        if self.temporary {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(unix)]
pub(super) fn toml_path_entry(path: &Path, permission: &str) -> Result<String, String> {
    let path = path
        .to_str()
        .ok_or_else(|| "contained Git hook path must be UTF-8".to_string())?
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    Ok(format!("\"{path}\"=\"{permission}\""))
}

#[cfg(unix)]
pub(super) fn trusted_codex_executable_from(
    binding: &TrustedWorktreeGit,
    environment: &BTreeMap<String, OsString>,
) -> Result<PathBuf, String> {
    let codex = safe_executable(Path::new("codex"), environment)
        .map_err(|error| format!("resolve contained Git hook sandbox: {error}"))?;
    if codex.starts_with(&binding.worktree) {
        return Err(
            "contained Git hook sandbox executable is writable by the implementer".to_string(),
        );
    }
    Ok(codex)
}

#[cfg(unix)]
pub(super) fn trusted_linter_from(
    binding: &TrustedWorktreeGit,
    environment: &BTreeMap<String, OsString>,
) -> Result<(PathBuf, PathBuf), String> {
    let home = environment
        .get("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or_else(|| "HOME must be absolute for contained Git hooks".to_string())?;
    let scripts_dir = environment
        .get("AUTOSPEC_SCRIPTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".autospec/scripts"));
    let scripts_dir = fs::canonicalize(&scripts_dir)
        .map_err(|error| format!("canonicalize contained hook scripts: {error}"))?;
    let candidate = scripts_dir.join("lint-implementation.sh");
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| format!("inspect contained hook linter: {error}"))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("contained hook linter must be a regular non-symlink file".to_string());
    }
    let linter = fs::canonicalize(&candidate)
        .map_err(|error| format!("canonicalize contained hook linter: {error}"))?;
    if scripts_dir.starts_with(&binding.worktree)
        || linter.parent() != Some(scripts_dir.as_path())
        || linter.starts_with(&binding.worktree)
    {
        return Err("contained hook linter must not be writable by the implementer".to_string());
    }
    Ok((scripts_dir, linter))
}

#[cfg(unix)]
pub(super) fn contained_hook_profile(
    binding: &TrustedWorktreeGit,
    bundle: &Path,
    codex: &Path,
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
        toml_path_entry(codex, "read")?,
        toml_path_entry(autospec, "read")?,
    ];
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

#[cfg(unix)]
impl TrustedHookBundle {
    pub(super) fn create(binding: &TrustedWorktreeGit, issue_body: &str) -> Result<Self, String> {
        let context = TrustedHookContext::current()?;
        Self::create_with_context(binding, issue_body, &context)
    }

    pub(super) fn create_with_context(
        binding: &TrustedWorktreeGit,
        issue_body: &str,
        context: &TrustedHookContext,
    ) -> Result<Self, String> {
        if binding.active_hooks.is_empty() {
            return Ok(Self {
                path: binding.hooks_dir.clone(),
                temporary: false,
            });
        }
        let codex = trusted_codex_executable_from(binding, &context.environment)?;
        let home = context
            .environment
            .get("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
            .ok_or_else(|| "HOME must be absolute for contained Git hooks".to_string())?;
        let (scripts_dir, linter) = trusted_linter_from(binding, &context.environment)?;
        let autospec = fs::canonicalize(&context.autospec)
            .map_err(|error| format!("canonicalize contained hook Autospec binary: {error}"))?;
        if autospec.starts_with(&binding.worktree) {
            return Err("contained hook tools must not be writable by the implementer".to_string());
        }
        let nonce = HOOK_BUNDLE_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = binding.git_dir.join(format!(
            "autospec-contained-hooks-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).map_err(|error| format!("create contained hook bundle: {error}"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("protect contained hook bundle: {error}"))?;
        for child in ["codex-home", "tmp"] {
            fs::create_dir(path.join(child))
                .map_err(|error| format!("create contained hook {child}: {error}"))?;
            fs::set_permissions(path.join(child), fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("protect contained hook {child}: {error}"))?;
        }
        let issue_body_path = path.join("issue-body.md");
        write_private_create_once(
            &issue_body_path,
            issue_body.as_bytes(),
            "contained hook issue body",
        )?;
        let profile = contained_hook_profile(binding, &path, &codex, &linter, &autospec)?;
        for hook in &binding.active_hooks {
            let name = hook
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or_else(|| "contained Git hook name must be UTF-8".to_string())?;
            let status_path = path.join("tmp").join(format!("{name}.status"));
            let script = format!(
                "#!/bin/sh\nset -eu\nstatus_file={}\nrm -f \"$status_file\"\n/usr/bin/env -i HOME={} PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin LANG=C.UTF-8 TMPDIR={} CODEX_HOME={} AUTOSPEC_SCRIPTS_DIR={} AUTOSPEC_BIN={} AUTOSPEC_LINT_ISSUE_BODY_FILE={} GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 {} sandbox -C {} -P autospec-git-hook -c {} -c 'default_permissions=\"autospec-git-hook\"' -c 'permissions.autospec-git-hook.network.enabled=false' -c 'shell_environment_policy.inherit=\"all\"' -- /bin/sh -c 'hook=$1; status_file=$2; shift 2; \"$hook\" \"$@\"; status=$?; printf \"%s\\n\" \"$status\" > \"$status_file\"; exit \"$status\"' autospec-hook {} \"$status_file\" \"$@\"\n[ -f \"$status_file\" ] || exit 1\nIFS= read -r status < \"$status_file\" || exit 1\nrm -f \"$status_file\"\ncase \"$status\" in ''|*[!0-9]*) exit 1;; esac\n[ \"$status\" -le 255 ] || exit 1\nexit \"$status\"\n",
                posix_shell_quote(status_path.to_string_lossy().as_ref()),
                posix_shell_quote(home.to_string_lossy().as_ref()),
                posix_shell_quote(path.join("tmp").to_string_lossy().as_ref()),
                posix_shell_quote(path.join("codex-home").to_string_lossy().as_ref()),
                posix_shell_quote(scripts_dir.to_string_lossy().as_ref()),
                posix_shell_quote(autospec.to_string_lossy().as_ref()),
                posix_shell_quote(issue_body_path.to_string_lossy().as_ref()),
                posix_shell_quote(codex.to_string_lossy().as_ref()),
                posix_shell_quote(binding.worktree.to_string_lossy().as_ref()),
                posix_shell_quote(&profile),
                posix_shell_quote(hook.to_string_lossy().as_ref()),
            );
            let wrapper = path.join(name);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(&wrapper)
                .map_err(|error| format!("create contained Git hook wrapper: {error}"))?;
            file.write_all(script.as_bytes())
                .map_err(|error| format!("write contained Git hook wrapper: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("sync contained Git hook wrapper: {error}"))?;
        }
        Ok(Self {
            path,
            temporary: true,
        })
    }
}

#[cfg(not(unix))]
pub(super) struct TrustedHookBundle {
    pub(super) path: PathBuf,
}

#[cfg(not(unix))]
pub(super) struct TrustedHookContext;

#[cfg(not(unix))]
impl TrustedHookBundle {
    pub(super) fn create(binding: &TrustedWorktreeGit, _issue_body: &str) -> Result<Self, String> {
        if fs::read_dir(&binding.hooks_dir)
            .map_err(|error| format!("inventory unsupported Git hooks: {error}"))?
            .filter_map(Result::ok)
            .any(|entry| !entry.file_name().to_string_lossy().ends_with(".sample"))
        {
            return Err("contained Git hooks are unsupported on this platform".to_string());
        }
        Ok(Self {
            path: binding.hooks_dir.clone(),
        })
    }

    pub(super) fn create_with_context(
        binding: &TrustedWorktreeGit,
        issue_body: &str,
        _context: &TrustedHookContext,
    ) -> Result<Self, String> {
        Self::create(binding, issue_body)
    }
}

pub(super) fn sandboxed_executor_diff(state: &PersistedInvocation) -> Result<Vec<u8>, String> {
    let binding = trusted_worktree_git(state)?;
    sandboxed_executor_diff_with_binding(&binding)
}

pub(super) fn sandboxed_executor_diff_with_binding(
    binding: &TrustedWorktreeGit,
) -> Result<Vec<u8>, String> {
    let output = binding
        .command()
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ])
        .args(EXECUTOR_INTERNAL_PATHSPECS)
        .output()
        .map_err(|error| format!("inspect sandboxed executor diff: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "inspect sandboxed executor diff: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

pub(super) fn trusted_git_config(
    binding: &TrustedWorktreeGit,
    args: &[&str],
) -> Result<Option<String>, String> {
    let output = binding
        .command()
        .arg("config")
        .args(args)
        .output()
        .map_err(|error| format!("inspect executor Git configuration: {error}"))?;
    match output.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        )),
        Some(1) => Ok(None),
        _ => Err(format!(
            "inspect executor Git configuration: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

pub(super) fn reject_external_filters(binding: &TrustedWorktreeGit) -> Result<(), String> {
    let mut changed = BTreeSet::new();
    for args in [
        vec![
            "diff-index",
            "--cached",
            "--name-only",
            "-z",
            "HEAD",
            "--",
            ".",
        ],
        vec![
            "ls-files",
            "--modified",
            "--deleted",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
    ] {
        let output = binding
            .command()
            .args(args)
            .args(EXECUTOR_INTERNAL_PATHSPECS)
            .output()
            .map_err(|error| format!("inventory executor paths for clean filters: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "inventory executor paths for clean filters: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        for path in output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            changed.insert(
                std::str::from_utf8(path)
                    .map_err(|_| {
                        "executor cannot attest clean filters for a non-UTF-8 path".to_string()
                    })?
                    .to_string(),
            );
        }
    }
    for path in changed {
        let output = binding
            .command()
            .args(["check-attr", "-z", "filter", "--", &path])
            .output()
            .map_err(|error| format!("attest executor clean filter for {path}: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "attest executor clean filter for {path}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let fields = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty())
            .collect::<Vec<_>>();
        let value = fields
            .get(2)
            .ok_or_else(|| format!("attest executor clean filter for {path}: malformed output"))?;
        if !matches!(*value, b"unspecified" | b"unset") {
            return Err(format!(
                "executor path {path} selects external clean filter {}",
                String::from_utf8_lossy(value)
            ));
        }
    }
    Ok(())
}

pub(super) fn attest_executor_signing(binding: &TrustedWorktreeGit) -> Result<(), String> {
    let signing = trusted_git_config(binding, &["--type=bool", "--get", "commit.gpgSign"])?
        .is_some_and(|value| value == "true");
    if !signing {
        return Ok(());
    }
    let format =
        trusted_git_config(binding, &["--get", "gpg.format"])?.unwrap_or_else(|| "openpgp".into());
    let format_program = format!("gpg.{format}.program");
    let program = trusted_git_config(binding, &["--get", &format_program])?
        .or(trusted_git_config(binding, &["--get", "gpg.program"])?)
        .unwrap_or_else(|| match format.as_str() {
            "ssh" => "ssh-keygen".into(),
            "x509" => "gpgsm".into(),
            _ => "gpg".into(),
        });
    resolve_signing_program(binding, &program)?;
    if format == "ssh" {
        if trusted_git_config(binding, &["--get", "gpg.ssh.defaultKeyCommand"])?.is_some() {
            return Err(
                "executor SSH signing key command requires contained commit support".to_string(),
            );
        }
        if let Some(key) = trusted_git_config(binding, &["--get", "user.signingKey"])? {
            if !key.starts_with("key::") {
                let key = PathBuf::from(key);
                let key = if key.is_absolute() {
                    key
                } else {
                    binding.worktree.join(key)
                };
                if key.exists()
                    && fs::canonicalize(key)
                        .map_err(|error| format!("canonicalize executor signing key: {error}"))?
                        .starts_with(&binding.worktree)
                {
                    return Err(
                        "executor signing key is writable by the sandboxed implementer".to_string(),
                    );
                }
            }
        }
    }
    Ok(())
}

pub(super) fn stage_sandboxed_executor_diff(
    binding: &TrustedWorktreeGit,
    hooks_path: &Path,
) -> Result<(), String> {
    let mut paths = BTreeSet::new();
    for args in [
        vec![
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            "HEAD",
            "--",
            ".",
        ],
        vec![
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
    ] {
        let output = binding
            .command()
            .args(args)
            .args(EXECUTOR_INTERNAL_PATHSPECS)
            .output()
            .map_err(|error| format!("inventory sandboxed executor paths: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "inventory sandboxed executor paths: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        for path in output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            paths.insert(
                std::str::from_utf8(path)
                    .map_err(|_| {
                        "executor cannot stage a non-UTF-8 implementation path".to_string()
                    })?
                    .to_string(),
            );
        }
    }
    if paths.is_empty() {
        return Ok(());
    }
    let add = binding
        .command()
        .arg("-c")
        .arg(format!("core.hooksPath={}", hooks_path.to_string_lossy()))
        .arg("--literal-pathspecs")
        .args(["add", "--all", "--"])
        .args(paths)
        .output()
        .map_err(|error| format!("stage sandboxed executor diff: {error}"))?;
    if !add.status.success() {
        return Err(format!(
            "stage sandboxed executor diff: {}",
            String::from_utf8_lossy(&add.stderr).trim()
        ));
    }
    Ok(())
}
