//! Capturing what a stalled attempt produced, before anything is torn down.
//!
//! Two failures this exists to prevent. The first is destructive: the scratch
//! directory is removed at job end, so work that was never captured is gone for
//! good. The second is a false negative that made the first worse — a
//! counter that inspected only the working tree read a *committing* agent as
//! having produced nothing, which reported real work as `NO-OUTPUT` and then
//! deleted it. Commits are work; both signals are captured here.
//!
//! Capture runs before teardown and is best effort per section: one unreadable
//! section is recorded in `capture_errors`, it never aborts the capture of the
//! others.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

/// What an attempt left behind, counted from git rather than from the tree alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkProduced {
    /// Nothing at all: no commits, no working-tree changes.
    None,
    /// Uncommitted changes only.
    WorkingTreeOnly,
    /// Commits ahead of base, with a clean tree.
    Commits { count: u64 },
    /// Commits ahead of base plus further uncommitted changes.
    CommitsAndWorkingTree { count: u64 },
}

impl WorkProduced {
    pub fn produced(self) -> bool {
        self != WorkProduced::None
    }

    pub fn commits(self) -> u64 {
        match self {
            WorkProduced::None | WorkProduced::WorkingTreeOnly => 0,
            WorkProduced::Commits { count } | WorkProduced::CommitsAndWorkingTree { count } => {
                count
            }
        }
    }

    /// Short label used in the note attached to a released issue.
    pub fn label(self) -> &'static str {
        match self {
            WorkProduced::None => "none",
            WorkProduced::WorkingTreeOnly => "working-tree changes only",
            WorkProduced::Commits { .. } => "commits",
            WorkProduced::CommitsAndWorkingTree { .. } => "commits plus working-tree changes",
        }
    }
}

impl std::fmt::Display for WorkProduced {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkProduced::None => formatter.write_str("nothing"),
            WorkProduced::WorkingTreeOnly => formatter.write_str("working-tree changes only"),
            WorkProduced::Commits { count } => write!(formatter, "{count} commit(s)"),
            WorkProduced::CommitsAndWorkingTree { count } => {
                write!(formatter, "{count} commit(s) plus working-tree changes")
            }
        }
    }
}

/// Build the produced-work classification from the two git signals.
pub fn classify_work(commits_ahead: u64, working_tree_dirty: bool) -> WorkProduced {
    match (commits_ahead, working_tree_dirty) {
        (0, false) => WorkProduced::None,
        (0, true) => WorkProduced::WorkingTreeOnly,
        (count, false) => WorkProduced::Commits { count },
        (count, true) => WorkProduced::CommitsAndWorkingTree { count },
    }
}

/// One commit captured ahead of base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRecord {
    pub id: String,
    pub subject: String,
}

/// Everything a stalled attempt left behind, captured before teardown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartialWork {
    pub commits: Vec<CommitRecord>,
    /// The commits themselves as a patch, not a file count.
    pub commit_patch: String,
    /// Uncommitted changes, tracked and untracked, as a patch.
    pub working_tree_patch: String,
    /// Tail of the agent session transcript.
    pub transcript_excerpt: String,
    /// Full transcript size at capture time.
    pub transcript_bytes: u64,
    /// Sections that could not be captured, and why.
    pub capture_errors: Vec<String>,
}

impl PartialWork {
    pub fn work_produced(&self) -> WorkProduced {
        classify_work(
            self.commits.len() as u64,
            !self.working_tree_patch.trim().is_empty(),
        )
    }

    pub fn produced_anything(&self) -> bool {
        self.work_produced().produced()
    }

    /// The artifacts the next attempt (or a human) reads, keyed by attempt number.
    pub fn artifacts(&self, attempt: u32) -> Vec<Artifact> {
        let mut artifacts = Vec::new();
        if !self.commit_patch.trim().is_empty() {
            artifacts.push(Artifact {
                name: format!("attempt-{attempt}-commits.patch"),
                body: self.commit_patch.clone(),
            });
        }
        if !self.working_tree_patch.trim().is_empty() {
            artifacts.push(Artifact {
                name: format!("attempt-{attempt}-working-tree.patch"),
                body: self.working_tree_patch.clone(),
            });
        }
        if !self.transcript_excerpt.trim().is_empty() {
            artifacts.push(Artifact {
                name: format!("attempt-{attempt}-transcript-tail.txt"),
                body: self.transcript_excerpt.clone(),
            });
        }
        artifacts
    }
}

