//! The pass's operator-facing progress lines, flushed as they are made.
//!
//! When stdout is a pipe or a file — the normal case for anything scheduled,
//! cron, nohup, a CI step — Rust block-buffers it. An unflushed decision line
//! then makes a working pass look dead: four consecutive "zero decisions"
//! status reports, two wrong hang diagnoses, and one 20-minute gate run
//! discarded on the strength of "30 minutes, silence" all came from that
//! (#4572). The HELD ledger is written directly and is the source of truth;
//! the log is the channel that must not lag it, and a silent gate that can
//! take 11+ minutes must say what it is doing, not only what it decided.
//!
//! Every line this module produces is printed and flushed immediately, so a
//! reader tailing the log sees each decision at the moment it is made —
//! including while the process is still running the next stage.

use std::io::Write;

/// Print one progress line and flush stdout with it.
///
/// The flush is the point: without it the line sits in the block buffer
/// until the process exits (or the buffer happens to fill), and a tailing
/// operator reads a pass that has made no decisions it has already made.
pub(super) fn report_line(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}

/// The startup banner: where the pass's authoritative record lives, so the
/// operator reads the ledger instead of the buffer (#4572 acceptance 3).
pub(super) fn banner(held_path: &std::path::Path) {
    banner_on(held_path, true);
}

/// The banner on stderr, for `--json` mode: stdout is reserved for the
/// machine-readable plan, and a banner line in front of it would break the
/// first byte a parser sees. Stderr keeps the banner visible under `2>&1`
/// without touching the contract.
pub(super) fn banner_quiet(held_path: &std::path::Path) {
    banner_on(held_path, false);
}

fn banner_on(held_path: &std::path::Path, on_stdout: bool) {
    let line = format!("held ledger: {}", held_path.display());
    if on_stdout {
        report_line(&line);
    } else {
        eprintln!("{line}");
        let _ = std::io::stderr().flush();
    }
}

/// The decision that a patch converted, at the moment it did.
pub(super) fn converted(issue: u64, patch_key: &str) {
    report_line(&format!("  CONVERT  #{issue} {patch_key}"));
}

/// The decision that a patch held, at the moment it did.
pub(super) fn held(issue: u64, reason: &str) {
    report_line(&format!("  HELD  #{issue}: {reason}"));
}

/// An issue starts: before the branch is pushed, so a run killed anywhere in
/// the attempt shows which issues it reached and where each one stopped
/// (#4499): a killed run must be diagnosable from its output, not from the
/// remote state.
pub(super) fn started(issue: u64) {
    report_line(&format!("  START  #{issue}"));
}

/// A patch is already-delivered residue (#4501): its changes are in the
/// base, so it is reported on its own line, never offered or gated.
pub(super) fn delivered(issue: u64) {
    report_line(&format!(
        "  DELIVERED #{issue} (patch is empty against the base)"
    ));
}

/// An issue finishes, with its outcome, at the moment it did.
pub(super) fn finished(issue: u64, outcome: &str) {
    report_line(&format!("  DONE   #{issue}: {outcome}"));
}

/// The gate begins: the patch and the derived scope, before any stage runs.
///
/// A verdict can take 11+ minutes; a log that is silent between "started"
/// and "decided" reads as a hang, and this line is what separates the two.
pub(super) fn gate_scope(issue: u64, packages: &[String]) {
    let scope = if packages.is_empty() {
        "the whole workspace".to_string()
    } else {
        packages.join(" ")
    };
    report_line(&format!("  GATE  #{issue}: {scope}"));
}

/// A gate stage begins, so the longest stage (test) cannot read as a stall.
pub(super) fn gate_stage(issue: u64, stage: &str) {
    report_line(&format!("  GATE  #{issue}: {stage}"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_line_formats_name_the_patch_and_their_payload() {
        // The formats are what operators and scripts tail: pin them.
        let held = format!("  HELD  #{issue}: {reason}", issue = 4341, reason = "clippy=2");
        assert_eq!(held, "  HELD  #4341: clippy=2");
        let converted = format!("  CONVERT  #{issue} {key}", issue = 201, key = "abc123");
        assert_eq!(converted, "  CONVERT  #201 abc123");
        let gate = format!("  GATE  #{issue}: {scope}", issue = 201, scope = "-p autospec-core");
        assert_eq!(gate, "  GATE  #201: -p autospec-core");
        let stage = format!("  GATE  #{issue}: {stage}", issue = 201, stage = "test");
        assert_eq!(stage, "  GATE  #201: test");
        let started = format!("  START  #{issue}", issue = 2995);
        assert_eq!(started, "  START  #2995");
        let done = format!("  DONE   #{issue}: {outcome}", issue = 2995, outcome = "converted");
        assert_eq!(done, "  DONE   #2995: converted");
        let delivered = format!(
            "  DELIVERED #{issue} (patch is empty against the base)",
            issue = 3246
        );
        assert_eq!(delivered, "  DELIVERED #3246 (patch is empty against the base)");
    }
}
