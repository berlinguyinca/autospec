//! The conversion gate: running each stage, and deciding whose failure it is.
//!
//! Split out of `convert.rs` (#4598). The gate is a self-contained decision —
//! run a stage, and if it fails, work out whether the change caused it — and it
//! was buried in a 2,600-line module that the repository's own size guidance had
//! been flagging for some time. Keeping it here means the attribution rules can
//! be read, and tested, without the surrounding pass.

use std::path::Path;
use std::process::{Command, Output};

use super::run_git_in;

/// The exit status a gate wrapper uses to say "I could not place this work".
///
/// 125 follows the convention `env` and `timeout` already use for "the wrapper
/// itself failed, the command never ran", so a wrapper that cannot obtain an
/// execution host is distinguishable from one whose command ran and failed.
/// Without a reserved code the two are the same non-zero integer, and a queue
/// that is merely full would be recorded as a defect in the change.
const PLACEMENT_FAILED: i32 = 125;

/// An optional command prefix the gate's work is run through, from
/// `AUTOSPEC_GATE_WRAPPER`.
///
/// The gate compiles and runs a test suite. That is real work, and it should
/// not be assumed to belong on whichever host happened to invoke the pass — for
/// a scheduled pass that host is a scheduler's submit node, shared with every
/// other user of the machine. Observed: a pass spent 96 minutes of `cargo` on a
/// login node (#4598).
///
/// autospec does not learn what any particular scheduler is. It accepts a
/// prefix and reports where the work ran; the operator supplies whatever their
/// site uses (`srun …`, `docker run …`, `ssh builder --`). With the variable
/// unset the behaviour is exactly as before: run it here.
pub(super) fn gate_wrapper() -> Vec<String> {
    parse_gate_wrapper(std::env::var("AUTOSPEC_GATE_WRAPPER").ok().as_deref())
}

/// The wrapper argv for a raw setting. Split out from the environment lookup so
/// it is testable without mutating process-wide state, which two tests running
/// in parallel cannot do safely.
pub(super) fn parse_gate_wrapper(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(raw) if !raw.trim().is_empty() => {
            raw.split_whitespace().map(str::to_string).collect()
        }
        _ => Vec::new(),
    }
}

/// Where the gate's work runs, for the report. `local` when unwrapped.
pub(super) fn gate_placement() -> String {
    let wrapper = gate_wrapper();
    if wrapper.is_empty() {
        "local".to_string()
    } else {
        wrapper.join(" ")
    }
}

pub(super) fn run_cargo(dir: &Path, stage: &[String]) -> Option<Output> {
    let wrapper = gate_wrapper();
    let mut command = match wrapper.split_first() {
        Some((program, rest)) => {
            let mut command = Command::new(program);
            command.args(rest).arg("cargo");
            command
        }
        None => Command::new("cargo"),
    };
    command.args(stage).current_dir(dir).output().ok()
}

/// Whether a stage's exit status means the work was never placed.
pub(super) fn was_never_placed(output: &Output) -> bool {
    !gate_wrapper().is_empty() && output.status.code() == Some(PLACEMENT_FAILED)
}

/// Say once per pass where the gate's work is being run.
///
/// A gate that compiles and tests must not leave that unstated: the same log
/// was produced whether the work landed on a compute node or on a shared submit
/// host, and the difference was 96 minutes of `cargo` on a login node that
/// nobody could see from the output (#4598).
pub(super) fn announce_placement() {
    static ANNOUNCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if ANNOUNCED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    eprintln!("gate: running the work via `{}`", gate_placement());
}

/// Where a failing gate stage's failure came from.
pub(super) enum StageOrigin {
    /// The stage is green at the base, so the failure is the patch's.
    Patch,
    /// The stage fails at the base too: the base is broken.
    Base,
    /// The base could not be restored or re-run, so the question is open.
    Undeterminable(String),
}

/// The stage's name for a human-facing message: `fmt`, `build`, `clippy`,
/// `test`.
pub(super) fn stage_name(stage: &[String]) -> String {
    stage
        .first()
        .cloned()
        .unwrap_or_else(|| "gate".to_string())
}

