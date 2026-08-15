use super::*;

pub(super) fn resolve_signing_program(
    binding: &TrustedWorktreeGit,
    program: &str,
) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(program);
    let resolved = if candidate.components().count() > 1 {
        let candidate = if candidate.is_absolute() {
            candidate
        } else {
            binding.worktree.join(candidate)
        };
        fs::canonicalize(candidate)
            .map_err(|error| format!("canonicalize executor signing program: {error}"))?
    } else {
        let path = std::env::var_os("PATH")
            .ok_or_else(|| "executor signing program resolution requires PATH".to_string())?;
        std::env::split_paths(&path)
            .map(|directory| directory.join(program))
            .find(|candidate| candidate.is_file())
            .map(fs::canonicalize)
            .transpose()
            .map_err(|error| format!("canonicalize executor signing program: {error}"))?
            .ok_or_else(|| format!("executor signing program {program} is unavailable"))?
    };
    if resolved.starts_with(&binding.worktree) {
        return Err(
            "executor signing program is writable by the sandboxed implementer".to_string(),
        );
    }
    Ok(resolved)
}
