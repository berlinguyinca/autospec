//! Whether an implementation run produced anything, counted from commits as well as the
//! working tree.
//!
//! A gate that asks only "is the working tree dirty?" reports an agent that *committed*
//! its work as having produced nothing. That false negative fired at least five times in
//! the session behind #3563, and four of those runs lived in node-local scratch that is
//! wiped at job end — so the verdict did not merely mislabel the work, it authorised
//! throwing it away.
//!
//! [`ProducedWork::detect`] therefore counts both signals, and captures the *patch* when
//! work exists only as commits. A count is enough to correct the verdict; only the patch
//! survives the workspace. The patch is returned rather than written, because this module
//! cannot know which directories outlive the run — see [`ProducedWork::write_patch`] for
//! the durable-sink half, whose path the caller chooses.
//!
//! Capture must not depend on which of the equivalent states the producer happened to
//! leave behind (#4582). Staged, committed, and working-tree-dirty are three encodings of
//! "the agent changed these files"; a capture that yields a patch for one of them and
//! silently `None` for the others is a data-loss path disguised as a conditional — ten
//! completed runs were discarded that way when the only captured state was the committed
//! one. `detect` therefore captures the uncommitted states as patch bytes too, and
//! `write_patch` writes whichever parts exist, so a non-empty detection always yields a
//! non-empty patch. [`ProducedWork::assert_captured`] is the deletion guard a caller runs
//! before tearing a scratch tree down: work without a durable patch refuses the deletion.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What an implementation run left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedWork {
    /// Paths with uncommitted changes, including untracked files.
    pub uncommitted_paths: Vec<String>,
    /// Commits on `HEAD` that the base ref does not have.
    pub commits_ahead: usize,
    /// The commits ahead of base as a patch, present whenever `commits_ahead > 0`.
    ///
    /// Held as bytes because `git format-patch` reproduces file content verbatim, and a
    /// diff that only round-trips when it happens to be UTF-8 is not a backup.
    pub committed_patch: Option<Vec<u8>>,
    /// Staged, dirty, and untracked changes as a patch, present whenever
    /// `uncommitted_paths` is non-empty. This is the state the committed-only capture
    /// used to drop (#4582): the common one, since an agent that stages and stops has
    /// produced work a commits-only diff cannot see.
    pub uncommitted_patch: Option<Vec<u8>>,
}

impl ProducedWork {
    /// Inspects `repository` for work produced relative to `base_ref`.
    ///
    /// Fails loudly when `git` cannot be run: an unreadable repository must not be
    /// reported as an empty one, which would recreate the bug in a new disguise.
    pub fn detect(repository: &Path, base_ref: &str) -> Result<Self, String> {
        Self::detect_excluding(repository, base_ref, &[])
    }

    /// As [`ProducedWork::detect`], ignoring paths matched by `exclusions`.
    ///
    /// `exclusions` are git pathspecs (`:(exclude)…`). A caller whose own verdict already
    /// discounts some paths — a harness's bookkeeping files, say — must pass the same set
    /// here, or this will report the harness's own scratch as the agent's work and turn a
    /// routine empty run into a hard failure.
    pub fn detect_excluding(
        repository: &Path,
        base_ref: &str,
        exclusions: &[&str],
    ) -> Result<Self, String> {
        let uncommitted_paths = uncommitted_paths(repository, exclusions)?;
        let commits_ahead = commits_ahead(repository, base_ref)?;
        let committed_patch = if commits_ahead > 0 {
            Some(committed_patch(repository, base_ref)?)
        } else {
            None
        };
        // The uncommitted states are captured with the same exclusions: a file the
        // caller's verdict discounts is not agent work, and patching it in would make the
        // artifact disagree with the count that authorised the capture.
        let uncommitted_patch = if uncommitted_paths.is_empty() {
            None
        } else {
            Some(uncommitted_patch(repository, exclusions)?)
        };
        Ok(Self {
            uncommitted_paths,
            commits_ahead,
            committed_patch,
            uncommitted_patch,
        })
    }

    /// Whether the run genuinely produced nothing.
    ///
    /// True only when *both* signals are empty. A clean working tree on its own says the
    /// agent committed, not that it idled.
    pub fn is_empty(&self) -> bool {
        self.uncommitted_paths.is_empty() && self.commits_ahead == 0
    }

    /// Whether the only evidence of work is committed, so a tree-only check would miss it.
    pub fn is_committed_only(&self) -> bool {
        self.uncommitted_paths.is_empty() && self.commits_ahead > 0
    }

    /// Writes the captured patch under `directory`, returning where it landed.
    ///
    /// `directory` must outlive the run's workspace — the point of the patch is to
    /// survive a scratch directory being wiped, and writing it inside that directory
    /// would preserve nothing. Returns `Ok(None)` when there was no patch to write.
    ///
    /// The patch is the union of whichever states hold work (#4582): the commits ahead
    /// of base (with their messages) first, then the staged, dirty, and untracked
    /// changes. A non-empty detection therefore always yields a non-empty file — a
    /// summary without the artifact it summarises is the outcome this exists to prevent.
    pub fn write_patch(&self, directory: &Path, name: &str) -> Result<Option<PathBuf>, String> {
        let mut patch: Vec<u8> = Vec::new();
        if let Some(part) = &self.committed_patch {
            patch.extend_from_slice(part);
        }
        if let Some(part) = &self.uncommitted_patch {
            if !patch.is_empty() {
                patch.push(b'\n');
            }
            patch.extend_from_slice(part);
        }
        if patch.is_empty() {
            return Ok(None);
        }
        fs::create_dir_all(directory).map_err(|error| {
            format!(
                "create captured work directory {}: {error}",
                directory.display()
            )
        })?;
        let path = directory.join(format!("{name}.patch"));
        fs::write(&path, &patch)
            .map_err(|error| format!("write captured work patch {}: {error}", path.display()))?;
        Ok(Some(path))
    }

