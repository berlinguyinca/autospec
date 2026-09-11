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
//!
//! The stage guard (#4294).
//!
//! The apply step's outcome is not the pass's verdict. After the apply,
//! the pass asks what the patch staged — and an empty index is a symptom
//! shared by two very different facts: the patch was already in main (a
//! three-way no-op), and the apply failed all-or-nothing (a rejected
//! binary hunk stages nothing even though the other hunks applied
//! cleanly). The historical bug (#4294, InferWeave #288): a rejected
//! binary patch aborted the whole apply, staged zero files, left no
//! unmerged paths, and the pass fell through to the stage guard — which
//! concluded "already in main" and skipped, routing intact, convertible
//! work into the one bucket nobody revisits, under a reason that sounded
//! like good news. The guard violated the invariant that "a failed
//! operation never falls through to a benign conclusion" *while appearing
//! to satisfy it*: it looked like a guard, and it was the defect. Four
//! more invariants are encoded here:
//!
//! 4. **A fall-through must never be able to reach a benign conclusion**
//!    ([`stage_guard`]). A non-applied [`ApplyOutcome`] can only produce
//!    [`StageVerdict::Held`]: after a failed apply, every path terminates
//!    in a recorded failure. "Apply failed" and "apply succeeded and
//!    staged nothing" no longer reach the same line with different
//!    meanings.
//! 5. **Derive the outcome from the operation's own result, not from a
//!    downstream symptom** ([`stage_guard`]). The guard takes the
//!    classified [`ApplyOutcome`] — which carries the apply's exit status
//!    — not the state of the index. The index is consulted only for what
//!    it uniquely tells: which files the successful apply staged. An
//!    empty index is a symptom, never a conclusion.
//! 6. **A skip needs positive evidence; a hold does not**
//!    ([`StageVerdict::AlreadyInMain`]). The only path to an "already in
//!    main" skip runs through a successful apply that staged nothing
//!    *and* the caller's positive confirmation that the content is in
//!    main (e.g. the patch applies in reverse against the base). A failed
//!    apply is never skipped: holding on uncertainty costs one human
//!    glance; skipping on uncertainty costs the work.
//! 7. **Generated binary artefacts are rebuilt, never patched**
//!    ([`rejected_binaries`]). A strict failure that refused a binary
//!    hunk names the artefact(s) in its hold record and says to
//!    regenerate them with the repo's generator — agent patches exclude
//!    generated binaries, and a hold that points at the regeneration is
//!    actionable instead of opaque.

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
    /// evidence (invariant 2). When the failure refused a binary hunk,
    /// the record also names the artefact(s) and the regeneration that
    /// fixes them (invariant 7).
    StrictFailure {
        /// The captured output of the failed apply command, quoted
        /// (bounded, one line, non-empty).
        error: String,
        /// The binary artefacts the apply refused to patch, in the order
        /// observed (empty when the failure names none).
        rejected_binaries: Vec<String>,
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
    /// [`classify_apply`] guarantees is non-empty. A strict failure that
    /// refused a binary hunk names the artefact(s) and the regeneration
    /// that fixes them (invariant 7).
    pub fn hold_line(&self) -> Option<String> {
        match self {
            Self::Applied => None,
            Self::Conflicted { files, .. } => Some(format!(
                "HELD: does not apply — conflicted: {}",
                conflict_list(files)
            )),
            Self::StrictFailure {
                error,
                rejected_binaries,
            } => {
                let mut line = format!("HELD: does not apply — strict failure: {error}");
                if !rejected_binaries.is_empty() {
                    let list = rejected_binaries.join(", ");
                    line.push_str(&format!(
                        " — generated binary artefact(s) `{list}` are rebuilt, not patched: \
                         regenerate them with the repo's generator and exclude them from agent patches"
                    ));
                }
                Some(line)
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
        Ok(ApplyOutcome::StrictFailure {
            rejected_binaries: rejected_binaries(apply_error),
            error: quoted,
        })
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

/// The stage-guard verdict (#4294): what the pass does once the apply
/// step has run and the caller has read what the patch staged.
///
/// The guard is the only gate between the apply step and the "already in
/// main" skip, and it derives the verdict from the apply's own result
/// ([`ApplyOutcome`]) rather than from the state of the index alone
/// (invariant 5): an empty index is a symptom shared by a no-change
/// patch and a total failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageVerdict {
    /// The apply succeeded and staged files; the conversion continues
    /// with them.
    Proceed {
        /// The staged files, in the order observed.
        files: Vec<String>,
    },
    /// The patch's content is already in main; the pass skips. Only
    /// reachable when the apply succeeded and staged nothing, and only
    /// with the caller's positive confirmation that the content is in
    /// main (invariant 6): a skip needs positive evidence; a hold does
    /// not. A failed apply is never skipped.
    AlreadyInMain {
        /// The caller's positive confirmation that the patch's content
        /// is in main (e.g. "the patch applies in reverse against
        /// origin/main"). Quoted in the skip record so the skip's
        /// evidence survives the session that made it.
        evidence: String,
    },
    /// The apply failed; the pass holds. A failed apply never reaches a
    /// benign verdict (invariant 4): every path from a failure
    /// terminates in this recorded failure, never in a skip.
    Held {
        /// The hold line for the failure (from
        /// [`ApplyOutcome::hold_line`]), non-empty by construction.
        reason: String,
    },
}

impl StageVerdict {
    /// The log line for a terminal verdict, or `None` when the
    /// conversion continues ([`Self::Proceed`]).
    pub fn line(&self) -> Option<String> {
        match self {
            Self::Proceed { .. } => None,
            Self::AlreadyInMain { evidence } => Some(format!(
                "SKIP: stages nothing (already in main) — {evidence}"
            )),
            Self::Held { reason } => Some(reason.clone()),
        }
    }
}

/// The stage guard (#4294): classify what happened once the apply step
/// has run and the caller has read the staged-file list.
///
/// The guard takes the classified [`ApplyOutcome`] — the operation's own
/// result — plus the staged files the caller observed, plus the caller's
/// positive confirmation that the patch's content is in main, if the
/// caller has one (invariant 5): the index state is a symptom, the
/// outcome is the fact.
///
/// The rules:
///
/// - **Applied, files staged** → [`StageVerdict::Proceed`]. The
///   conversion continues.
/// - **Applied, nothing staged** → the patch was already in main (a
///   three-way no-op). The skip is only reachable with non-blank
///   `already_in_main_evidence`; without it the guard refuses to decide
///   (invariant 6). An empty index is not evidence — it is the same
///   symptom a total failure produces.
/// - **Not applied** → [`StageVerdict::Held`] with the failure's hold
///   line, unconditionally (invariant 4). `already_in_main_evidence` is
///   ignored by construction: a failed apply never skips. A patch whose
///   content is genuinely in main is a no-net-change the pass classifies
///   before the apply step, not a discovery the stage guard makes after
///   a failure. Holding on uncertainty costs one human glance; skipping
///   on uncertainty costs the work.
pub fn stage_guard(
    outcome: &ApplyOutcome,
    staged_files: &[String],
    already_in_main_evidence: Option<&str>,
) -> Result<StageVerdict, String> {
    let evidence = already_in_main_evidence
        .map(str::trim)
        .filter(|e| !e.is_empty());

    if !outcome.is_applied() {
        let reason = outcome
            .hold_line()
            .expect("a non-applied outcome carries a hold line by construction");
        return Ok(StageVerdict::Held { reason });
    }

    if !staged_files.is_empty() {
        return Ok(StageVerdict::Proceed {
            files: staged_files.to_vec(),
        });
    }

    match evidence {
        Some(e) => Ok(StageVerdict::AlreadyInMain {
            evidence: e.to_string(),
        }),
        None => Err(
            "the apply succeeded and staged nothing, but no positive evidence that the \
             content is already in main was provided — an empty index is a symptom shared \
             by a no-change patch and a failure; confirm the content is in main (e.g. that \
             the patch applies in reverse against the base) before skipping, or hold"
                .to_string(),
        ),
    }
}

/// Extract the paths of the binary artefacts the apply refused to patch,
/// from the captured error output (invariant 7).
///
/// `git apply` names a rejected binary hunk as
/// `error: cannot apply binary patch to '<path>' without full index
/// line`; the plain `error: <path>: patch does not apply` form is not
/// binary-specific and is deliberately not matched. Paths are
/// de-duplicated in the order observed.
pub fn rejected_binaries(error: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in error.lines() {
        let rest = line
            .trim()
            .strip_prefix("cannot apply binary patch to '")
            .or_else(|| {
                line.trim()
                    .strip_prefix("error: cannot apply binary patch to '")
            });
        let Some(rest) = rest else { continue };
        let Some(path) = rest.split('\'').next() else {
            continue;
        };
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        if !found.iter().any(|p| p == path) {
            found.push(path.to_string());
        }
    }
    found
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
