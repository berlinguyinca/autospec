//! Pre-filter scope contract (issue #4489).
//!
//! The conversion pre-filter ran `cargo clippy -p autospec-core
//! --all-targets` while the gate ran `cargo clippy --workspace
//! --all-targets`. A patch touching `autospec-cli` passed the pre-filter
//! with `clippy=0` and carried two workspace clippy errors into the batch —
//! and the same patch also broke a test the pre-filter never ran. The
//! acceptance scenarios:
//!
//! * the pre-filter's scope is derived from the patch's touched crates, or
//!   is the full workspace — never a fixed single crate;
//! * a batch failure reports whether any member passed the pre-filter at a
//!   narrower scope than the gate, so the class is detected rather than
//!   re-encountered;
//! * the pre-filter and the gate share one definition of "the checks",
//!   differing only in which checks run, never in what they cover.

use std::collections::BTreeSet;

use autospec_core::prefilter_scope::{
    batch_failure_line, crates_touched, derive_prefilter_scope, gate_commands, has_scope_gap,
    prefilter_commands, scope_gaps, BatchMember, CheckScope, ScopeGap, GATE_CHECKS,
    PREFILTER_CHECK_NAMES,
};

fn crates(names: &[&str]) -> CheckScope {
    CheckScope::Crates(
        names
            .iter()
            .map(|name| (*name).to_string())
            .collect::<BTreeSet<_>>(),
    )
}

// -- Acceptance 1: the scope is derived from the patch's touched crates,
//    or is the full workspace — never a fixed single crate.

#[test]
fn a_patch_touching_autospec_cli_gets_autospec_cli_not_a_fixed_crate() {
    // The incident: the pre-filter ran `-p autospec-core` on a patch that
    // touched autospec-cli. The derivation must produce the crate the
    // patch touched, so a patch that touches nothing in autospec-core can
    // never be pre-filtered under autospec-core's scope.
    let scope = derive_prefilter_scope(&["crates/autospec-cli/src/main.rs"]);
    assert_eq!(scope, crates(&["autospec-cli"]));
    assert_ne!(
        scope,
        crates(&["autospec-core"]),
        "a fixed single crate is exactly what the derivation must refuse"
    );
}

#[test]
fn a_patch_touching_several_crates_gets_all_of_them() {
    let scope = derive_prefilter_scope(&[
        "crates/autospec-core/src/lib.rs",
        "crates/autospec-cli/src/main.rs",
        "crates/autospec-core/tests/foo.rs",
    ]);
    assert_eq!(scope, crates(&["autospec-cli", "autospec-core"]));
}

#[test]
fn a_patch_touching_no_crate_falls_back_to_the_workspace() {
    // No resolvable crate is fail-closed: the conservative scope is the
    // full workspace, never an invented narrow one.
    let scope = derive_prefilter_scope(&["scripts/lint-issue.sh", "docs/notes.md"]);
    assert_eq!(scope, CheckScope::Workspace);
}

#[test]
fn an_empty_patch_gets_the_workspace() {
    let empty: [&str; 0] = [];
    assert_eq!(derive_prefilter_scope(&empty), CheckScope::Workspace);
}

#[test]
fn crates_touched_reads_crates_at_any_depth_and_ignores_other_paths() {
    assert_eq!(
        crates_touched(&[
            "crates/autospec-core/src/lib.rs",
            "crates/autospec-cli/src/commands/deep/nested/mod.rs",
            "crates/autospec-cli",
            "crates/",
            "crates/../crates/autospec-core/src/x.rs",
            "mycrates/other/src/lib.rs",
            "scripts/lint-issue.sh",
        ]),
        ["autospec-cli", "autospec-core"]
            .iter()
            .map(|name| (*name).to_string())
            .collect::<BTreeSet<_>>()
    );
}

#[test]
fn crate_tokens_are_sorted_and_rendered_as_p_flags() {
    assert_eq!(
        crates(&["autospec-core", "autospec-cli"]).tokens(),
        ["-p", "autospec-cli", "-p", "autospec-core"]
    );
    assert_eq!(CheckScope::Workspace.tokens(), ["--workspace"]);
}

// -- Acceptance 2: a batch failure reports the scope gap.

#[test]
fn the_incident_batch_failure_is_attributed_to_a_scope_gap() {
    // The incident reconstruction: the gate is `--workspace`; one member
    // passed the pre-filter under the fixed `-p autospec-core` scope even
    // though its patch touched autospec-cli, and one under a narrower
    // crate scope than the gate's. Both must be named by the report.
    let gate = CheckScope::Workspace;
    let batch = [
        BatchMember {
            patch: "patch-1".to_string(),
            recorded_scope: crates(&["autospec-core"]),
        },
        BatchMember {
            patch: "patch-2".to_string(),
            recorded_scope: CheckScope::Workspace,
        },
        BatchMember {
            patch: "patch-3".to_string(),
            recorded_scope: crates(&["autospec-cli"]),
        },
    ];

    assert!(has_scope_gap(&batch, &gate));
    assert_eq!(
        scope_gaps(&batch, &gate),
        vec![
            ScopeGap {
                patch: "patch-1".to_string(),
                recorded_scope: crates(&["autospec-core"])
            },
            ScopeGap {
                patch: "patch-3".to_string(),
                recorded_scope: crates(&["autospec-cli"])
            }
        ]
    );

    let line = batch_failure_line(&batch, &gate);
    assert!(line.contains("patch-1"), "{line}");
    assert!(line.contains("patch-3"), "{line}");
    assert!(
        !line.contains("patch-2"),
        "no gap was recorded for patch-2: {line}"
    );
    assert!(
        line.contains("--workspace"),
        "the gate's scope is named: {line}"
    );
    assert!(line.contains("scope gap"), "the class is named: {line}");
}

