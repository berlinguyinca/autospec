//! VCS interface for the integration phase (issue #3565).
//!
//! The phase talks to version control exclusively through [`Vcs`] so that no
//! particular product (GitHub, Gitea, plain git) is assumed.
//!
//! **No skip, ever.** The trait deliberately exposes no `skip` operation:
//! `git rebase --skip` silently discards a commit and reports success, which
//! is exactly the failure mode the integration phase exists to prevent. A
//! branch that cannot be integrated is left untouched (its rebase is aborted)
//! and reported, never dropped.

/// One conflict region inside a conflicted file.
///
/// `start` is the 0-based line index in [`ConflictedFile::base`] where this
/// region begins; the `ours` lines occupy exactly the range
/// `[start, start + ours.len())`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictHunk {
    pub start: usize,
    /// Lines of the region in the common ancestor.
    pub ancestor: Vec<String>,
    /// Lines of the region on the trunk (ours) side.
    pub ours: Vec<String>,
    /// Lines of the region on the branch (theirs) side.
    pub theirs: Vec<String>,
}

/// A file with conflicts captured during a rebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictedFile {
    pub path: String,
    /// Full content of the file on the trunk (ours) side at rebase time,
    /// without conflict markers.
    pub base: String,
    /// Conflict regions, in ascending `start` order.
    pub hunks: Vec<ConflictHunk>,
}

/// A resolved version of a conflicted file, ready to be committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFile {
    pub path: String,
    pub content: String,
}

/// Outcome of rebasing one branch onto the current trunk.
///
/// After a `Conflict` the backend must have restored the repository to a
/// clean state (the rebase is aborted after the conflict data is captured);
/// [`Vcs::apply_resolution`] re-enters the rebase to commit a resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebaseOutcome {
    /// The branch replayed cleanly onto the trunk.
    Clean,
    /// The branch conflicts with the trunk; every conflict region is
    /// reported so the phase can classify and gate it.
    Conflict { files: Vec<ConflictedFile> },
}

/// A VCS failure. The message is surfaced verbatim in the batch report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcsError(pub String);

impl std::fmt::Display for VcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VcsError {}

/// Version-control operations the integration phase relies on.
pub trait Vcs {
    /// The batch, in dependency (landing) order.
    fn batch(&self) -> Result<Vec<String>, VcsError>;

    /// Rebase `branch` onto the current trunk.
    ///
    /// Never skips: the call returns either [`RebaseOutcome::Clean`] or the
    /// full conflict data. There is no operation in this trait that discards
    /// a commit.
    fn rebase(&mut self, branch: &str) -> Result<RebaseOutcome, VcsError>;

    /// Commit a resolution for a conflicted rebase of `branch`, then
    /// continue the rebase to completion.
    fn apply_resolution(&mut self, branch: &str, files: &[ResolvedFile]) -> Result<(), VcsError>;

    /// Land the rebased branch into the trunk (fast-forward only).
    fn land(&mut self, branch: &str) -> Result<(), VcsError>;

    /// Leave the repository in a settled state: no rebase in progress and
    /// the trunk checked out. Called once when the batch is fully
    /// processed, so a run that halted on a branch still hands the
    /// repository back on the trunk, never on the halted branch.
    fn settle(&mut self) -> Result<(), VcsError>;
}
