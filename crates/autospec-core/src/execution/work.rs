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
        Ok(Self {
            uncommitted_paths,
            commits_ahead,
            committed_patch,
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
    pub fn write_patch(&self, directory: &Path, name: &str) -> Result<Option<PathBuf>, String> {
        let Some(patch) = self.committed_patch.as_deref() else {
            return Ok(None);
        };
        fs::create_dir_all(directory).map_err(|error| {
            format!(
                "create captured work directory {}: {error}",
                directory.display()
            )
        })?;
        let path = directory.join(format!("{name}.patch"));
        fs::write(&path, patch)
            .map_err(|error| format!("write captured work patch {}: {error}", path.display()))?;
        Ok(Some(path))
    }

    pub fn to_json(&self) -> String {
        let paths = self
            .uncommitted_paths
            .iter()
            .map(|path| format!("\"{}\"", escape_json(path)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"produced_work\":{},\"uncommitted_paths\":[{paths}],\"commits_ahead\":{},\"patch_bytes\":{}}}",
            !self.is_empty(),
            self.commits_ahead,
            self.committed_patch.as_ref().map_or(0, Vec::len),
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
