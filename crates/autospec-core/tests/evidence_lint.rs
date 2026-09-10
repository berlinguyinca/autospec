//! AS-EVD-001 / AS-EVD-002: a failure must not name evidence it destroys.
//!
//! Covers GitHub issue #3882. Positive cases (defective scripts fire),
//! negative cases (well-behaved scripts stay silent), and the populated
//! negative case from acceptance criterion 3: force the failure of a
//! well-behaved script and confirm the evidence is present in the completed
//! run's output (the #3793 lesson: print the evidence, do not point at a
//! file the trap will delete).

#![cfg_attr(not(unix), allow(dead_code))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::lint::evidence::{
    lint_failure_evidence, trap_removal_targets, EvidenceFinding, EvidenceRule,
};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);

struct TempScriptRoot {
    path: PathBuf,
}

impl TempScriptRoot {
    fn new() -> Self {
        let nonce = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autospec-evidence-lint-{nonce}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary root is created");
        Self { path }
    }

    fn write_script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.path.join(name);
        fs::write(&path, body).expect("script is written");
        path
    }
}

impl Drop for TempScriptRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run_bash(script: &Path) -> (i32, String) {
    let output = Command::new("bash")
        .arg(script)
        .env("FORCE_FAILURE", "1")
        .output()
        .expect("bash runs the script");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), text)
}

// ---------------------------------------------------------------------------
// Rule ids
// ---------------------------------------------------------------------------

#[test]
fn rule_ids_are_stable_as_evd_001_and_002() {
    assert_eq!(EvidenceRule::UncapturedCommandReference.id(), "AS-EVD-001");
    assert_eq!(EvidenceRule::TrapDestroysReferencedFile.id(), "AS-EVD-002");
}

// ---------------------------------------------------------------------------
// AS-EVD-001: failure message names a command whose output was never captured
// ---------------------------------------------------------------------------

const CMD_NOT_CAPTURED: &str = r#"#!/usr/bin/env bash
set -uo pipefail
if ! docker compose -p inferweave-gateway up; then
  echo "gateway failed to start (see `docker compose -p inferweave-gateway logs`)" >&2
  exit 1
fi
"#;

#[test]
fn cmd_reference_without_capture_is_flagged() {
    let findings = lint_failure_evidence(CMD_NOT_CAPTURED);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.rule, EvidenceRule::UncapturedCommandReference);
    assert_eq!(finding.line, 4);
    assert_eq!(finding.subject, "docker compose -p inferweave-gateway logs");
    assert!(finding.message.contains("never captures"));
}

const CMD_CAPTURED_REDIRECT: &str = r#"#!/usr/bin/env bash
set -uo pipefail
docker compose -p inferweave-gateway logs > logs/gateway.log 2>&1
if ! docker compose -p inferweave-gateway up; then
  echo "gateway failed to start (see `docker compose -p inferweave-gateway logs`)" >&2
  exit 1
fi
"#;

#[test]
fn cmd_reference_with_redirect_capture_is_silent() {
    assert!(
        lint_failure_evidence(CMD_CAPTURED_REDIRECT).is_empty(),
        "{:?}",
        lint_failure_evidence(CMD_CAPTURED_REDIRECT)
    );
}

const CMD_CAPTURED_TEE: &str = r#"#!/usr/bin/env bash
set -uo pipefail
docker compose -p inferweave-gateway logs 2>&1 | tee logs/gateway.log
if ! docker compose -p inferweave-gateway up; then
  echo "gateway failed to start (see `docker compose -p inferweave-gateway logs`)" >&2
  exit 1
fi
"#;

#[test]
fn cmd_reference_with_tee_capture_is_silent() {
    assert!(lint_failure_evidence(CMD_CAPTURED_TEE).is_empty());
}