#[test]
fn a_batch_failure_with_no_narrower_scope_rules_the_gap_out() {
    // Every member recorded the gate's own scope, so the report rules the
    // gap out explicitly instead of leaving it to be re-diagnosed.
    let gate = CheckScope::Workspace;
    let batch = [
        BatchMember {
            patch: "patch-1".to_string(),
            recorded_scope: CheckScope::Workspace,
        },
        BatchMember {
            patch: "patch-2".to_string(),
            recorded_scope: CheckScope::Workspace,
        },
    ];

    assert!(!has_scope_gap(&batch, &gate));
    assert!(scope_gaps(&batch, &gate).is_empty());
    let line = batch_failure_line(&batch, &gate);
    // The report still says which direction it looked, so a reader does
    // not have to re-diagnose the question.
    assert!(line.contains("no member passed the pre-filter"), "{line}");
    assert!(line.contains("--workspace"), "{line}");
}

#[test]
fn a_crates_scope_is_narrower_than_the_workspace_but_not_its_superset() {
    let workspace = CheckScope::Workspace;
    let one = crates(&["autospec-cli"]);
    let two = crates(&["autospec-cli", "autospec-core"]);

    assert!(one.is_narrower_than(&workspace));
    assert!(two.is_narrower_than(&workspace));
    assert!(!workspace.is_narrower_than(&workspace));
    assert!(!two.is_narrower_than(&one)); // a superset examines at least as much
    assert!(one.is_narrower_than(&two));
}

#[test]
fn a_gate_at_a_crates_scope_still_detects_narrower_members() {
    // The comparison is general: narrower-than-gate, not narrower-than-
    // workspace. A gate at two crates still flags a member admitted under
    // one of them.
    let gate = crates(&["autospec-cli", "autospec-core"]);
    let batch = [
        BatchMember {
            patch: "patch-1".to_string(),
            recorded_scope: crates(&["autospec-cli"]),
        },
        BatchMember {
            patch: "patch-2".to_string(),
            recorded_scope: crates(&["autospec-cli", "autospec-core"]),
        },
    ];
    let line = batch_failure_line(&batch, &gate);
    assert!(line.contains("patch-1"), "{line}");
    assert!(!line.contains("patch-2"), "{line}");
}

// -- Acceptance 3: pre-filter and gate share one definition of the checks.

#[test]
fn the_prefilter_commands_are_the_gate_commands_for_the_same_checks() {
    // For every check the pre-filter runs, its rendered command must be
    // byte-identical to the gate's command for that check, at every scope:
    // the two differ only in which checks run, never in what a check
    // covers.
    for scope in [
        CheckScope::Workspace,
        crates(&["autospec-cli"]),
        crates(&["autospec-cli", "autospec-core"]),
    ] {
        let gate = gate_commands(&scope);
        let prefilter = prefilter_commands(&scope);
        for command in &prefilter {
            assert!(
                gate.contains(command),
                "prefilter command {command:?} is not a gate command at {scope}: {gate:?}"
            );
        }
    }
}

#[test]
fn the_prefilter_runs_fewer_checks_than_the_gate() {
    // Speed comes from running fewer checks (clippy before tests), never
    // from narrowing a check's scope.
    let gate = gate_commands(&CheckScope::Workspace);
    let prefilter = prefilter_commands(&CheckScope::Workspace);
    assert!(
        prefilter.len() < gate.len(),
        "the pre-filter must run a strict subset of the gate's checks"
    );
    assert_eq!(prefilter.len(), PREFILTER_CHECK_NAMES.len());
    assert_eq!(gate.len(), GATE_CHECKS.len());
    // The subset is by name: every pre-filter check is a named gate check.
    for name in PREFILTER_CHECK_NAMES {
        assert!(
            GATE_CHECKS.iter().any(|check| check.name == *name),
            "{name} is in PREFILTER_CHECK_NAMES but not in GATE_CHECKS"
        );
    }
}

#[test]
fn the_gate_commands_match_the_invariant() {
    // The gate of the incident: `cargo clippy --workspace --all-targets`
    // (plus the test stage). The shared definition renders exactly that.
    let gate = gate_commands(&CheckScope::Workspace);
    let workspace_clippy: Vec<String> = ["cargo", "clippy", "--workspace", "--all-targets"]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect();
    assert!(
        gate.contains(&workspace_clippy),
        "gate must run the workspace clippy: {gate:?}"
    );
}