/// Something attached to an issue, or stored for the next attempt to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub name: String,
    pub body: String,
}

/// Where a stalled attempt's evidence comes from.
///
/// Behind an interface so capture is not welded to git, to a local filesystem,
/// or to any particular issue tracker: a runner over a container worktree
/// implements the same four reads.
pub trait WorktreeEvidence {
    /// Commits ahead of `base`, oldest first.
    fn commits_ahead_of_base(&self, base: &str) -> Result<Vec<CommitRecord>, String>;

    /// Those commits as a patch.
    fn commit_patch(&self, base: &str) -> Result<String, String>;

    /// Uncommitted changes, including paths not yet tracked.
    fn working_tree_patch(&self) -> Result<String, String>;

    /// `(tail, total_bytes)` of the agent session transcript.
    fn transcript_tail(&self, max_bytes: usize) -> Result<(String, u64), String>;
}

/// Capture every section that can be read, recording the ones that cannot.
///
/// Never fails: an attempt whose evidence is half-readable still gets that half
/// preserved, which is the whole point of capturing before teardown.
pub fn capture_partial_work(
    evidence: &dyn WorktreeEvidence,
    base: &str,
    transcript_tail_bytes: usize,
) -> PartialWork {
    let mut work = PartialWork::default();

    match evidence.commits_ahead_of_base(base) {
        Ok(commits) => work.commits = commits,
        Err(error) => work.capture_errors.push(format!("commits: {error}")),
    }
    match evidence.commit_patch(base) {
        Ok(patch) => work.commit_patch = patch,
        Err(error) => work.capture_errors.push(format!("commit patch: {error}")),
    }
    match evidence.working_tree_patch() {
        Ok(patch) => work.working_tree_patch = patch,
        Err(error) => work.capture_errors.push(format!("working tree: {error}")),
    }
    match evidence.transcript_tail(transcript_tail_bytes) {
        Ok((excerpt, total)) => {
            work.transcript_excerpt = excerpt;
            work.transcript_bytes = total;
        }
        Err(error) => work.capture_errors.push(format!("transcript: {error}")),
    }

    work
}

/// [`WorktreeEvidence`] backed by a git worktree and a transcript file.
///
/// Every call is a plain `git` invocation with no GNU-only tooling, so capture
/// behaves identically on Linux and macOS. Set `AUTOSPEC_GIT_PROGRAM` to point
/// at a non-default git.
#[derive(Debug, Clone)]
pub struct GitWorktreeEvidence {
    worktree: PathBuf,
    transcript: Option<PathBuf>,
}

impl GitWorktreeEvidence {
    pub fn new(worktree: impl Into<PathBuf>, transcript: Option<PathBuf>) -> Self {
        Self {
            worktree: worktree.into(),
            transcript,
        }
    }

