//! Integration tests for `autospec_core::self_gate` (issue #4263).
//!
//! One fixture test per rule, plus regression tests that reconstruct the
//! incident: the queue-order reaper that starved the two oldest offenders
//! for three sweeps, the selector keyed on the issue, and the denominator
//! that was 15 when there were 11 issues and 14 patches.

use autospec_core::self_gate::{
    gate_destructive_run, select_reaps, trust_verdict, AttemptLedger, AutomationArtifact,
    Condition, GateVerdict, ReapCandidate, ReliabilityEvidence, RunMode, SelectionReport,
    WorkState,
};

const MAX_KILLS: usize = 5;

// ── The incident fixture: two persistent worst offenders, cap 5 ────────────

/// The two persistent worst offenders of the incident: 4h01m and 3h51m
/// past a 3h limit — 3660 s and 3060 s over. They sit at the tail of the
/// reaper's list order.
fn worst_offenders() -> Vec<ReapCandidate> {
    vec![
        ReapCandidate {
            id: "agent-a".into(),
            over_by_secs: 3660, // 4h01m past the 3h limit
        },
        ReapCandidate {
            id: "agent-b".into(),
            over_by_secs: 3060, // 3h51m past the 3h limit
        },
    ]
}

/// The younger offenders at the head of the list order on sweep `sweep`.
///
/// Killed agents are re-dispatched, so each sweep the list starts with a
/// fresh batch of younger offenders ahead of the two persistent worst.
fn younger_offenders(sweep: usize) -> Vec<ReapCandidate> {
    [1200, 900, 600, 450, 300]
        .into_iter()
        .enumerate()
        .map(|(i, secs)| ReapCandidate {
            id: format!("agent-s{sweep}-y{}", i + 1),
            over_by_secs: secs,
        })
        .collect()
}

/// The legacy reaper: walk the list in order, kill up to the cap.
fn legacy_queue_order_reaper(
    candidates: &mut Vec<ReapCandidate>,
    cap: usize,
) -> Vec<ReapCandidate> {
    let take = cap.min(candidates.len());
    let reaped: Vec<ReapCandidate> = candidates.drain(..take).collect();
    reaped
}

// ── Defect 1 / rule 4: the reaper ──────────────────────────────────────────

#[test]
fn list_order_starves_the_worst_offenders() {
    // Reconstructs the incident: three sweeps, cap 5, list order. Each
    // sweep the list starts with a fresh batch of five younger offenders
    // (killed agents are re-dispatched), ahead of the two persistent worst
    // offenders at the tail. The cap is consumed on the younger batch
    // every sweep; the two worst survive all three sweeps while younger
    // offenders are killed in each.
    let mut worst_alive = vec!["agent-a", "agent-b"];
    let mut younger_killed_per_sweep = Vec::new();

    for sweep in 1..=3 {
        let mut queue = younger_offenders(sweep);
        queue.extend(worst_offenders());
        let reaped = legacy_queue_order_reaper(&mut queue, MAX_KILLS);
        younger_killed_per_sweep.push(reaped.len());
        worst_alive.retain(|w| !reaped.iter().any(|r| &r.id == w));
    }

    // The cap filled on younger offenders in every sweep.
    assert_eq!(younger_killed_per_sweep, vec![5, 5, 5]);
    // The two worst offenders survived all three sweeps.
    assert_eq!(worst_alive, vec!["agent-a", "agent-b"]);
}

#[test]
fn severity_order_selects_the_worst_first() {
    // One sweep of the incident's queue, selected by severity: the two
    // worst offenders are selected first, before any younger offender can
    // consume the cap.
    let mut queue = younger_offenders(1);
    queue.extend(worst_offenders());
    let plan = select_reaps(&queue, MAX_KILLS).expect("cap 5 is a valid cap");

    let ids = plan.selected_ids();
    assert_eq!(ids[0], "agent-a");
    assert_eq!(ids[1], "agent-b");
    assert_eq!(ids.len(), 5);
    // The two youngest offenders are deferred, not silently skipped.
    assert_eq!(plan.deferred.len(), 2);
    assert!(plan.line().contains("deferred by the cap"));
}

#[test]
fn deferred_offenders_are_named_and_the_denominator_reconciles() {
    let mut queue = younger_offenders(1);
    queue.extend(worst_offenders());
    let plan = select_reaps(&queue, 2).expect("cap 2 is a valid cap");

    // "reaping 2 of 7" — the denominator is the whole offender population.
    let line = plan.line();
    assert!(line.starts_with("reaping 2 of 7 offenders"), "got: {line}");
    // Every deferred offender is named in the line.
    for deferred in &plan.deferred {
        assert!(
            line.contains(&deferred.id),
            "missing {id} in: {line}",
            id = deferred.id
        );
    }
    // Selected + deferred reconciles to the population.
    assert_eq!(plan.selected.len() + plan.deferred.len(), queue.len());
}

#[test]
fn a_zero_cap_is_refused_not_a_silent_pass() {
    let queue = worst_offenders();
    let err = select_reaps(&queue, 0).expect_err("cap 0 must be refused");
    assert!(err.contains("reap cap is zero"), "got: {err}");
}

