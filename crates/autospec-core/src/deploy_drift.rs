//! Deployment drift (#4416): deploy from a committed ref, or make
//! divergence loud.
//!
//! The incident: every model over ~30 GB had never registered on the fleet,
//! because the deployed worker sampled `/props` once, ten seconds after
//! launch, and a large model answers 503 until its weights are resident
//! (105 GiB took over 1 h 44 m). A day was spent diagnosing it. The fix was
//! written, deployed and verified. Committing that work revealed that `main`
//! had **already** solved it by a better route: deriving the context window
//! from the launch arguments and registering immediately, never touching
//! `/props`. The repository had moved on; the running fleet had not. The
//! deployed file had diverged from its tracked version by 542 lines across
//! six files, with 38 `.bak-*` copies left behind.
//!
//! The bug was not open in the codebase. It was open only in the
//! **deployment**, invisible from both sides: reading the repository would
//! not have found it, and reading the running file would not have revealed
//! that a better fix already existed.
//!
//! Three invariants, mechanical here:
//!
//! 1. **Deploy from a committed ref, or make divergence loud**
//!    ([`startup_check`], [`startup_action`], [`StartupPolicy`]). A start-up
//!    check comparing the running script's hash against the tracked version
//!    at the deployed ref costs nothing and converts an invisible class of
//!    bug into a log line. A worker whose script differs refuses to start,
//!    or logs at ERROR; a script that has no tracked version at the deployed
//!    ref is fail-closed, never assumed current.
//! 2. **A fleet scan reports any file that differs from HEAD**
//!    ([`FleetScan`]). The CI or periodic job walks the fleet directories,
//!    compares each running file against HEAD, and prints one line per
//!    diverged file; [`FleetScan::exit_code`] is the job's verdict, so the
//!    report wires straight into CI.
//! 3. **`.bak-*` files are swept and the practice replaced by branches**
//!    ([`is_backup_name`], [`FleetScan`]). A backup copy left in a fleet
//!    directory is reported with the replacement practice: commit the
//!    in-flight edit to a branch, do not leave a `.bak-*` copy.
//!
//! Everything here is pure: the caller reads the running file and the
//! tracked versions (at the deployed ref and at HEAD) and passes both in.
//! No I/O, no clock, no subprocess.

use crate::autonomous::waterfall::sha256_hex;

/// The first 12 hex digits of a sha256: enough to name a file's content in a
/// log line without printing the whole digest.
fn short(hash: &str) -> &str {
    &hash[..12]
}

/// What a start-up check found for one deployed script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupVerdict {
    /// The running script is byte-identical to the tracked version at the
    /// deployed ref: start.
    InSync {
        /// sha256 of the (identical) running and tracked content.
        sha256: String,
    },
    /// The running script differs from the tracked version at the deployed
    /// ref: refuse to start, or log at ERROR.
    Drifted {
        /// sha256 of the running script.
        running: String,
        /// sha256 of the tracked version at the deployed ref.
        tracked: String,
    },
    /// No tracked version exists at the deployed ref: divergence cannot be
    /// cleared in either direction. This is not [`StartupVerdict::InSync`];
    /// it is the fail-closed answer.
    Untracked {
        /// sha256 of the running script.
        running: String,
    },
}

impl StartupVerdict {
    /// The log line that makes the divergence loud, or `None` when the
    /// script is in sync and there is nothing to say.
    ///
    /// The line always names the path, the deployed ref, and the content
    /// hashes, so a reader of the log can locate both copies without
    /// re-reading the files.
    pub fn message(&self, path: &str, deployed_ref: &str) -> Option<String> {
        match self {
            Self::InSync { .. } => None,
            Self::Drifted { running, tracked } => Some(format!(
                "ERROR: {path} differs from the tracked version at {deployed_ref} \
                 (running {}, tracked {}); deploy from a committed ref or make \
                 divergence loud (#4416)",
                short(running),
                short(tracked)
            )),
            Self::Untracked { running } => Some(format!(
                "ERROR: {path} has no tracked version at {deployed_ref} \
                 (running {}); fail-closed: deploy from a committed ref (#4416)",
                short(running)
            )),
        }
    }
}

/// Compare the running script against the tracked version at the deployed
/// ref.
///
/// `tracked = None` is the file absent from the deployed ref at all, which
/// is [`StartupVerdict::Untracked`] and fails closed: a script nobody has
/// committed is never assumed current.
pub fn startup_check(running: &[u8], tracked: Option<&[u8]>) -> StartupVerdict {
    match tracked {
        None => StartupVerdict::Untracked {
            running: sha256_hex(running),
        },
        Some(t) if t == running => StartupVerdict::InSync {
            sha256: sha256_hex(running),
        },
        Some(t) => StartupVerdict::Drifted {
            running: sha256_hex(running),
            tracked: sha256_hex(t),
        },
    }
}