    /// The deletion guard (#4582): a scratch tree may go away only when everything it
    /// holds is either absent or durably captured.
    ///
    /// `written` is the path [`ProducedWork::write_patch`] returned. An empty detection
    /// passes unconditionally; a non-empty one passes only when a non-empty patch exists
    /// at that path. Anything else is an error naming the work, so the caller cannot
    /// proceed to the teardown that would delete the only copy.
    pub fn assert_captured(&self, written: Option<&Path>) -> Result<(), String> {
        if self.is_empty() {
            return Ok(());
        }
        match written {
            Some(path) if fs::metadata(path).is_ok_and(|meta| meta.len() > 0) => Ok(()),
            _ => Err(format!(
                "refusing to delete the scratch tree: {} and no non-empty captured patch exists",
                self.to_json()
            )),
        }
    }

    pub fn to_json(&self) -> String {
        let paths = self
            .uncommitted_paths
            .iter()
            .map(|path| format!("\"{}\"", escape_json(path)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"produced_work\":{},\"uncommitted_paths\":[{paths}],\"commits_ahead\":{},\"patch_bytes\":{},\"uncommitted_patch_bytes\":{}}}",
            !self.is_empty(),
            self.commits_ahead,
            self.committed_patch.as_ref().map_or(0, Vec::len),
            self.uncommitted_patch.as_ref().map_or(0, Vec::len),
        )
    }
}

fn uncommitted_paths(repository: &Path, exclusions: &[&str]) -> Result<Vec<String>, String> {
    // NUL-delimited, so a path containing a newline or a quote cannot split one record
    // into two and inflate the count.
    let mut args = vec![
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--",
        ".",
    ];
    args.extend_from_slice(exclusions);
    let stdout = git_stdout(repository, &args)?;

    let mut paths = Vec::new();
    let mut records = stdout.split(|byte| *byte == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        if record.len() < 4 {
            continue;
        }
        // A rename or copy spends two records: the new path, then the original. Reading
        // the second as a change of its own would count one edit as two.
        if matches!(record[0], b'R' | b'C') {
            records.next();
        }
        paths.push(String::from_utf8_lossy(&record[3..]).into_owned());
    }
    Ok(paths)
}

fn commits_ahead(repository: &Path, base_ref: &str) -> Result<usize, String> {
    let range = format!("{base_ref}..HEAD");
    let stdout = git_stdout(repository, &["rev-list", "--count", &range])?;
    String::from_utf8_lossy(&stdout)
        .trim()
        .parse()
        .map_err(|error| format!("parse commits ahead of {base_ref}: {error}"))
}

fn committed_patch(repository: &Path, base_ref: &str) -> Result<Vec<u8>, String> {
    let range = format!("{base_ref}..HEAD");
    // `format-patch` rather than `diff`, so the captured work carries its commit
    // messages: a successor picking this up needs the reasoning, not only the hunks.
    git_stdout(
        repository,
        &["format-patch", "--stdout", "--no-signature", &range],
    )
}

/// The staged, dirty, and untracked states as patch bytes, under the same exclusions the
/// detection was run with. Read-only: the index is never touched, so a detection cannot
/// leave the tree in a state the agent did not choose.
fn uncommitted_patch(repository: &Path, exclusions: &[&str]) -> Result<Vec<u8>, String> {
    // `git diff HEAD` covers the two tracked states at once — index vs HEAD (staged) and
    // worktree vs index (dirty) collapse into worktree vs HEAD. Untracked files are in
    // neither, so they are diffed against /dev/null one by one below.
    let mut args = vec!["diff", "HEAD", "--", "."];
    args.extend_from_slice(exclusions);
    let mut patch = git_stdout(repository, &args)?;

    let mut list_args = vec![
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
        "--",
        ".",
    ];
    list_args.extend_from_slice(exclusions);
    let listing = git_stdout(repository, &list_args)?;
    for entry in listing.split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }
        let file = String::from_utf8_lossy(entry).into_owned();
        patch.push(b'\n');
        patch.extend_from_slice(&untracked_diff(repository, &file)?);
    }
    Ok(patch)
}

/// The `--no-index` diff of one untracked file against the empty state. Unlike plain
/// `diff`, git's exit code here is the verdict: 1 means "differences found", which is
/// the success case for a file that exists.
fn untracked_diff(repository: &Path, file: &str) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(["diff", "--no-index", "--", "/dev/null", file])
        .current_dir(repository)
        .output()
        .map_err(|error| format!("git is required to detect produced work but did not run ({error}); nothing was measured"))?;
    match output.status.code() {
        Some(0) => Ok(Vec::new()),
        Some(1) => Ok(output.stdout),
        other => Err(format!(
            "git diff --no-index {file} exited {other:?} in {}: {}",
            repository.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

fn git_stdout(repository: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository)
        .output()
        .map_err(|error| {
            format!(
                "git is required to detect produced work but did not run ({error}); \
                 nothing was measured"
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed in {}: {}",
            args.join(" "),
            repository.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}
