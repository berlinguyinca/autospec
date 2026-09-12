//! The fleet's own merge throughput invalidates the fleet's in-flight
//! patches (issue #4151).
//!
//! The regression tests run in the configuration the incident required:
//! 53 of 141 held patches (38%) died on merge conflicts, and every one of
//! the 90 conflicting-file mentions was in a file `main` had changed in
//! the same 24-hour window that carried 160 commits. The three other
//! explanations — stale patches, a missing 3-way ancestor, manifest-file
//! serialisation — were measured out first; the tests here pin the fourth
//! and the four mitigations it demands.

use autospec_core::held_conflicts::{
    contention_order, declaration_manifest_findings, dispatch_batches, held_conflicts_line,
    is_declaration_manifest, overlap_report, DeclaredIssue, InFlightPatch, TrunkCommit,
    TrunkMovement,
};

const LIB: &str = "crates/autospec-core/src/lib.rs";
const EXEC_MOD: &str = "crates/autospec-core/src/execution/mod.rs";
const CLI_REF: &str = "docs/cli-reference.md";
const PIPE: &str = "crates/autospec-core/src/execution/patch_pipeline.rs";
const EVAL_MOD: &str = "crates/autospec-core/src/evaluation/mod.rs";

/// The incident window: 160 commits to `main` in 24 h touching 502 files,
/// with the per-file churn the incident measured.
fn incident_trunk() -> TrunkMovement {
    let hot: [(&str, u32); 5] = [
        (LIB, 25),
        (EXEC_MOD, 12),
        (CLI_REF, 11),
        (PIPE, 8),
        (EVAL_MOD, 6),
    ];
    let mut commits = Vec::new();
    let mut n = 0u32;
    for (file, count) in hot {
        for _ in 0..count {
            commits.push(TrunkCommit {
                sha: format!("h{n}"),
                files: vec![file.to_string()],
            });
            n += 1;
        }
    }
    // The other 98 commits touched files none of the incident's patches
    // did, one each: 62 + 98 = 160 commits.
    for _ in 0..98 {
        commits.push(TrunkCommit {
            sha: format!("c{n}"),
            files: vec![format!("cold/{n}.rs")],
        });
        n += 1;
    }
    assert_eq!(commits.len(), 160);
    TrunkMovement { commits }
}

/// The five incident patches, in arrival order (not churn order).
fn incident_patches() -> Vec<InFlightPatch> {
    vec![
        InFlightPatch::new(4101, "base", vec![PIPE.to_string()]).unwrap(),
        InFlightPatch::new(4102, "base", vec![CLI_REF.to_string()]).unwrap(),
        InFlightPatch::new(4103, "base", vec![EXEC_MOD.to_string()]).unwrap(),
        InFlightPatch::new(4104, "base", vec![LIB.to_string()]).unwrap(),
        InFlightPatch::new(4105, "base", vec![EVAL_MOD.to_string()]).unwrap(),
    ]
}

#[test]
fn a_patch_without_a_recorded_base_cannot_join_the_order() {
    // Invariant 1: the base sha is the field the rest of the pipeline is
    // computed from. A patch that names none is refused, never defaulted.
    assert_eq!(InFlightPatch::new(4104, "", vec![LIB.to_string()]), None);
    assert_eq!(InFlightPatch::new(4104, "   ", vec![LIB.to_string()]), None);
}

#[test]
fn the_incident_patches_convert_in_churn_order_not_arrival_order() {
    // Invariant 3: the patch racing `lib.rs` (25 rewrites) converts first,
    // not the one that arrived first.
    let trunk = incident_trunk();
    let plan = contention_order(&incident_patches(), &trunk);
    let issues: Vec<u32> = plan.slots.iter().map(|s| s.issue).collect();
    let contention: Vec<u32> = plan.slots.iter().map(|s| s.contention).collect();
    assert_eq!(issues, vec![4104, 4103, 4102, 4101, 4105]);
    assert_eq!(contention, vec![25, 12, 11, 8, 6]);
    assert_eq!(plan.slots[0].hot_file.as_deref(), Some(LIB));
}

#[test]
fn the_contention_line_names_every_driver_so_the_order_is_falsifiable() {
    let line = contention_order(&incident_patches(), &incident_trunk()).line();
    assert!(line.starts_with("contention order: #4104 (25 commits to main since base, "));
    assert!(
        line.contains(
            "#4105 (6 commits to main since base, crates/autospec-core/src/evaluation/mod.rs)"
        ),
        "{line}"
    );
}