/// Re-run one gate stage with the patch removed, to decide whether the failure
/// belongs to the change or was inherited from the base.
///
/// The worktree is a fresh checkout of the base with the patch applied on top,
/// so the base is recoverable in place: `checkout -- .` reverts the tracked
/// files the patch modified and `clean -fd` removes the ones it added.
/// `target/` is ignored by the repository, so it survives and the base re-run
/// reuses the compilation cache rather than starting cold.
///
/// The worktree is left at the base afterwards. Every caller returns
/// immediately and tears the worktree down, and no caller reads the patched
/// tree after a stage has failed.
pub(super) fn stage_origin(worktree: &Path, stage: &[String]) -> StageOrigin {
    // The patch was applied with `git apply --3way`, which stages its result:
    // modified files land in the index and new files are added to it. A restore
    // that copies index-to-worktree (`checkout -- .`) therefore restores the
    // patched state onto itself, and `clean -fd` does not remove a new file the
    // index tracks — the "base" re-run ran on the patch, and every failing
    // patch was attributed to a green base (#4610). The conversion branch sits
    // at the base commit (the patch is staged, never committed), so resetting
    // index and worktree to HEAD is exactly the undo the apply deserves.
    if let Err(error) = run_git_in(worktree, &["reset", "--hard", "HEAD"]) {
        return StageOrigin::Undeterminable(format!("could not restore the base: {error}"));
    }
    if let Err(error) = run_git_in(worktree, &["clean", "-fd"]) {
        return StageOrigin::Undeterminable(format!("could not clean the base: {error}"));
    }
    match run_cargo(worktree, stage) {
        Some(output) if output.status.code() == Some(0) => StageOrigin::Patch,
        Some(_) => StageOrigin::Base,
        None => StageOrigin::Undeterminable(format!("cargo {stage:?} failed to spawn at the base")),
    }
}

/// Turn a failing stage into a verdict that says whose failure it is.
///
/// Every stage is attributed, not only the test stage. The test stage got a
/// baseline first because one incident demanded it; the same reasoning applies
/// identically to the others, and the stage that actually broke the pipeline
/// in production was `fmt`. Re-running the stage at the base costs time on a
/// failure and nothing at all on a pass — the existing trade in this module:
/// being slow is recoverable, being wrong is not.
///
/// An undeterminable base is reported as unverifiable rather than as the
/// patch's failure. The two errors are not symmetric: re-offering a good patch
/// costs a delay, while holding one writes a durable false claim about someone
/// else's change.
pub(super) fn classify_stage_failure(worktree: &Path, stage: &[String], text: String) -> GateResult {
    verdict_for(stage_origin(worktree, stage), stage, text)
}

/// The verdict a stage failure earns, given where the failure came from.
///
/// Split from the re-run so the decision itself is testable without a git
/// worktree and a compiler: this mapping is the whole point of #4596, and it
/// should not be reachable only through a ten-minute integration run.
pub(super) fn verdict_for(origin: StageOrigin, stage: &[String], text: String) -> GateResult {
    match origin {
        StageOrigin::Patch => GateResult::Fail(text),
        StageOrigin::Base => GateResult::BaseUnverifiable {
            stage: stage_name(stage),
            detail: first_lines(&text, 12),
        },
        StageOrigin::Undeterminable(why) => GateResult::BaseUnverifiable {
            stage: stage_name(stage),
            detail: why,
        },
    }
}

