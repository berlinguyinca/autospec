//! Apply-step outcome classification and hold recording (#3980).
//!
//! The conversion pass's apply step (`git apply --3way`) has two distinct
//! failure shapes, and a hold record must distinguish them — the repair an
//! operator performs hours later depends on which one happened:
//!
//! - **Conflicted**: the three-way merge ran and left conflict markers in
//!   the worktree. The failure is *described by the worktree itself* —
//!   `git diff --name-only --diff-filter=U` names the files, and each
//!   file's content gives the hunk count and declaration-only (registry-only)
//!   shape (see [`super::patch_conflicts::hold_shape`]). The resolution
//!   path is [`super::patch_conflicts::resolve`].
//! - **Strict failure**: the apply was refused outright — most often the
//!   worktree lacks the patch's base blobs ("error: repository lacks the
//!   necessary object to perform 3-way merge") — and *nothing* was left
//!   behind. The unmerged-file list is empty by construction. The only
//!   description of the failure is the command's own error output.
//!
//! The historical bug (#3980, observed 2026-09-09 in issue #3892): the
//! pass discarded the apply command's output (`>/dev/null 2>&1`) and built
//! the hold reason from the unmerged-file list — so a strict failure was
//! recorded as `HELD: does not apply -- ` with the reason *empty*. The
//! record pointed at nothing: the file list was empty by construction, and
//! the one artefact that said *why* the patch did not apply had been
//! thrown away.
//!
//! The invariants encoded here:
//!
//! 1. **Never discard the output of a command whose failure is later
//!    described** ([`classify_apply`]). A failed apply must be classified
//!    with its captured output; an empty capture on a failed command is a
//!    protocol error, not an empty reason. This is what makes the empty
//!    hold record unrepresentable: instead of rendering
//!    `HELD: does not apply -- ` the pass fails closed with a stated
//!    reason and the caller fixes its capture.
//! 2. **The two failure shapes get two different records**
//!    ([`ApplyOutcome::hold_line`]). A conflicted hold names every
//!    conflicted file with its hunk count and registry-only flag; a
//!    strict-failure hold quotes the captured error. Neither borrows the
//!    other's evidence.
//! 3. **A quoted error is bounded, never elided** ([`quote_error`]): the
//!    first lines of the output, capped at a byte budget, with an explicit
//!    `…` when truncated and newlines folded so the record stays one log
//!    line. The quote is non-empty by construction.

use super::patch_conflicts::{hold_shape, HoldShape};

/// Lines of captured error output kept in a hold record.
const MAX_QUOTED_LINES: usize = 10;
/// Bytes of captured error output kept in a hold record.
const MAX_QUOTED_BYTES: usize = 2000;

/// One conflicted file as the caller observed it: the path from
/// `git diff --name-only --diff-filter=U` plus its on-disk content, so the
/// hold record can state the hunk count and the declaration-only shape
/// instead of only the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictObservation {
    /// Repo-relative path of the conflicted file.
    pub path: String,
    /// The file's on-disk content, conflict markers included.
    pub content: String,
}

/// The outcome of one `git apply --3way` attempt, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// The patch applied cleanly; the conversion continues.
    Applied,
    /// The three-way merge left conflict markers in the worktree. The hold
    /// record names every conflicted file with its hunk count and
    /// registry-only flag (invariant 2); the resolution path is
    /// [`super::patch_conflicts::resolve`].
    Conflicted {
        /// One shape per conflicted file, in the order observed.
        files: Vec<HoldShape>,
        /// The captured output of the failed apply command, quoted
        /// (bounded, one line) — kept for the log even though the record
        /// is carried by the file list.
        error: String,
    },
    /// The apply was refused outright and left nothing behind (typically
    /// the base blobs are unreachable). The hold record quotes the
    /// captured error; with no markers on disk the error text is the only
    /// evidence (invariant 2).
    StrictFailure {
        /// The captured output of the failed apply command, quoted
        /// (bounded, one line, non-empty).
        error: String,
    },
}

impl ApplyOutcome {
    pub fn is_applied(&self) -> bool {
        matches!(self, Self::Applied)
    }

    /// The durable hold record for a non-applied outcome — `None` when the
    /// patch applied and there is nothing to hold.
    ///
    /// The reason after `HELD: does not apply` is non-empty by
    /// construction in both hold arms: the conflicted arm carries at least
    /// one file shape, and the strict-failure arm carries a quote that
    /// [`classify_apply`] guarantees is non-empty.
    pub fn hold_line(&self) -> Option<String> {
        match self {
            Self::Applied => None,
            Self::Conflicted { files, .. } => Some(format!(
                "HELD: does not apply — conflicted: {}",
                conflict_list(files)
            )),
            Self::StrictFailure { error } => {
                Some(format!("HELD: does not apply — strict failure: {error}"))
            }
        }
    }
}