    fn git(&self, arguments: &[&str]) -> Result<String, String> {
        let program = std::env::var_os("AUTOSPEC_GIT_PROGRAM").unwrap_or_else(|| "git".into());
        let output = Command::new(&program)
            .current_dir(&self.worktree)
            .args(arguments)
            .output()
            .map_err(|error| format!("could not run git {}: {error}", arguments.join(" ")))?;
        if !output.status.success() {
            return Err(format!(
                "git {} failed: {}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl WorktreeEvidence for GitWorktreeEvidence {
    fn commits_ahead_of_base(&self, base: &str) -> Result<Vec<CommitRecord>, String> {
        let listing = self.git(&["log", "--format=%H%x09%s", &format!("{base}..HEAD")])?;
        Ok(listing
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(2, '\t');
                let id = fields.next()?.trim();
                if id.is_empty() {
                    return None;
                }
                Some(CommitRecord {
                    id: id.to_string(),
                    subject: fields.next().unwrap_or_default().trim().to_string(),
                })
            })
            .rev()
            .collect())
    }

    fn commit_patch(&self, base: &str) -> Result<String, String> {
        self.git(&["format-patch", "--stdout", &format!("{base}..HEAD")])
    }

    fn working_tree_patch(&self) -> Result<String, String> {
        let mut patch = self.git(&["diff", "--no-ext-diff", "HEAD"])?;
        let untracked = self.git(&["ls-files", "--others", "--exclude-standard"])?;
        let untracked: Vec<&str> = untracked
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        if !untracked.is_empty() {
            if !patch.is_empty() && !patch.ends_with('\n') {
                patch.push('\n');
            }
            patch.push_str("Untracked files:\n");
            for path in untracked {
                patch.push_str(path);
                patch.push('\n');
            }
        }
        Ok(patch)
    }

    fn transcript_tail(&self, max_bytes: usize) -> Result<(String, u64), String> {
        let Some(path) = self.transcript.as_ref() else {
            return Ok((String::new(), 0));
        };
        read_tail(path, max_bytes).map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// Read the last `max_bytes` of a file, snapped to a UTF-8 boundary.
pub fn read_tail(path: &Path, max_bytes: usize) -> io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let total = file.metadata()?.len();
    if max_bytes == 0 || total == 0 {
        return Ok((String::new(), total));
    }
    let start = total.saturating_sub(max_bytes as u64);
    file.seek(SeekFrom::Start(start))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    // Snap forward to the first char boundary so the tail is valid UTF-8.
    let offset = match std::str::from_utf8(&buffer) {
        Ok(_) => 0,
        Err(error) => error.valid_up_to(),
    };
    Ok((
        String::from_utf8_lossy(&buffer[offset..]).into_owned(),
        total,
    ))
}

/// Filesystem store for captured work, so the next attempt starts from the
/// previous one's evidence instead of from zero.
///
/// Layout is `root/issue-<number>/attempt-<n>/<artifact name>`. Directories and
/// files are private on Unix, matching the runtime-state convention elsewhere.
#[derive(Debug, Clone)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn attempt_dir(&self, issue: u64, attempt: u32) -> PathBuf {
        self.root
            .join(format!("issue-{issue}"))
            .join(format!("attempt-{attempt}"))
    }

    /// Write one captured artifact, returning the path it landed at.
    pub fn write(&self, issue: u64, attempt: u32, artifact: &Artifact) -> io::Result<PathBuf> {
        let dir = self.attempt_dir(issue, attempt);
        std::fs::create_dir_all(&dir)?;
        private(&dir);
        let path = dir.join(&artifact.name);
        std::fs::write(&path, &artifact.body)?;
        private(&path);
        Ok(path)
    }

    /// Write every captured artifact for one attempt, reporting where each landed.
    pub fn write_all(
        &self,
        issue: u64,
        attempt: u32,
        work: &PartialWork,
    ) -> io::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for artifact in work.artifacts(attempt) {
            paths.push(self.write(issue, attempt, &artifact)?);
        }
        Ok(paths)
    }

    /// Highest attempt number stored for an issue, if any.
    pub fn latest_attempt(&self, issue: u64) -> io::Result<Option<u32>> {
        let dir = self.root.join(format!("issue-{issue}"));
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut latest: Option<u32> = None;
        for entry in entries {
            let name = entry?.file_name().to_string_lossy().into_owned();
            let Some(number) = name.strip_prefix("attempt-") else {
                continue;
            };
            let Ok(number) = number.parse::<u32>() else {
                continue;
            };
            latest = Some(latest.map_or(number, |current: u32| current.max(number)));
        }
        Ok(latest)
    }

    /// Everything the most recent attempt left for this issue.
    pub fn read_latest(&self, issue: u64) -> io::Result<Vec<Artifact>> {
        let Some(attempt) = self.latest_attempt(issue)? else {
            return Ok(Vec::new());
        };
        let dir = self.attempt_dir(issue, attempt);
        let mut artifacts = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if !path.is_file() {
                continue;
            }
            artifacts.push(Artifact {
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                body: std::fs::read_to_string(&path)?,
            });
        }
        artifacts.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(artifacts)
    }
}

/// Restrict permissions on Unix; other platforms keep their defaults.
fn private(_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if _path.is_dir() { 0o700 } else { 0o600 };
        let _ = std::fs::set_permissions(_path, std::fs::Permissions::from_mode(mode));
    }
}