#[test]
fn a_cap_larger_than_the_population_reaps_everyone_without_deferrals() {
    let mut queue = younger_offenders(1);
    queue.extend(worst_offenders());
    let plan = select_reaps(&queue, 50).expect("cap 50 is a valid cap");
    assert_eq!(plan.selected.len(), 7);
    assert!(plan.deferred.is_empty());
    assert_eq!(plan.line(), "reaping 7 of 7 offenders");
}

// ── Defect 2: the attempt ledger is keyed by patch ─────────────────────────

#[test]
fn a_new_patch_for_an_attempted_issue_is_a_candidate() {
    let mut ledger = AttemptLedger::new();
    // Issue #4263's first patch was attempted (and failed).
    ledger.record_attempt("issue-4263-patch-1");

    // The second patch for the same issue is a fresh, unattempted patch.
    assert!(!ledger.is_attempted("issue-4263-patch-2"));
    // The first patch is still attempted.
    assert!(ledger.is_attempted("issue-4263-patch-1"));
    assert_eq!(ledger.attempted_count(), 1);
}

#[test]
fn the_selector_line_reports_the_patch_denominator() {
    let mut ledger = AttemptLedger::new();
    let all_patches: &[&str] = &[
        "issue-101-patch-1",
        "issue-101-patch-2",
        "issue-102-patch-1",
        "issue-103-patch-1",
        "issue-104-patch-1",
    ];
    ledger.record_attempt("issue-101-patch-1");
    ledger.record_attempt("issue-103-patch-1");

    // The denominator is patches (5), the unit inspected; the issue count
    // (4) rides alongside, never in place of it.
    assert_eq!(
        ledger.selector_line(all_patches, 4),
        "attempted 2 of 5 patches (4 issues)"
    );
}

// ── Defect 3: a conversion candidate is a patch, not a directory ───────────

#[test]
fn a_directory_is_not_a_conversion_candidate() {
    // The agent started; the directory exists; there is no output.
    let in_progress = WorkState::InProgress {
        directory: "out/issue-4263".into(),
    };
    assert_eq!(in_progress.directory(), "out/issue-4263");
    assert_eq!(in_progress.patch(), None);
    assert!(!in_progress.is_conversion_candidate());
}

#[test]
fn only_has_patch_is_a_conversion_candidate() {
    let has_patch = WorkState::HasPatch {
        directory: "out/issue-101".into(),
        patch: "out/issue-101/patch-2.diff".into(),
    };
    assert!(has_patch.is_conversion_candidate());
    assert_eq!(has_patch.patch(), Some("out/issue-101/patch-2.diff"));

    let converted = WorkState::Converted {
        directory: "out/issue-102".into(),
        patch: "out/issue-102/patch-1.diff".into(),
    };
    // Converted output has a patch but is already done: not a candidate.
    assert!(!converted.is_conversion_candidate());
}

#[test]
fn the_candidate_count_counts_patches_not_directories() {
    // The incident's population: 11 issues, 14 patches, 15 directories —
    // restarted agents left extra directories behind. Modelled as work
    // states, one per directory.
    let work: Vec<WorkState> = vec![
        WorkState::HasPatch {
            directory: "out/issue-101".into(),
            patch: "p-1".into(),
        },
        // issue-101 restarted; the new directory has no output yet.
        WorkState::InProgress {
            directory: "out/issue-101-restart".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-102".into(),
            patch: "p-2".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-103".into(),
            patch: "p-3".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-104".into(),
            patch: "p-4".into(),
        },
        WorkState::Converted {
            directory: "out/issue-105".into(),
            patch: "p-5".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-106".into(),
            patch: "p-6".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-107".into(),
            patch: "p-7".into(),
        },
        WorkState::Converted {
            directory: "out/issue-107-restart".into(),
            patch: "p-8".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-108".into(),
            patch: "p-9".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-109".into(),
            patch: "p-10".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-109-restart".into(),
            patch: "p-11".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-110".into(),
            patch: "p-12".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-111".into(),
            patch: "p-13".into(),
        },
        WorkState::HasPatch {
            directory: "out/issue-111-restart".into(),
            patch: "p-14".into(),
        },
    ];

    let directories = work.len();
    let patches = work.iter().filter(|w| w.patch().is_some()).count();
    let issues = work
        .iter()
        .map(|w| w.directory().strip_prefix("out/issue-").unwrap_or("out/"))
        .map(|s| s.split('-').next().unwrap_or(s).to_string())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let candidates = work.iter().filter(|w| w.is_conversion_candidate()).count();

    assert_eq!(issues, 11); // the unit nobody counted
    assert_eq!(directories, 15); // the unit the broken selector counted
    assert_eq!(patches, 14); // the unit it should have counted
    assert_eq!(candidates, 12); // the numerator, out of the patches

    let report = SelectionReport {
        numerator: candidates,
        denominator: patches,
        written_to_file: true,
    };
    assert_eq!(report.line(), "12 of 14 selected");
    assert!(report.findings().is_empty());
}

