//! Git implementation of the integration-phase VCS interface (issue #3565).
//!
//! Platform independence: this module shells out to the `git` binary only —
//! no GNU-only flags, no POSIX-shell assumptions, identical on Linux and
//! macOS. No GitHub (or any other product) is assumed; any git remote or
//! bare clone works.
//!
//! **`git rebase --skip` is never invoked.** The only rebase flags this
//! module passes are `-c merge.conflictStyle=diff3` (to capture the
//! ancestor side of every conflict) and, when committing a resolution,
//! `-c core.editor=true rebase --continue`. A branch that cannot be
//! integrated is restored with `git rebase --abort`, which keeps every
//! commit — it discards nothing.

use super::vcs::{ConflictHunk, ConflictedFile, RebaseOutcome, ResolvedFile, Vcs, VcsError};
use std::path::PathBuf;
use std::process::Command;

/// A [`Vcs`] backed by a local git repository.
pub struct GitVcs {
    repo: PathBuf,
    trunk: String,
    /// The batch, in dependency (landing) order.
    branches: Vec<String>,
}

impl GitVcs {
    pub fn new(repo: PathBuf, trunk: String, branches: Vec<String>) -> Self {
        Self {
            repo,
            trunk,
            branches,
        }
    }

    /// The exact `git` argv used to rebase one branch onto the trunk.
    ///
    /// Exposed so tests can prove the automated path never passes `--skip`
    /// (or `--edit`/`--abort`): skipping a commit is the one operation the
    /// integration phase must not be able to perform.
    pub fn rebase_invocation(trunk: &str) -> Vec<String> {
        vec![
            "-c".to_string(),
            "merge.conflictStyle=diff3".to_string(),
            "rebase".to_string(),
            trunk.to_string(),
        ]
    }

    fn run_git(&self, args: &[String]) -> Result<String, VcsError> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo)
            .output()
            .map_err(|e| VcsError(format!("failed to run git: {e}")))?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if output.status.success() {
            Ok(stdout)
        } else {
            let combined = format!("{stdout}{stderr}").trim().to_string();
            Err(VcsError(format!(
                "git {} failed: {combined}",
                args.join(" ")
            )))
        }
    }

    /// Parse conflict markers (diff3 style) out of a conflicted working
    /// file. Returns the hunks with `start` measured in lines of the
    /// trunk-side (stage 2) file.
    pub fn parse_conflict_markers(content: &str) -> Result<Vec<ConflictHunk>, VcsError> {
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        enum State {
            Base,
            Ours,
            Ancestor,
            Theirs,
        }
        let mut hunks = Vec::new();
        let mut state = State::Base;
        let mut base_lines = 0usize;
        let mut current: Option<ConflictHunk> = None;
        for line in content.lines() {
            let line = line.to_string();
            let marker = if line.starts_with("<<<<<<<") {
                Some("open")
            } else if line.starts_with("|||||||") {
                Some("ancestor")
            } else if line.starts_with(">>>>>>>") {
                Some("close")
            } else if line.starts_with("=======") && state != State::Base {
                Some("divider")
            } else {
                None
            };
            match (state, marker) {
                (State::Base, None) => base_lines += 1,
                (State::Base, Some("open")) => {
                    current = Some(ConflictHunk {
                        start: base_lines,
                        ancestor: Vec::new(),
                        ours: Vec::new(),
                        theirs: Vec::new(),
                    });
                    state = State::Ours;
                }
                (State::Ours, Some("ancestor")) => state = State::Ancestor,
                (State::Ours, Some("divider")) => state = State::Theirs,
                (State::Ours, None) => current.as_mut().unwrap().ours.push(line),
                (State::Ancestor, Some("divider")) => state = State::Theirs,
                (State::Ancestor, None) => current.as_mut().unwrap().ancestor.push(line),
                (State::Theirs, Some("close")) => {
                    hunks.push(current.take().expect("close without open"));
                    state = State::Base;
                }
                (State::Theirs, None) => current.as_mut().unwrap().theirs.push(line),
                other => {
                    return Err(VcsError(format!(
                        "malformed conflict markers (unexpected state/marker: {other:?})"
                    )))
                }
            }
        }
        if state != State::Base {
            return Err(VcsError(
                "unterminated conflict markers in file".to_string(),
            ));
        }
        Ok(hunks)
    }

    fn conflicted_files(&self) -> Result<Vec<ConflictedFile>, VcsError> {
        let names = self.run_git(&[
            "-c".to_string(),
            "core.quotePath=false".to_string(),
            "diff".to_string(),
            "--name-only".to_string(),
            "--diff-filter=U".to_string(),
        ])?;
        let mut files = Vec::new();
        for path in names.lines().filter(|l| !l.trim().is_empty()) {
            let working = std::fs::read_to_string(self.repo.join(path))
                .map_err(|e| VcsError(format!("cannot read conflicted file {path}: {e}")))?;
            let base = self
                .run_git(&["show".to_string(), format!(":2:{path}")])
                .map_err(|e| VcsError(format!("cannot read trunk-side stage for {path}: {e}")))?;
            let hunks = Self::parse_conflict_markers(&working)
                .map_err(|e| VcsError(format!("{path}: {e}")))?;
            if hunks.is_empty() {
                return Err(VcsError(format!(
                    "{path} is conflicted but carries no text conflict markers (binary file?); \
                     cannot classify, a human must resolve it"
                )));
            }
            files.push(ConflictedFile {
                path: path.to_string(),
                base,
                hunks,
            });
        }
        Ok(files)
    }

    /// Re-run the rebase for `branch`, expected to hit the same conflict
    /// that was captured and aborted. Returns `true` when a conflict is in
    /// progress (the index now holds unmerged paths).
    fn restart_rebase(&self, branch: &str) -> Result<bool, VcsError> {
        self.run_git(&["checkout".to_string(), branch.to_string()])
            .map_err(|e| VcsError(format!("checkout {branch}: {e}")))?;
        match self.run_git(&Self::rebase_invocation(&self.trunk)) {
            Ok(_) => Ok(false),
            Err(_) => Ok(true),
        }
    }
}

