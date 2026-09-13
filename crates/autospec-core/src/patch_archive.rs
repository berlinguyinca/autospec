//! The archive paths a retired patch is moved to (issue #3994, #4559).
//!
//! These live in their own module rather than in `stored_output.rs`: that
//! file is over the size ratchet's threshold, and an oversized file may be
//! edited and shrunk but not grown. The `stored_output` module re-exports
//! both paths, so existing call sites keep their `stored_output::` names.

use std::path::{Path, PathBuf};

/// The archive path a superseded patch is retired to:
/// `out/issue-<N>/superseded/<stem>-<timestamp>.patch` (issue #3994).
///
/// `issue_dir` is the issue's output directory (`out/issue-<N>` in the
/// default layout) and `patch_name` the patch's file name (default
/// `changes.patch`), so the retired patch keeps its stem and gains the
/// retirement timestamp. Retirement is archival, never deletion: the patch is
/// *moved* under the issue's `superseded/` directory, so no patch content is
/// lost and the issue returns to the eligible pool.
pub fn superseded_archive_path(issue_dir: &Path, patch_name: &str, timestamp: u64) -> PathBuf {
    archive_path_in(issue_dir, "superseded", patch_name, timestamp)
}

/// The archive path for a patch held for its language (issue #4559): the
/// same shape as [`superseded_archive_path`] — stem plus retirement
/// timestamp, moved, never deleted — under the issue's `language-held/`
/// directory. The two directories are different queues: a superseded patch
/// is stale and returns to the eligible pool when re-dispatched; a
/// language-held patch will never convert, so its queue entry is freed with
/// the move and the directory is the terminal record.
pub fn language_held_archive_path(issue_dir: &Path, patch_name: &str, timestamp: u64) -> PathBuf {
    archive_path_in(issue_dir, "language-held", patch_name, timestamp)
}

/// The shared archive shape: `<issue_dir>/<archive_dir>/<stem>-<ts>.patch`,
/// where `archive_dir` names which queue the retirement belongs to.
fn archive_path_in(
    issue_dir: &Path,
    archive_dir: &str,
    patch_name: &str,
    timestamp: u64,
) -> PathBuf {
    let stem = patch_name
        .rsplit_once('.')
        .map(|(stem, _ext)| stem)
        .unwrap_or(patch_name);
    issue_dir
        .join(archive_dir)
        .join(format!("{stem}-{timestamp}.patch"))
}