/// Join the conflicted file shapes into the `path: N hunk(s), shape` list
/// the conflicted hold record is built from (one entry per file, `; `-separated).
fn conflict_list(files: &[HoldShape]) -> String {
    files
        .iter()
        .map(HoldShape::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Compose the canonical capture of a failed command's output from its two
/// streams: stderr first (git writes apply failures there), and stdout
/// only when stderr was silent (some git versions write the three-way
/// report to stdout). The result is trimmed; an empty result means the
/// caller captured nothing, which [`classify_apply`] rejects as a
/// protocol error (invariant 1).
pub fn captured_error(stderr: &str, stdout: &str) -> String {
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    stdout.trim().to_string()
}

/// Classify the outcome of a `git apply --3way` attempt from what the
/// caller observed.
///
/// - `exit_status` — the apply command's exit code (0 = applied).
/// - `apply_error` — the *captured* output of the command (compose it
///   with [`captured_error`]). Mandatory whenever the command failed:
///   the hold record quotes it, so an empty capture on a failed command
///   is a protocol error, not an empty reason (invariant 1).
/// - `conflicted_files` — the unmerged files (`--diff-filter=U`) with
///   their on-disk content, empty when nothing was left behind.
///
/// Errors (fail closed, never an empty hold record):
/// - exit 0 with conflicted files: inconsistent observation — a
///   successful `git apply --3way` does not leave markers.
/// - non-zero exit with an empty `apply_error`: the output was discarded
///   (the #3980 bug shape); the caller must capture it.
pub fn classify_apply(
    exit_status: i32,
    apply_error: &str,
    conflicted_files: &[ConflictObservation],
) -> Result<ApplyOutcome, String> {
    if exit_status == 0 {
        if conflicted_files.is_empty() {
            return Ok(ApplyOutcome::Applied);
        }
        return Err(format!(
            "the apply command exited 0 but left {} conflicted file(s) on disk; \
             inconsistent observation — re-check the worktree before recording a verdict",
            conflicted_files.len()
        ));
    }
    if apply_error.trim().is_empty() {
        return Err(
            "the apply command failed but its output was not captured; a hold record \
             must quote the failure it describes — capture the command's output \
             instead of discarding it (#3980)"
                .to_string(),
        );
    }
    let quoted = quote_error(apply_error);
    if conflicted_files.is_empty() {
        Ok(ApplyOutcome::StrictFailure { error: quoted })
    } else {
        let files = conflicted_files
            .iter()
            .map(|observation| hold_shape(&observation.path, &observation.content))
            .collect();
        Ok(ApplyOutcome::Conflicted {
            files,
            error: quoted,
        })
    }
}

/// Quote captured command output for embedding in a one-line hold record
/// (invariant 3): at most [`MAX_QUOTED_LINES`] lines and
/// [`MAX_QUOTED_BYTES`] bytes, newlines folded to ` | `, an explicit `…`
/// when anything was dropped. `error` must be non-blank; the result
/// never is.
fn quote_error(error: &str) -> String {
    let trimmed = error.trim();
    let total_lines = trimmed.lines().count();
    let mut lines: Vec<&str> = trimmed
        .lines()
        .take(MAX_QUOTED_LINES)
        .map(str::trim)
        .collect();
    if total_lines > lines.len() {
        lines.push("…");
    }
    let mut quoted = lines.join(" | ");
    if quoted.len() > MAX_QUOTED_BYTES {
        let mut keep = MAX_QUOTED_BYTES - 3; // room for the `…` (3 bytes)
        while keep > 0 && !quoted.is_char_boundary(keep) {
            keep -= 1;
        }
        quoted.truncate(keep);
        quoted = quoted.trim_end().to_string();
        quoted.push('…');
    }
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(path: &str, content: &str) -> ConflictObservation {
        ConflictObservation {
            path: path.to_string(),
            content: content.to_string(),
        }
    }

    const CONFLICT_CONTENT: &str = "\
<<<<<<< HEAD
pub mod config;
=======
pub mod config;
pub mod correlate;
>>>>>>> feat/insights-correlate
";

    #[test]
    fn clean_apply_applies() {
        let outcome = classify_apply(0, "", &[]).expect("clean apply");
        assert!(outcome.is_applied());
        assert!(outcome.hold_line().is_none(), "nothing to hold");
    }

    #[test]
    fn strict_failure_quotes_the_captured_error() {
        // The #3980 shape: base blobs unreachable, no markers left.
        let error = "error: repository lacks the necessary object to perform 3-way merge.\n";
        let outcome = classify_apply(1, error, &[]).expect("strict failure");
        assert!(!outcome.is_applied());
        let line = outcome.hold_line().expect("a strict failure holds");
        assert!(
            line.starts_with("HELD: does not apply — strict failure: "),
            "{line}"
        );
        assert!(
            line.contains("repository lacks the necessary object"),
            "{line}"
        );
        // The reason is not empty — the historical bug.
        assert!(
            line.trim_end().ends_with("3-way merge."),
            "the captured error must be quoted, got: {line}"
        );
    }

    #[test]
    fn an_uncaptured_failure_is_a_protocol_error_not_an_empty_reason() {
        // The #3980 bug shape, rejected instead of rendered.
        let err = classify_apply(1, "", &[]).expect_err("empty capture on failure");
        assert!(err.contains("not captured"), "{err}");
        assert!(err.contains("#3980"), "{err}");

        // Whitespace-only capture is the same bug in a noisier disguise.
        let err = classify_apply(2, "  \n\t ", &[]).expect_err("blank capture on failure");
        assert!(err.contains("not captured"), "{err}");
    }

    #[test]
    fn conflicted_outcome_names_files_hunks_and_shape() {
        let code_content = "<<<<<<< HEAD\nlet a = 1;\n=======\nlet a = 2;\n>>>>>>> feat/x\n";
        let outcome = classify_apply(
            1,
            "Applied patch to 'crates/autospec-core/src/insights/mod.rs' with conflicts.\n",
            &[
                obs("crates/autospec-core/src/insights/mod.rs", CONFLICT_CONTENT),
                obs("crates/autospec-core/src/lib.rs", code_content),
            ],
        )
        .expect("conflicted");
        assert!(!outcome.is_applied());
        let line = outcome.hold_line().expect("a conflict holds");
        assert!(
            line.starts_with("HELD: does not apply — conflicted: "),
            "{line}"
        );
        assert!(
            line.contains(
                "crates/autospec-core/src/insights/mod.rs: 1 hunk(s), module declarations only"
            ),
            "{line}"
        );
        assert!(
            line.contains("crates/autospec-core/src/lib.rs: 1 hunk(s), code present"),
            "{line}"
        );
        assert!(
            line.contains("; "),
            "multiple files are joined, not dropped: {line}"
        );
        // The captured output is kept for the log even though the file
        // list carries the record.
        match &outcome {
            ApplyOutcome::Conflicted { error, .. } => {
                assert!(error.contains("with conflicts"), "{error}");
            }
            other => panic!("expected Conflicted, got {other:?}"),
        }
    }

    #[test]
    fn a_successful_apply_with_markers_is_inconsistent() {
        let err = classify_apply(
            0,
            "",
            &[obs("crates/autospec-core/src/lib.rs", CONFLICT_CONTENT)],
        )
        .expect_err("exit 0 with markers");
        assert!(err.contains("inconsistent observation"), "{err}");
    }

    #[test]
    fn the_quoted_error_is_bounded_and_one_line() {
        let big: String = (1..=60)
            .map(|i| format!("error: line {i} of a very long failure report"))
            .collect::<Vec<_>>()
            .join("\n");
        let outcome = classify_apply(1, &big, &[]).expect("strict failure");
        let line = outcome.hold_line().expect("hold");
        assert!(!line.contains('\n'), "the record stays one line: {line}");
        assert!(line.contains('…'), "truncation is explicit: {line}");
        // 10 kept lines, well under the byte budget.
        let reason = line
            .strip_prefix("HELD: does not apply — strict failure: ")
            .unwrap();
        assert!(
            reason.len() <= MAX_QUOTED_BYTES,
            "reason is {len} bytes",
            len = reason.len()
        );
    }

    #[test]
    fn a_huge_single_line_is_truncated_on_a_char_boundary() {
        let big = format!("error: {}", "x".repeat(5000));
        let outcome = classify_apply(1, &big, &[]).expect("strict failure");
        let line = outcome.hold_line().expect("hold");
        let reason = line
            .strip_prefix("HELD: does not apply — strict failure: ")
            .unwrap();
        assert!(
            reason.len() <= MAX_QUOTED_BYTES,
            "reason is {len} bytes",
            len = reason.len()
        );
        assert!(reason.ends_with('…'), "{reason}");
    }

    #[test]
    fn captured_error_prefers_stderr_and_falls_back_to_stdout() {
        assert_eq!(
            captured_error("error: patch does not apply\n", ""),
            "error: patch does not apply"
        );
        assert_eq!(
            captured_error("  ", "Applied patch with conflicts.\n"),
            "Applied patch with conflicts."
        );
        assert_eq!(captured_error("", "   "), "");
    }
}