#[test]
fn issues_racing_the_same_hot_surface_are_serialised() {
    // Invariant 2: three issues that all declare `execution/mod.rs` plus
    // one disjoint issue — at most one of the three can convert.
    let issues = vec![
        DeclaredIssue {
            issue: 4103,
            paths: vec![
                EXEC_MOD.to_string(),
                "crates/autospec-core/src/execution/a.rs".to_string(),
            ],
        },
        DeclaredIssue {
            issue: 4106,
            paths: vec![
                EXEC_MOD.to_string(),
                "crates/autospec-core/src/execution/b.rs".to_string(),
            ],
        },
        DeclaredIssue {
            issue: 4107,
            paths: vec![EXEC_MOD.to_string()],
        },
        DeclaredIssue {
            issue: 4108,
            paths: vec!["crates/autospec-core/src/evaluation/c.rs".to_string()],
        },
    ];
    let plan = dispatch_batches(&issues);
    assert_eq!(plan.batches.len(), 2);
    assert_eq!(plan.batches[0].issues, vec![4103, 4106, 4107]);
    assert_eq!(plan.batches[0].shared_paths, vec![EXEC_MOD.to_string()]);
    assert_eq!(plan.batches[1].issues, vec![4108]);
    assert_eq!(plan.line(), "dispatch: 3 serialized (shared surface: crates/autospec-core/src/execution/mod.rs), 1 fanned out of 4");
}

#[test]
fn the_incident_lib_rs_shape_is_a_pure_declaration_manifest() {
    // Invariant 4: the incident `lib.rs` — 101 non-comment lines, 60 of
    // them `mod` declarations, the rest comments and blanks — is the shape
    // for which a union merge is correct.
    let mut lines = Vec::new();
    lines.push("// Top-level modules.".to_string());
    for i in 0..60 {
        if i % 10 == 9 {
            lines.push(String::new());
        }
        lines.push(format!("pub mod module_{i};"));
    }
    while lines.len() < 101 {
        lines.push(String::new());
    }
    let content = lines.join("\n") + "\n";
    assert!(
        is_declaration_manifest(&content),
        "{:?}",
        declaration_manifest_findings(&content)
    );

    // The guard: the moment the file holds anything but declarations,
    // comments, and blanks, union is refused and the line is named.
    let with_body = format!("{content}fn helper() {{}}\n");
    let findings = declaration_manifest_findings(&with_body);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].contains("fn helper()"), "{:?}", findings[0]);
}

#[test]
fn a_held_line_for_the_incident_names_the_real_cause() {
    // Invariant 5: the bare line invites a person to study a patch that
    // was simply overtaken.
    assert_eq!(
        held_conflicts_line(LIB, Some(25), "24h"),
        "HELD: conflicts in crates/autospec-core/src/lib.rs (main changed crates/autospec-core/src/lib.rs 25 times in 24h)"
    );
    // A conflict main cannot explain is a different line: the patch is
    // the suspect, and a missing measurement is a third.
    assert_eq!(
        held_conflicts_line("quiet.rs", Some(0), "24h"),
        "HELD: conflicts in quiet.rs (main did not touch it in 24h; the patch is the suspect)"
    );
    assert_eq!(
        held_conflicts_line("quiet.rs", None, "24h"),
        "HELD: conflicts in quiet.rs (churn not recorded)"
    );
}

#[test]
fn every_incident_conflict_was_in_a_file_main_changed() {
    // The measurement that confirmed hypothesis 4 at 100%: 90
    // conflicting-file mentions, 502 files main touched, all 90 mentions
    // in a touched file.
    let mut mentions = Vec::new();
    for (file, count) in [
        (LIB, 5u32),
        (EXEC_MOD, 5),
        (CLI_REF, 4),
        (PIPE, 5),
        (EVAL_MOD, 7),
    ] {
        for _ in 0..count {
            mentions.push(file.to_string());
        }
    }
    // The remaining 64 mentions were in files main touched that the
    // incident's table did not rank.
    for i in 0..64 {
        mentions.push(format!("other/{i}.rs"));
    }
    assert_eq!(mentions.len(), 90);

    let mut touched: Vec<String> = vec![
        LIB.to_string(),
        EXEC_MOD.to_string(),
        CLI_REF.to_string(),
        PIPE.to_string(),
        EVAL_MOD.to_string(),
    ];
    for i in 0..497 {
        touched.push(format!("other/{i}.rs"));
    }
    assert_eq!(touched.len(), 502);

    let report = overlap_report(&mentions, &touched);
    assert_eq!(report.mentions, 90);
    assert_eq!(report.attributed, 90);
    assert!(report.unexplained.is_empty());
    assert_eq!(
        report.line(),
        "90 of 90 conflicting-file mentions in files main changed (100%)"
    );
}

#[test]
fn a_conflict_main_cannot_explain_is_named_not_counted_into_silence() {
    let mentions = vec![LIB.to_string(), "untouched.rs".to_string()];
    let report = overlap_report(&mentions, &[LIB.to_string()]);
    assert_eq!(report.attributed, 1);
    assert_eq!(report.unexplained, vec!["untouched.rs".to_string()]);
    assert_eq!(
        report.line(),
        "1 of 2 conflicting-file mentions in files main changed (50%); unexplained: untouched.rs"
    );
}
