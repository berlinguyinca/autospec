//! Resolving the autospec executable that long-lived units spawn children from.
//!
//! `std::env::current_exe()` resolves `/proc/self/exe` on Linux, which keeps pointing
//! at the inode the process started from. An upgrade that replaces the binary — the
//! 24h self-update, or any `install` — unlinks that inode and creates a new one, so a
//! supervisor started before the upgrade resolves to a path Linux reports as
//! `"<path> (deleted)"` and every spawn fails with `ENOENT`.
//!
//! A supervisor is precisely the process that outlives an upgrade, so it must resolve
//! the file on disk instead of the inode it was born from.

use std::path::{Path, PathBuf};

/// The suffix Linux appends to `/proc/<pid>/exe` once the target has been unlinked.
const DELETED_SUFFIX: &str = " (deleted)";

fn deleted_suffix_stripped(path: &Path) -> Option<PathBuf> {
    let text = path.to_str()?;
    text.strip_suffix(DELETED_SUFFIX).map(PathBuf::from)
}

fn home_install_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".autospec")
            .join("bin")
            .join("autospec")
    })
}

/// Resolve the executable to spawn an autonomous child from.
///
/// Prefers the running executable while it still exists. When it has been replaced,
/// falls back to the live file at the same path, then `AUTOSPEC_BIN`, then the default
/// install location — so an upgrade mid-run costs nothing.
pub(super) fn autonomous_program(context: &str) -> Result<PathBuf, String> {
    let current =
        std::env::current_exe().map_err(|error| format!("cannot resolve {context}: {error}"))?;
    if current.is_file() {
        return Ok(current);
    }
    let mut tried = vec![current.display().to_string()];
    let replaced = deleted_suffix_stripped(&current);
    let configured = std::env::var_os("AUTOSPEC_BIN").map(PathBuf::from);
    for candidate in [replaced, configured, home_install_path()]
        .into_iter()
        .flatten()
    {
        if candidate.is_file() {
            return Ok(candidate);
        }
        tried.push(candidate.display().to_string());
    }
    Err(format!(
        "cannot resolve {context}; tried {}",
        tried.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::{autonomous_program, deleted_suffix_stripped};
    use std::path::{Path, PathBuf};

    #[test]
    fn a_live_executable_resolves_to_itself() {
        let resolved = autonomous_program("test program").expect("resolve");
        assert!(
            resolved.is_file(),
            "a running test binary still exists on disk"
        );
    }

    #[test]
    fn the_deleted_suffix_is_stripped_back_to_the_real_path() {
        let deleted = PathBuf::from("/home/someone/.autospec/bin/autospec (deleted)");

        assert_eq!(
            deleted_suffix_stripped(&deleted).as_deref(),
            Some(Path::new("/home/someone/.autospec/bin/autospec")),
            "an upgraded binary must resolve back to the live file at the same path"
        );
    }

    #[test]
    fn a_path_without_the_suffix_is_left_alone() {
        let live = PathBuf::from("/home/someone/.autospec/bin/autospec");

        assert_eq!(deleted_suffix_stripped(&live), None);
    }
}