/// What a deployment does when the start-up check does not pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPolicy {
    /// Refuse to start: the strongest form, for a service where running a
    /// drifted script is worse than not running.
    Refuse,
    /// Continue, but log at ERROR: the divergence is loud, never silent.
    LogError,
}

/// The concrete outcome of one start-up check under one policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupAction {
    /// The check passed: start normally.
    Start,
    /// Refuse to start, with the message the service must log before exiting.
    Refuse(String),
    /// Continue, with the message the service must log at ERROR.
    LogError(String),
}

/// Decide what one service does at start-up given the check's verdict and
/// its policy.
///
/// The decision table is the whole policy:
///
/// | verdict   | policy             | action                                   |
/// |-----------|--------------------|------------------------------------------|
/// | in sync   | (any)              | [`StartupAction::Start`]                 |
/// | drifted   | [`StartupPolicy::Refuse`]   | [`StartupAction::Refuse`] (named hashes) |
/// | drifted   | [`StartupPolicy::LogError`] | [`StartupAction::LogError`] (at ERROR)  |
/// | untracked | (either)           | the same as drifted: fail-closed         |
pub fn startup_action(
    policy: StartupPolicy,
    path: &str,
    deployed_ref: &str,
    verdict: &StartupVerdict,
) -> StartupAction {
    match verdict.message(path, deployed_ref) {
        None => StartupAction::Start,
        Some(message) => match policy {
            StartupPolicy::Refuse => StartupAction::Refuse(format!("{message}; refusing to start")),
            StartupPolicy::LogError => StartupAction::LogError(message),
        },
    }
}

/// Whether `name` carries the `.bak-*` backup marker: a base name containing
/// `.bak-` (e.g. `worker.sh.bak-20260814`, or a file literally named
/// `.bak-worker.sh`). The directory part is ignored; only the base name
/// counts.
pub fn is_backup_name(name: &str) -> bool {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.contains(".bak-")
}

/// One file under a fleet directory, as the scan found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetEntry {
    /// Path of the file relative to the fleet directory.
    pub path: String,
    /// sha256 of the running (deployed) copy.
    pub running: String,
    /// sha256 of the HEAD version, or `None` when the file is absent from
    /// HEAD (present only in the deployment).
    pub head: Option<String>,
    /// Whether the base name carries the `.bak-` backup marker.
    pub backup: bool,
}

impl FleetEntry {
    /// `true` when the running copy is not what HEAD tracks: content differs,
    /// or the file is absent from HEAD altogether.
    pub fn diverges(&self) -> bool {
        self.head.as_deref() != Some(self.running.as_str())
    }
}

/// The scan of one fleet directory: every running file compared against
/// HEAD, plus the `.bak-*` sweep. This is what the CI or periodic job runs:
/// [`FleetScan::report`] is what it prints and [`FleetScan::exit_code`] is
/// what it returns, so a diverged fleet turns a green job red.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FleetScan {
    entries: Vec<FleetEntry>,
}

impl FleetScan {
    /// An empty scan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one running file against its HEAD version.
    ///
    /// `head = None` is the file absent from HEAD: it still gets a line,
    /// because a deployment-only file is divergence too.
    pub fn add(&mut self, path: &str, running: &[u8], head: Option<&[u8]>) {
        self.entries.push(FleetEntry {
            path: path.to_string(),
            running: sha256_hex(running),
            head: head.map(sha256_hex),
            backup: is_backup_name(path),
        });
    }

    /// The recorded files.
    pub fn entries(&self) -> &[FleetEntry] {
        &self.entries
    }

    /// `true` only when every file matches HEAD and no `.bak-*` copy is
    /// present.
    pub fn is_clean(&self) -> bool {
        self.entries.iter().all(|e| !e.diverges() && !e.backup)
    }

    /// One line per finding; a single `clean` line when there are none.
    ///
    /// Findings are emitted in insertion order, so the report is stable for
    /// a stable walk of the fleet directory.
    pub fn report(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for e in &self.entries {
            match &e.head {
                Some(h) if *h != e.running => lines.push(format!(
                    "drift: {}: running {}, HEAD {}",
                    e.path,
                    short(&e.running),
                    short(h)
                )),
                None => lines.push(format!(
                    "not in HEAD: {}: running {}",
                    e.path,
                    short(&e.running)
                )),
                Some(_) => {}
            }
            if e.backup {
                lines.push(format!(
                    "backup: {}: sweep it; move the in-flight edit to a branch \
                     and commit, do not leave a .bak-* copy in the fleet \
                     directory (#4416)",
                    e.path
                ));
            }
        }
        if lines.is_empty() {
            lines.push(format!(
                "clean: {} file(s) under the fleet directory match HEAD; no .bak-* files",
                self.entries.len()
            ));
        }
        lines
    }

    /// The job's verdict: 0 when the fleet matches HEAD and is free of
    /// `.bak-*` files, 1 otherwise.
    pub fn exit_code(&self) -> i32 {
        if self.is_clean() {
            0
        } else {
            1
        }
    }
}