// ── Rule 1: the dry-run gate ───────────────────────────────────────────────

#[test]
fn a_destructive_script_without_a_dry_run_mode_is_refused() {
    let verdict = gate_destructive_run(false, RunMode::Production, false);
    assert!(matches!(verdict, GateVerdict::Refused { .. }));
    if let GateVerdict::Refused { reason } = verdict {
        // The refusal names the missing piece.
        assert!(reason.contains("--dry-run"), "got: {reason}");
    }
}

#[test]
fn the_first_production_run_requires_a_dry_run_first() {
    let verdict = gate_destructive_run(true, RunMode::Production, false);
    assert!(matches!(verdict, GateVerdict::Refused { .. }));
    if let GateVerdict::Refused { reason } = verdict {
        assert!(reason.contains("dry run never performed"), "got: {reason}");
    }
}

#[test]
fn a_dry_run_is_never_refused_for_being_first() {
    // Even a script that has never been dry-run before may run its dry
    // run: the dry run is the gate, not something the gate gates.
    let verdict = gate_destructive_run(true, RunMode::DryRun, false);
    assert_eq!(verdict, GateVerdict::Proceed);
}

#[test]
fn production_proceeds_after_a_reviewed_dry_run() {
    let verdict = gate_destructive_run(true, RunMode::Production, true);
    assert_eq!(verdict, GateVerdict::Proceed);
}

// ── Rule 2: denominator and reviewable-as-file ─────────────────────────────

#[test]
fn a_report_always_carries_the_denominator() {
    let report = SelectionReport {
        numerator: 3,
        denominator: 14,
        written_to_file: true,
    };
    assert_eq!(report.line(), "3 of 14 selected");
    assert!(report.findings().is_empty());
}

#[test]
fn an_ephemeral_report_is_a_finding() {
    // Computed in a one-line jq, printed to a dead terminal: the
    // denominator is not reviewable after the run.
    let report = SelectionReport {
        numerator: 3,
        denominator: 15,
        written_to_file: false,
    };
    let findings = report.findings();
    assert!(
        findings
            .iter()
            .any(|f| f.contains("never written to a file")),
        "got: {findings:?}"
    );
}

#[test]
fn a_numerator_above_the_denominator_is_a_counting_bug() {
    let report = SelectionReport {
        numerator: 15,
        denominator: 14,
        written_to_file: true,
    };
    let findings = report.findings();
    assert!(
        findings
            .iter()
            .any(|f| f.contains("counting the wrong unit")),
        "got: {findings:?}"
    );
}

// ── Rule 3: automation that decides work is itself work ────────────────────

#[test]
fn an_inline_predicate_has_no_home() {
    // The one-line jq in a wrapper script: no file, no comments, no test.
    let artifact = AutomationArtifact {
        file: None,
        conditions: vec![Condition {
            name: "patch exists".into(),
            bug_comment: Some("directories exist before output does".into()),
        }],
        fixture_test: false,
    };
    let findings = artifact.findings();
    assert!(
        findings.iter().any(|f| f.contains("lives inline")),
        "got: {findings:?}"
    );
    assert!(
        findings.iter().any(|f| f.contains("no fixture test")),
        "got: {findings:?}"
    );
}

#[test]
fn an_undocumented_condition_is_a_finding() {
    let artifact = AutomationArtifact {
        file: Some("scripts/select-conversions.sh".into()),
        conditions: vec![
            Condition {
                name: "patch exists".into(),
                bug_comment: Some("directories exist before output does".into()),
            },
            Condition {
                // A filter whose absence nobody would notice: no comment
                // naming the bug it prevents.
                name: "not attempted".into(),
                bug_comment: None,
            },
        ],
        fixture_test: true,
    };
    let findings = artifact.findings();
    assert_eq!(findings.len(), 1, "got: {findings:?}");
    assert!(findings[0].contains("not attempted"), "got: {findings:?}");
    assert!(
        findings[0].contains("no comment naming the bug"),
        "got: {findings:?}"
    );
}

#[test]
fn a_fully_gated_artifact_has_no_findings() {
    let artifact = AutomationArtifact {
        file: Some("scripts/select-conversions.sh".into()),
        conditions: vec![
            Condition {
                name: "patch exists".into(),
                bug_comment: Some("directories exist before output does".into()),
            },
            Condition {
                name: "not attempted".into(),
                bug_comment: Some("an attempt is a fact about the patch, not the issue".into()),
            },
        ],
        fixture_test: true,
    };
    assert!(artifact.findings().is_empty());
}

// ── Rule 5: recency is not reliability ─────────────────────────────────────

#[test]
fn recent_success_is_not_trust_evidence() {
    // "I wrote this helper an hour ago and it worked once."
    assert!(!trust_verdict(ReliabilityEvidence::RanRecently {
        times: 1
    }));
    // A thousand happy-path runs are the same evidence at a larger number.
    assert!(!trust_verdict(ReliabilityEvidence::RanRecently {
        times: 1000
    }));
}

#[test]
fn the_gates_are_the_trust_inputs() {
    assert!(trust_verdict(ReliabilityEvidence::Gated));
}
