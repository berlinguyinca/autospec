//! Per-issue conversion claims (issue #4214).
//!
//! The conversion queue determines what to work on by asking "does this issue
//! already have an open or merged PR?" A patch in flight — held by a running
//! pass that has not been given a PR yet — reads as "never attempted", so two
//! overlapping passes convert the same patch. The doubled work is cheap; the
//! doubled gate run on the contended machine is the failure this repository
//! has already proven (#4166).
//!
//! The invariants this module encodes:
//!
//! 1. **A worker pool claims work before it starts, not after it finishes.**
//!    The claim is one file per issue under the shared state directory
//!    ([`ConversionClaim::path_for`], `state/converting/<issue>`), created
//!    atomically ([`ConversionClaim::try_acquire`]) so it is visible to the
//!    next scheduler pass from the moment it exists — not when the
//!    converting process exits. The one-file-per-claim shape is the same one
//!    the fleet's `desired.sh` uses.
//! 2. **A pass refuses to start an issue another pass holds, and says so.**
//!    [`ConversionClaim::try_acquire`] returns [`ConversionClaimError::Held`]
//!    naming the holder; the rendered refusal ([`ConversionClaimError::line`])
//!    is what the pass reports instead of starting the duplicate.
//! 3. **The eligibility question is "is anyone working on this?", never
//!    "has anyone worked on this?"** A claim that predates the PR is not
//!    evidence the work is done; the convertible set must exclude in-flight
//!    claims, which [`in_flight`] makes queryable for exactly that purpose.
//!
//! The claim is the only filesystem piece here, and it mirrors
//! [`crate::gate_serialization::GateLock`]: the acquire is a `create_new`
//! claim so two passes cannot both believe they hold an issue, and
//! [`ConversionClaim::release`] refuses to delete a file another holder now
//! owns. No advisory locking is used, so the state is durable across process
//! restarts and correct on shared network storage.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Errors from [`ConversionClaim::try_acquire`] and [`ConversionClaim::release`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversionClaimError {
    /// The issue is already claimed by another pass: refuse to start and say
    /// so ([`ConversionClaimError::line`]).
    Held {
        /// The issue the claim is on.
        issue: u64,
        /// The holder's pid, if the claim file carried one.
        holder_pid: Option<u32>,
    },
    /// A filesystem error.
    Io(String),
}

impl ConversionClaimError {
    /// The refusal as a log line: the pass reports this instead of starting
    /// the duplicate conversion.
    pub fn line(&self) -> String {
        match self {
            Self::Held { issue, holder_pid } => {
                let holder = holder_pid
                    .map(|pid| format!("held by pass {pid}"))
                    .unwrap_or_else(|| "holder pid unreadable".to_string());
                format!(
                    "issue {issue} is already being converted ({holder}); \
                     refusing to start"
                )
            }
            Self::Io(error) => format!("conversion claim error: {error}"),
        }
    }
}

/// One pass's claim on one issue's conversion (issue #4214).
///
/// Invariant 1: the claim exists before the pass does any work, so the next
/// scheduler pass sees it while the converting pass is still running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionClaim {
    path: PathBuf,
    issue: u64,
    holder_pid: u32,
}

impl ConversionClaim {
    /// The canonical claim path: one file per issue under the state root,
    /// so the claim is visible to every pass sharing the state directory.
    pub fn path_for(state_root: &Path, issue: u64) -> PathBuf {
        state_root.join("converting").join(issue.to_string())
    }