/// The first `n` non-empty lines of a stage's output, for a message that has to
/// stay readable in a log.
pub(super) fn first_lines(text: &str, n: usize) -> String {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .take(n)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Report a broken base once per pass, however many patches hit it.
///
/// The failure is a property of the base, not of the patches, so it is one
/// event. Printing it per patch is what turned a single unformatted file into
/// hundreds of lines that each read as a different problem.
pub(super) fn report_base_broken(base_sha: &str, stage: &str, detail: &str) {
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if REPORTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    eprintln!(
        "ALARM: the base is broken. `cargo {stage}` fails at {base}, before any patch is \
         applied. Every patch in this pass is unverifiable until the base is green; none \
         will be held for it. Fix the base, then re-run.\n{detail}",
        base = &base_sha[..base_sha.len().min(12)],
    );
}

pub(super) enum GateResult {
    Pass,
    /// A stage failed; the stage's output.
    Fail(String),
    /// A stage failed, and the *same stage fails at the base*. The patch has
    /// not been shown to be bad — it has not been shown to be anything, because
    /// the base it was measured against is broken (issue #4596).
    ///
    /// This is deliberately a third outcome and not a `Fail`. A held patch is a
    /// durable claim that the change is defective; recording that for a defect
    /// the base already had is both wrong and long-lived. A red `fmt` on trunk
    /// produced one such claim per queued patch, each naming an innocent change.
    BaseUnverifiable { stage: String, detail: String },
    /// Every stage was green but the evidence contradicts the patch: the
    /// patch adds test functions and the test count is unchanged (issue
    /// #4532). The reason names the contradiction; it is the HELD reason.
    Contradiction(String),
}


#[cfg(test)]
mod tests {
    use super::*;

    // --- #4596: a failing stage must say whose failure it is ---------------

    fn stage(name: &str) -> Vec<String> {
        vec![name.to_string(), "--check".to_string()]
    }

    #[test]
    fn a_stage_green_at_the_base_makes_the_failure_the_patchs() {
        let verdict = verdict_for(
            StageOrigin::Patch,
            &stage("fmt"),
            "Diff in src/lib.rs".to_string(),
        );
        match verdict {
            GateResult::Fail(text) => assert!(text.contains("Diff in src/lib.rs")),
            _ => panic!("a base-green stage failure belongs to the patch"),
        }
    }

    #[test]
    fn a_stage_that_also_fails_at_the_base_never_holds_the_patch() {
        // The production incident: `fmt` red on trunk, every queued patch held
        // for it. The patch is not bad -- it is unmeasured.
        let verdict = verdict_for(
            StageOrigin::Base,
            &stage("fmt"),
            "Diff in crates/core/src/process_termination.rs".to_string(),
        );
        match verdict {
            GateResult::BaseUnverifiable { stage, detail } => {
                assert_eq!(stage, "fmt");
                assert!(detail.contains("process_termination.rs"));
            }
            _ => panic!("a base failure must not be recorded against the patch"),
        }
    }

    #[test]
    fn an_undeterminable_base_is_unverifiable_rather_than_the_patchs_fault() {
        // The two errors are not symmetric. Re-offering a good patch costs a
        // delay; holding one writes a durable false claim about someone else's
        // change.
        let verdict = verdict_for(
            StageOrigin::Undeterminable("could not restore the base".to_string()),
            &stage("clippy"),
            "irrelevant".to_string(),
        );
        match verdict {
            GateResult::BaseUnverifiable { stage, detail } => {
                assert_eq!(stage, "clippy");
                assert!(detail.contains("could not restore the base"));
            }
            _ => panic!("an unanswered question is not a patch defect"),
        }
    }

    #[test]
    fn every_stage_is_attributed_not_only_the_test_stage() {
        // #4596's root cause: the test stage had a baseline and the other three
        // did not, so the stage that actually broke the pipeline (`fmt`) was
        // the one with no attribution at all.
        for name in ["fmt", "build", "clippy", "test"] {
            match verdict_for(StageOrigin::Base, &stage(name), "output".to_string()) {
                GateResult::BaseUnverifiable { stage, .. } => assert_eq!(stage, name),
                _ => panic!("{name} must attribute a base failure to the base"),
            }
        }
    }

    #[test]
    fn the_reported_detail_stays_short_enough_to_read_in_a_log() {
        let noisy = (0..500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        match verdict_for(StageOrigin::Base, &stage("build"), noisy) {
            GateResult::BaseUnverifiable { detail, .. } => {
                assert_eq!(detail.lines().count(), 12);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn blank_lines_do_not_consume_the_detail_budget() {
        let padded = "\n\n\nreal one\n\n\nreal two\n\n";
        match verdict_for(StageOrigin::Base, &stage("build"), padded.to_string()) {
            GateResult::BaseUnverifiable { detail, .. } => {
                assert_eq!(detail, "real one\nreal two");
            }
            _ => unreachable!(),
        }
    }

    // --- #4598: the gate must not assume it may compile where it was invoked --

    #[test]
    fn an_unset_wrapper_runs_the_gate_exactly_as_before() {
        assert!(parse_gate_wrapper(None).is_empty());
        assert!(parse_gate_wrapper(Some("")).is_empty());
        assert!(parse_gate_wrapper(Some("   ")).is_empty());
    }

    #[test]
    fn a_wrapper_is_split_into_a_command_and_its_arguments() {
        let wrapper = parse_gate_wrapper(Some("srun -c 8 --mem 32G -t 02:30:00"));
        assert_eq!(wrapper.first().map(String::as_str), Some("srun"));
        assert_eq!(wrapper.len(), 7);
    }

    #[test]
    fn the_wrapper_is_site_supplied_and_not_a_scheduler_autospec_knows() {
        // autospec must keep running on one machine with no scheduler at all,
        // so placement is a string the operator supplies, never a dependency.
        for raw in ["srun --", "docker run --rm img", "ssh builder --"] {
            assert!(!parse_gate_wrapper(Some(raw)).is_empty(), "{raw} should parse");
        }
    }

    #[test]
    fn a_patch_that_was_never_placed_is_unverifiable_not_defective() {
        // A full queue is not a defect in anybody's change.
        let verdict = verdict_for(
            StageOrigin::Undeterminable(
                "the work was never placed on an execution host".to_string(),
            ),
            &vec!["test".to_string()],
            String::new(),
        );
        match verdict {
            GateResult::BaseUnverifiable { detail, .. } => {
                assert!(detail.contains("never placed"))
            }
            _ => panic!("a placement failure must never be held against the patch"),
        }
    }
}
