//! # The conversion-pass guard (issue #3654)
//!
//! A worktree lock that excludes only other converters still lets a human
//! operator reset, clean, or prune a worktree in the middle of a
//! conversion pass. That is exactly what produced the "HELD: build error"
//! verdict: maintenance corrupted the tree under an in-flight pass, and the
//! pass memoized the corrupted result in the durable memo keyed on (patch
//! hash, base sha), where it poisoned every later pass over the same input.
//!
//! The guard has four parts, one per acceptance criterion:
//!
//! 1. [`marker::Marker`] — a marker file inside the worktree naming the
//!    holder and its PID, acquired atomically and released on drop.
//! 2. [`maintenance`] — the shared reset/clean/prune helpers, which check
//!    the marker and refuse while a pass is in flight.
//! 3. [`run_pass`] — a pass runs under the marker, and its verdict commits
//!    to the [`memo::VerdictMemo`] only after the pass completed cleanly;
//!    an error or a panic leaves the memo untouched.
//! 4. [`memo::VerdictMemo::invalidate`] — the cheap way to drop a suspect
//!    cached verdict, with a logged line naming the dropped key.

pub mod maintenance;
pub mod marker;
pub mod memo;

pub use maintenance::{clean, maintain, prune, reset, MaintenanceAction, MaintenanceError};
pub use marker::{Marker, MarkerError, MarkerRecord, MARKER_DIR, MARKER_FILE, MAX_HOLDER_LEN};
pub use memo::{MemoError, MemoKey, Verdict, VerdictMemo};

use std::path::Path;

/// Run one conversion pass under the worktree marker and commit its
/// verdict to the memo (acceptance criteria 1 and 3).
///
/// `op` performs the conversion and returns the verdict it reached. The
/// verdict is written to `memo` under `key` only when `op` returns `Ok` —
/// a pass that errored, or panicked, ran into something (possibly human
/// maintenance) and says nothing trustworthy about the target. The marker
/// is held for the whole pass and released on every exit, panic included.
///
/// Refuses to start when the worktree is already held by someone else.
pub fn run_pass(
    worktree: &Path,
    holder: &str,
    key: &MemoKey,
    memo: &mut VerdictMemo,
    op: impl FnOnce() -> Result<Verdict, String>,
) -> Result<Verdict, String> {
    // The marker lives for the whole pass; `Drop` releases it on every
    // exit, panic included. `_marker`: the binding is intentional ballast.
    let _marker = Marker::acquire(worktree, holder).map_err(|error| error.to_string())?;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(op));
    let verdict = match outcome {
        Ok(Ok(verdict)) => verdict,
        Ok(Err(reason)) => return Err(reason),
        Err(payload) => std::panic::resume_unwind(payload),
    };
    memo.record(key, &verdict)
        .map_err(|error| format!("commit verdict to memo: {error}"))?;
    Ok(verdict)
}