    /// Try to claim the conversion of one issue.
    ///
    /// The claim is atomic (`create_new`): a concurrent pass gets
    /// [`ConversionClaimError::Held`], not a race — invariant 2. The file
    /// carries the holder's pid so a later reader — and
    /// [`ConversionClaim::release`] — can tell whose claim it is looking at.
    pub fn try_acquire(
        state_root: &Path,
        issue: u64,
    ) -> Result<ConversionClaim, ConversionClaimError> {
        let path = Self::path_for(state_root, issue);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| ConversionClaimError::Io(error.to_string()))?;
        }
        let holder_pid = std::process::id();
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(ConversionClaimError::Held {
                    issue,
                    holder_pid: read_holder_pid(&path),
                });
            }
            Err(error) => return Err(ConversionClaimError::Io(error.to_string())),
        };
        file.write_all(format!("{holder_pid}\n").as_bytes())
            .map_err(|error| ConversionClaimError::Io(error.to_string()))?;
        Ok(ConversionClaim {
            path,
            issue,
            holder_pid,
        })
    }

    /// Release the claim.
    ///
    /// Refuses to delete a claim that another pass now holds: the file is
    /// checked against the holder this handle claims to be, and a mismatch is
    /// [`ConversionClaimError::Held`], never a silent deletion of someone
    /// else's claim. A claim file that is already gone has nothing left to
    /// protect and releases cleanly.
    pub fn release(self) -> Result<(), ConversionClaimError> {
        match fs::metadata(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(ConversionClaimError::Io(error.to_string())),
        }
        let holder = read_holder_pid(&self.path).ok_or(ConversionClaimError::Held {
            issue: self.issue,
            holder_pid: None,
        })?;
        if holder != self.holder_pid {
            return Err(ConversionClaimError::Held {
                issue: self.issue,
                holder_pid: Some(holder),
            });
        }
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ConversionClaimError::Io(error.to_string())),
        }
    }
}

/// The holder pid recorded in a claim file, if it is one.
fn read_holder_pid(path: &Path) -> Option<u32> {
    let content = fs::read_to_string(path).ok()?;
    content.trim().parse().ok()
}

