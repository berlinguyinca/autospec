use super::*;

pub(in crate::commands::autonomous::executor_bridge) struct TrustedHookContext {
    pub(in crate::commands::autonomous::executor_bridge) environment: BTreeMap<String, OsString>,
    pub(in crate::commands::autonomous::executor_bridge) autospec: PathBuf,
}

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

pub(in crate::commands::autonomous::executor_bridge) fn trusted_linter_from(
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