const CMD_CAPTURED_SUBSTITUTION: &str = r#"#!/usr/bin/env bash
set -uo pipefail
gateway_logs="$(docker compose -p inferweave-gateway logs 2>&1)"
if ! docker compose -p inferweave-gateway up; then
  echo "gateway failed to start (see `docker compose -p inferweave-gateway logs`)" >&2
  exit 1
fi
"#;

#[test]
fn cmd_reference_with_command_substitution_capture_is_silent() {
    assert!(lint_failure_evidence(CMD_CAPTURED_SUBSTITUTION).is_empty());
}

const CMD_LOGICAL_OR_IS_NOT_CAPTURE: &str = r#"#!/usr/bin/env bash
set -uo pipefail
docker compose -p inferweave-gateway logs || :
if ! docker compose -p inferweave-gateway up; then
  echo "gateway failed to start (see `docker compose -p inferweave-gateway logs`)" >&2
  exit 1
fi
"#;

#[test]
fn cmd_run_with_logical_or_only_is_still_flagged() {
    let findings = lint_failure_evidence(CMD_LOGICAL_OR_IS_NOT_CAPTURE);
    assert_eq!(findings.len(), 1, "|| is not a capture: {findings:?}");
    assert_eq!(findings[0].rule, EvidenceRule::UncapturedCommandReference);
}

const PROSE_FILE_REFERENCE: &str = r#"#!/usr/bin/env bash
set -uo pipefail
if [ ! -f "build/output.bin" ]; then
  echo "build failed (see `docs/BUILD-ERRORS.md`)" >&2
  exit 1
fi
"#;

#[test]
fn file_reference_in_failure_message_is_not_a_command() {
    // A single-word backtick span with a file suffix is a file reference,
    // not a command: AS-EVD-001 must not fire on prose.
    assert!(lint_failure_evidence(PROSE_FILE_REFERENCE).is_empty());
}

const SUCCESS_MESSAGE_NOT_FLAGGED: &str = r#"#!/usr/bin/env bash
echo "all good (see `docker compose ps`)"
"#;

#[test]
fn success_message_is_not_a_failure_path() {
    assert!(lint_failure_evidence(SUCCESS_MESSAGE_NOT_FLAGGED).is_empty());
}

const MESSAGE_WITHOUT_REFERENCE: &str = r#"#!/usr/bin/env bash
if ! true; then
  echo "gateway failed to start" >&2
  exit 1
fi
"#;

#[test]
fn failure_message_without_reference_is_silent() {
    assert!(lint_failure_evidence(MESSAGE_WITHOUT_REFERENCE).is_empty());
}

// ---------------------------------------------------------------------------
// AS-EVD-002: trap cleanup removes a file a failure message refers to
// ---------------------------------------------------------------------------

const TRAP_REMOVES_REFERENCED_FILE: &str = r#"#!/usr/bin/env bash
set -uo pipefail
log_file="$(mktemp)"
trap 'rm -f "$log_file"' EXIT
if [ "$1" = "--fail" ]; then
  echo "run failed, log at $log_file" >&2
  exit 1
fi
"#;

#[test]
fn trap_removes_file_named_in_failure_message() {
    let findings = lint_failure_evidence(TRAP_REMOVES_REFERENCED_FILE);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.rule, EvidenceRule::TrapDestroysReferencedFile);
    assert_eq!(finding.line, 6);
    assert_eq!(finding.subject, "$log_file");
}

const TRAP_REMOVES_DIRECTORY: &str = r#"#!/usr/bin/env bash
set -uo pipefail
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
if [ "$1" = "--fail" ]; then
  echo "run failed, log at $workdir/gateway.log" >&2
  exit 1
fi
"#;

#[test]
fn trap_removes_directory_containing_referenced_file() {
    let findings = lint_failure_evidence(TRAP_REMOVES_DIRECTORY);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, EvidenceRule::TrapDestroysReferencedFile);
    assert_eq!(findings[0].subject, "$workdir/gateway.log");
}