impl Vcs for GitVcs {
    fn batch(&self) -> Result<Vec<String>, VcsError> {
        Ok(self.branches.clone())
    }

    fn rebase(&mut self, branch: &str) -> Result<RebaseOutcome, VcsError> {
        if !self.restart_rebase(branch)? {
            return Ok(RebaseOutcome::Clean);
        }
        let files = self.conflicted_files()?;
        // Restore the pre-rebase state. `--abort` keeps every commit; it is
        // not `--skip` and discards nothing. The resolution is committed
        // later via apply_resolution, which re-enters the rebase.
        self.run_git(&["rebase".to_string(), "--abort".to_string()])
            .map_err(|e| VcsError(format!("rebase --abort after conflict capture: {e}")))?;
        Ok(RebaseOutcome::Conflict { files })
    }

    fn apply_resolution(&mut self, branch: &str, files: &[ResolvedFile]) -> Result<(), VcsError> {
        if !self.restart_rebase(branch)? {
            // The branch now rebases cleanly; nothing to resolve.
            return Ok(());
        }
        for file in files {
            std::fs::write(self.repo.join(&file.path), &file.content)
                .map_err(|e| VcsError(format!("cannot write resolution for {}: {e}", file.path)))?;
            self.run_git(&["add".to_string(), file.path.clone()])
                .map_err(|e| VcsError(format!("git add {}: {e}", file.path)))?;
        }
        let continue_args = vec![
            "-c".to_string(),
            "core.editor=true".to_string(),
            "rebase".to_string(),
            "--continue".to_string(),
        ];
        match self.run_git(&continue_args) {
            Ok(_) => Ok(()),
            Err(e) => {
                self.run_git(&["rebase".to_string(), "--abort".to_string()])
                    .map_err(|abort| {
                        VcsError(format!(
                            "rebase --continue failed: {e}; --abort failed: {abort}"
                        ))
                    })?;
                Err(e)
            }
        }
    }

    fn land(&mut self, branch: &str) -> Result<(), VcsError> {
        self.run_git(&["checkout".to_string(), self.trunk.clone()])
            .map_err(|e| VcsError(format!("checkout trunk: {e}")))?;
        self.run_git(&[
            "merge".to_string(),
            "--ff-only".to_string(),
            branch.to_string(),
        ])
        .map_err(|e| VcsError(format!("ff-only merge of {branch} into trunk: {e}")))?;
        Ok(())
    }

    fn settle(&mut self) -> Result<(), VcsError> {
        // Defensive: every phase path already aborts a conflicted rebase,
        // but never hand the repository back with a rebase in progress.
        // (`--abort` keeps every commit; it is not `--skip`.) The call is a
        // harmless no-op error when no rebase is running.
        let _ = self.run_git(&["rebase".to_string(), "--abort".to_string()]);
        self.run_git(&["checkout".to_string(), self.trunk.clone()])
            .map(|_| ())
            .map_err(|e| VcsError(format!("settle: checkout trunk: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebase_invocation_never_contains_skip_edit_or_abort() {
        let args = GitVcs::rebase_invocation("main");
        assert_eq!(
            args,
            vec!["-c", "merge.conflictStyle=diff3", "rebase", "main",]
        );
        for banned in ["--skip", "--edit", "--abort", "--apply"] {
            assert!(!args.iter().any(|a| a == banned), "passes {banned}");
        }
    }

    #[test]
    fn parses_diff3_markers_with_base_offsets() {
        let content = "line0\n<<<<<<< HEAD\nours1\nours2\n||||||| merged common ancestors\nanc\n=======\ntheirs1\ntheirs2\ntheirs3\n>>>>>>> feature/x\nline9\n";
        let hunks = GitVcs::parse_conflict_markers(content).expect("parse");
        assert_eq!(hunks.len(), 1);
        let hunk = &hunks[0];
        assert_eq!(hunk.start, 1);
        assert_eq!(hunk.ours, vec!["ours1", "ours2"]);
        assert_eq!(hunk.ancestor, vec!["anc"]);
        assert_eq!(hunk.theirs, vec!["theirs1", "theirs2", "theirs3"]);
    }

    #[test]
    fn rejects_unterminated_markers() {
        let content =
            "line0\n<<<<<<< HEAD\nours\n||||||| merged common ancestors\nanc\n=======\ntheirs\n";
        assert!(GitVcs::parse_conflict_markers(content).is_err());
    }
}
