use std::fs;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::path::Component;

pub(super) fn resolve_executor_worktree_root_with(
    temporary: PathBuf,
    canonicalize: impl FnOnce(&Path) -> std::io::Result<PathBuf>,
) -> Result<PathBuf, String> {
    let canonical = canonicalize(&temporary).map_err(|error| {
        format!(
            "canonicalize executor temporary directory {}: {error}",
            temporary.display()
        )
    })?;
    if !canonical.is_absolute() {
        return Err(format!(
            "canonical executor temporary directory is not absolute: {}",
            canonical.display()
        ));
    }
    #[cfg(windows)]
    if !matches!(canonical.components().next(), Some(Component::Prefix(_))) {
        return Err(format!(
            "canonical executor temporary directory is not drive-qualified: {}",
            canonical.display()
        ));
    }
    Ok(canonical.join("autospec-executor"))
}

pub(crate) fn executor_worktree_root() -> Result<PathBuf, String> {
    static ROOT: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        #[cfg(windows)]
        let temporary = std::env::temp_dir();
        #[cfg(not(windows))]
        let temporary = PathBuf::from("/tmp");
        resolve_executor_worktree_root_with(temporary, |path| fs::canonicalize(path))
    })
    .clone()
}