const NAMED_HANDLER_REMOVES_REFERENCED_FILE: &str = r#"#!/usr/bin/env bash
set -uo pipefail
tmp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT
if [ "$1" = "--fail" ]; then
  echo "run failed, log at `$tmp_dir/run.log`" >&2
  exit 1
fi
"#;

#[test]
fn named_trap_handler_removing_referenced_file_is_flagged() {
    let findings = lint_failure_evidence(NAMED_HANDLER_REMOVES_REFERENCED_FILE);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, EvidenceRule::TrapDestroysReferencedFile);
    assert_eq!(findings[0].subject, "$tmp_dir/run.log");
}

const TRAP_REMOVES_OTHER_FILE: &str = r#"#!/usr/bin/env bash
set -uo pipefail
log_file="$(mktemp)"
scratch="$(mktemp)"
trap 'rm -f "$scratch"' EXIT
if [ "$1" = "--fail" ]; then
  echo "run failed, log at $log_file" >&2
  exit 1
fi
"#;

#[test]
fn trap_removing_an_unreferenced_file_is_silent() {
    assert!(lint_failure_evidence(TRAP_REMOVES_OTHER_FILE).is_empty());
}

const TRAP_ON_NON_EXIT_SIGNAL: &str = r#"#!/usr/bin/env bash
set -uo pipefail
log_file="$(mktemp)"
trap 'rm -f "$log_file"' DEBUG
if [ "$1" = "--fail" ]; then
  echo "run failed, log at $log_file" >&2
  exit 1
fi
"#;