/// The issues claimed by in-flight conversion passes (issue #4214).
///
/// Invariant 3, made queryable: the eligibility check asks what is claimed
/// now, not what has ever had a PR. Only numeric file names are claims;
/// anything else under `state/converting/` is not one and is not a claim on
/// any issue.
pub fn in_flight(state_root: &Path) -> Result<BTreeSet<u64>, String> {
    let dir = state_root.join("converting");
    match fs::read_dir(&dir) {
        Ok(entries) => {
            let mut claimed = BTreeSet::new();
            for entry in entries {
                let entry = entry.map_err(|error| error.to_string())?;
                let file_name = entry.file_name();
                let Some(name) = file_name.to_str() else {
                    continue;
                };
                let Ok(issue) = name.parse::<u64>() else {
                    continue;
                };
                claimed.insert(issue);
            }
            Ok(claimed)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("autospec-conversion-claim-{name}-{nonce}"));
        fs::create_dir_all(&root).expect("temp root");
        root
    }

    // --- Invariant 1: the claim is visible before the work starts --------

    #[test]
    fn the_claim_is_one_file_per_issue_under_the_state_root() {
        let root = Path::new("/state");
        assert_eq!(
            ConversionClaim::path_for(root, 3728),
            PathBuf::from("/state/converting/3728")
        );
    }

    #[test]
    fn a_claim_is_visible_to_the_next_pass_while_held() {
        let root = temp_root("visible");
        let claim = ConversionClaim::try_acquire(&root, 3728).expect("acquire");
        // The next scheduler pass scans the directory and sees the issue.
        let seen = in_flight(&root).expect("scan");
        assert!(seen.contains(&3728), "the held claim is listed: {seen:?}");
        claim.release().expect("release");
        assert!(
            !in_flight(&root).expect("scan").contains(&3728),
            "the released claim is gone"
        );
        let _ = fs::remove_dir_all(&root);
    }

    // --- Invariant 2: the second pass is refused and named ---------------

    #[test]
    fn the_second_pass_is_refused_and_named() {
        let root = temp_root("held");
        let first = ConversionClaim::try_acquire(&root, 4125).expect("first pass claims");
        let error = ConversionClaim::try_acquire(&root, 4125).expect_err("second pass is refused");
        assert_eq!(
            error,
            ConversionClaimError::Held {
                issue: 4125,
                holder_pid: Some(std::process::id()),
            }
        );
        // The refusal says so, naming the issue and the holder.
        let line = error.line();
        assert!(line.contains("issue 4125"), "{line}");
        assert!(line.contains("refusing to start"), "{line}");
        // The first pass is undisturbed by the refused second.
        assert!(ConversionClaim::path_for(&root, 4125).exists());
        first.release().expect("release");
        // Released: the next pass may start.
        let next = ConversionClaim::try_acquire(&root, 4125).expect("re-acquire after release");
        next.release().expect("release");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn one_pass_may_claim_several_issues_at_once() {
        // The running pass was holding 3728 3737 4125 4135 3741 unprocessed.
        let root = temp_root("multi");
        let mut claims = Vec::new();
        for issue in [3728u64, 3737, 4125, 4135, 3741] {
            claims.push(ConversionClaim::try_acquire(&root, issue).expect("acquire"));
        }
        let seen = in_flight(&root).expect("scan");
        assert_eq!(seen, [3728, 3737, 3741, 4125, 4135].into_iter().collect());
        for claim in claims {
            claim.release().expect("release");
        }
        assert!(in_flight(&root).expect("scan").is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn release_refuses_to_delete_a_claim_it_does_not_hold() {
        let root = temp_root("release");
        let first = ConversionClaim::try_acquire(&root, 4135).expect("acquire");
        // The file is overwritten with a holder pid that is not ours: a
        // stale claim, or someone else's. Deleting it would be wrong.
        fs::write(ConversionClaim::path_for(&root, 4135), "999999\n").expect("overwrite");
        let error = first.release().expect_err("release is refused");
        assert_eq!(
            error,
            ConversionClaimError::Held {
                issue: 4135,
                holder_pid: Some(999999),
            }
        );
        assert!(
            ConversionClaim::path_for(&root, 4135).exists(),
            "foreign claim untouched"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_claim_file_is_not_a_held_claim() {
        // The file vanished under a holder (manual cleanup): release is a
        // no-op success, because there is nothing left to protect.
        let root = temp_root("vanished");
        let first = ConversionClaim::try_acquire(&root, 3737).expect("acquire");
        fs::remove_file(ConversionClaim::path_for(&root, 3737)).expect("remove");
        first
            .release()
            .expect("release of a vanished claim is clean");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreadable_holder_is_still_held() {
        let root = temp_root("unreadable");
        let first = ConversionClaim::try_acquire(&root, 3741).expect("acquire");
        fs::write(ConversionClaim::path_for(&root, 3741), "not-a-pid\n").expect("overwrite");
        let error = ConversionClaim::try_acquire(&root, 3741).expect_err("still refused");
        assert_eq!(
            error,
            ConversionClaimError::Held {
                issue: 3741,
                holder_pid: None,
            }
        );
        assert!(error.line().contains("holder pid unreadable"));
        // The owner cannot release a claim it no longer recognises.
        assert!(first.release().is_err());
        let _ = fs::remove_dir_all(&root);
    }

    // --- Invariant 3: in-flight is an existence question -----------------

    #[test]
    fn in_flight_lists_only_numeric_claim_files() {
        let root = temp_root("scan");
        let claim = ConversionClaim::try_acquire(&root, 3845).expect("acquire");
        // A non-numeric name under the directory is not a claim on any
        // issue and does not corrupt the scan.
        fs::write(root.join("converting").join("notes.txt"), "x\n").expect("write");
        let seen = in_flight(&root).expect("scan");
        assert_eq!(seen, [3845].into_iter().collect());
        claim.release().expect("release");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn in_flight_on_an_absent_directory_is_an_empty_set() {
        let root = temp_root("absent");
        assert!(in_flight(&root).expect("scan").is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