#[test]
fn trap_on_a_non_exit_signal_does_not_destroy_evidence() {
    // Only cleanup traps (EXIT/INT/TERM/HUP) run after the failure message is
    // acted on; a DEBUG trap is a different lifecycle and is out of scope.
    assert!(lint_failure_evidence(TRAP_ON_NON_EXIT_SIGNAL).is_empty());
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

#[test]
fn trap_removal_targets_collects_inline_and_named_handlers() {
    let script = r#"
trap 'rm -f "$a" "$b"' EXIT
cleanup() {
  rm -rf "$dir"
  echo "bye"
}
trap cleanup EXIT
trap 'rm -f "$c"' DEBUG
"#;
    let targets = trap_removal_targets(script);
    assert!(targets.contains(&"$a".to_string()), "{targets:?}");
    assert!(targets.contains(&"$b".to_string()), "{targets:?}");
    assert!(targets.contains(&"$dir".to_string()), "{targets:?}");
    assert!(!targets.contains(&"$c".to_string()), "{targets:?}");
    assert!(!targets.contains(&"bye".to_string()), "{targets:?}");
}

#[test]
fn findings_are_sorted_by_line_then_subject() {
    let script = r#"
if ! true; then
  echo "b `zeta cmd` and a `alpha cmd`" >&2
  echo "a `alpha cmd`" >&2
  exit 1
fi
"#;
    let findings = lint_failure_evidence(script);
    assert_eq!(findings.len(), 3, "{findings:?}");
    assert_eq!(findings[0].line, 3);
    assert_eq!(findings[0].subject, "alpha cmd");
    assert_eq!(findings[1].line, 3);
    assert_eq!(findings[1].subject, "zeta cmd");
    assert_eq!(findings[2].line, 4);
}

#[test]
fn empty_and_comment_only_scripts_are_silent() {
    assert!(lint_failure_evidence("").is_empty());
    assert!(lint_failure_evidence("# echo \"see `some command`\" >&2\n").is_empty());
}

#[test]
fn finding_message_names_the_line_and_the_subject() {
    let findings = lint_failure_evidence(CMD_NOT_CAPTURED);
    let finding: &EvidenceFinding = &findings[0];
    assert!(finding.message.contains("line 4"), "{finding:?}");
    assert!(
        finding
            .message
            .contains("docker compose -p inferweave-gateway logs"),
        "{finding:?}"
    );
}

// ---------------------------------------------------------------------------
// AC #3: populated negative case — force the failure and confirm the
// evidence is present in the completed run's output (#3793)
// ---------------------------------------------------------------------------

/// The defective shape, populated: the failure path names a command whose
/// output was never captured and a file the EXIT trap deletes.
const DEFECTIVE_SCRIPT: &str = r#"#!/usr/bin/env bash
# Populated defective case: both AS-EVD rules fire on this script.
set -uo pipefail
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
printf 'gateway line one\ngateway line two\n' > "$workdir/gateway.log"
if [ "${FORCE_FAILURE:-0}" = "1" ]; then
  echo "gateway failed to start (see `docker compose -p inferweave-gateway logs`)" >&2
  echo "run failed, log at $workdir/gateway.log" >&2
  exit 1
fi
echo "ok"
"#;

/// The well-behaved shape: the failure path captures the evidence, prints it
/// into the run output, and does not name a file the trap will delete.
const WELL_BEHAVED_SCRIPT: &str = r#"#!/usr/bin/env bash
# Populated negative case: the evidence is printed into the run output before
# the trap cleans up, so the completed run's output carries it.
set -uo pipefail
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
printf 'gateway line one\ngateway line two\n' > "$workdir/gateway.log"
if [ "${FORCE_FAILURE:-0}" = "1" ]; then
  echo "gateway log contents:" >&2
  cat "$workdir/gateway.log" >&2
  echo "run failed; the log contents are printed above" >&2
  exit 1
fi
echo "ok"
"#;

#[cfg(unix)]
mod end_to_end {
    use super::{run_bash, TempScriptRoot, DEFECTIVE_SCRIPT, WELL_BEHAVED_SCRIPT};
    use autospec_core::lint::evidence::{lint_failure_evidence, EvidenceRule};
    use std::process::Command;

    fn bash_available() -> bool {
        Command::new("bash")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn defective_script_fires_both_rules_and_loses_its_evidence_on_disk() {
        if !bash_available() {
            eprintln!("skip: bash not available");
            return;
        }
        let root = TempScriptRoot::new();
        let script = root.write_script("defective.sh", DEFECTIVE_SCRIPT);

        let findings = lint_failure_evidence(DEFECTIVE_SCRIPT);
        let rules: Vec<_> = findings.iter().map(|f| f.rule).collect();
        assert!(
            rules.contains(&EvidenceRule::UncapturedCommandReference),
            "{findings:?}"
        );
        assert!(
            rules.contains(&EvidenceRule::TrapDestroysReferencedFile),
            "{findings:?}"
        );

        let (code, output) = run_bash(&script);
        assert_eq!(code, 1, "forced failure exits non-zero; output:\n{output}");
        // The failure messages are in the completed run's output...
        // (the backtick span is a live command substitution, so the run
        // output carries the message minus the substituted command text)
        assert!(
            output.contains("gateway failed to start (see"),
            "output:\n{output}"
        );
        assert!(output.contains("gateway.log"), "output:\n{output}");
        // ...but the evidence itself was never captured: the run output has
        // no log contents, and the trap destroyed the file on disk.
        assert!(!output.contains("gateway line one"));
    }

    #[test]
    fn well_behaved_script_is_silent_and_keeps_evidence_in_run_output() {
        if !bash_available() {
            eprintln!("skip: bash not available");
            return;
        }
        let root = TempScriptRoot::new();
        let script = root.write_script("well-behaved.sh", WELL_BEHAVED_SCRIPT);

        let findings = lint_failure_evidence(WELL_BEHAVED_SCRIPT);
        assert!(findings.is_empty(), "{findings:?}");

        let (code, output) = run_bash(&script);
        assert_eq!(code, 1, "forced failure exits non-zero; output:\n{output}");
        // The evidence is present in the completed run's output even though
        // the trap removed the workdir: the #3793 invariant.
        assert!(output.contains("gateway line one"), "output:\n{output}");
        assert!(output.contains("gateway line two"), "output:\n{output}");
    }
}
