//! Work selection is part of the system, not the invocation (issue #4257).
//!
//! The regression tests reconstruct the configuration the bug required: a
//! conversion loop whose selection predicate existed nowhere but in the
//! runner's shell history, reconstructed differently on every pass. On a
//! selector that is written down, every defect of 2026-09-07 is a checkable
//! invariant: the scope names one project, the in-flight/finished
//! distinction is a named exclusion with a removal count, and a zero is
//! reported with its denominator.

use autospec_core::work_selection::{SelectionReport, SelectionSpec, SelectionVerdict, SpecError};

/// One agent work-item as the conversion pass sees it: an issue directory
/// under some project's `out/`, with the state that decides whether it is
/// work to convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AgentPatch {
    project: &'static str,
    issue: u32,
    /// A NON-EMPTY `changes.patch` exists: the agent actually finished.
    patch_nonempty: bool,
    /// The issue has an open or merged PR.
    has_pr: bool,
    /// The issue is closed.
    issue_closed: bool,
    /// The issue was already attempted.
    attempted: bool,
}

/// The predicate of the conversion selector, written down. Every condition
/// is one of the bugs that was actually shipped; the scope is the one that
/// would have opened InferWeave patches as autospec pull requests.
fn conversion_spec() -> SelectionSpec<AgentPatch> {
    SelectionSpec::new(
        "scoped to ONE project's out/",
        "issue numbers collide across projects",
    )
    .unwrap()
    .exclude(
        "finished_patches",
        "a NON-EMPTY changes.patch exists",
        "the agent actually finished",
        |p: &AgentPatch| !p.patch_nonempty,
    )
    .unwrap()
    .exclude(
        "have_pr",
        "no open or merged PR",
        "CLOSED is an abandoned attempt, not a conversion",
        |p| p.has_pr,
    )
    .unwrap()
    .exclude(
        "closed_issue",
        "the issue is still OPEN",
        "a patch for a closed issue is moot",
        |p| p.issue_closed,
    )
    .unwrap()
    .exclude(
        "attempted",
        "not already attempted",
        "unless --retry-held",
        |p| p.attempted,
    )
    .unwrap()
}

// --- Rule 1: the predicate is an artifact, not an invocation -------------

#[test]
fn the_comment_block_enumerates_every_condition_with_its_justification() {
    let spec = conversion_spec();
    let block = spec.render();

    // One numbered line per condition: the scope first, then the four
    // exclusions, each as `# <n>. <condition> ... (<justification>)`.
    let lines: Vec<&str> = block.lines().collect();
    assert_eq!(lines.len(), 5, "block:\n{block}");
    let expected: [(&str, &str); 5] = [
        (
            "scoped to ONE project's out/",
            "issue numbers collide across projects",
        ),
        (
            "a NON-EMPTY changes.patch exists",
            "the agent actually finished",
        ),
        (
            "no open or merged PR",
            "CLOSED is an abandoned attempt, not a conversion",
        ),
        (
            "the issue is still OPEN",
            "a patch for a closed issue is moot",
        ),
        ("not already attempted", "unless --retry-held"),
    ];
    for (i, line) in lines.iter().enumerate() {
        let (condition, justification) = expected[i];
        assert!(
            line.starts_with(&format!("# {:>3}. ", i + 1)),
            "line {i} not numbered: {line:?}"
        );
        assert!(
            line.contains(condition),
            "line {i} missing condition: {line:?}"
        );
        assert!(
            line.ends_with(&format!("({justification})")),
            "line {i} missing justification: {line:?}"
        );
    }
}

#[test]
fn the_spec_names_the_scope_and_its_exclusions() {
    let spec = conversion_spec();
    assert_eq!(
        spec.scope(),
        (
            "scoped to ONE project's out/",
            "issue numbers collide across projects"
        )
    );
    assert_eq!(
        spec.exclusion_names(),
        vec!["finished_patches", "have_pr", "closed_issue", "attempted"]
    );
}

#[test]
fn a_condition_without_a_written_justification_is_not_an_artifact() {
    let err = SelectionSpec::new("scoped to one project's out/", "issue numbers collide")
        .unwrap()
        .exclude(
            "finished_patches",
            "a NON-EMPTY changes.patch exists",
            "",
            |p: &AgentPatch| {
                let _ = p;
                false
            },
        )
        .unwrap_err();
    assert_eq!(err, SpecError::EmptyJustification);
}

#[test]
fn a_condition_without_a_name_cannot_be_reported() {
    let err = SelectionSpec::new("scoped to one project's out/", "issue numbers collide")
        .unwrap()
        .exclude(
            "   ",
            "a NON-EMPTY changes.patch exists",
            "the agent actually finished",
            |p: &AgentPatch| {
                let _ = p;
                false
            },
        )
        .unwrap_err();
    assert_eq!(err, SpecError::EmptyName);
}

#[test]
fn an_unnamed_scope_is_refused() {
    assert_eq!(
        SelectionSpec::<AgentPatch>::new("", "issue numbers collide").unwrap_err(),
        SpecError::EmptyName
    );
    assert_eq!(
        SelectionSpec::<AgentPatch>::new("scoped to one project's out/", "").unwrap_err(),
        SpecError::EmptyJustification
    );
}

#[test]
fn duplicate_exclusion_names_make_the_report_ambiguous() {
    let err = conversion_spec()
        .exclude(
            "finished_patches",
            "a second condition wearing the same label",
            "it must not be allowed",
            |p: &AgentPatch| {
                let _ = p;
                false
            },
        )
        .unwrap_err();
    assert_eq!(
        err,
        SpecError::DuplicateName("finished_patches".to_string())
    );
}

#[test]
fn a_report_line_label_cannot_be_an_exclusion_name() {
    let err = SelectionSpec::new("scoped to one project's out/", "issue numbers collide")
        .unwrap()
        .exclude(
            "candidates",
            "some condition",
            "it would collide with the report line",
            |p: &AgentPatch| {
                let _ = p;
                false
            },
        )
        .unwrap_err();
    assert_eq!(err, SpecError::ReservedName("candidates".to_string()));
}

// --- Rule 2: the selector reports its denominator -------------------------

#[test]
fn the_line_reproduces_the_issue_verbatim() {
    // 423 items, every one still in flight (no finished patch): the
    // finished_patches exclusion removes all of them. Of the 423, 314 have
    // a PR, 265 are closed issues, and 232 were already attempted — the
    // exact numbers the 2026-09-07 pass should have printed.
    let items: Vec<AgentPatch> = (0..423)
        .map(|i| AgentPatch {
            project: "autospec",
            issue: i as u32,
            patch_nonempty: false,
            has_pr: i < 314,
            issue_closed: i >= 109 && i < 374,
            attempted: i >= 191,
        })
        .collect();

    let report = conversion_spec().select(&items);
    assert_eq!(
        report.line(),
        "considered=423 finished_patches=423 have_pr=314 closed_issue=265 attempted=232 -> candidates=0"
    );
    assert!(report.reconciles());
}

#[test]
fn every_exclusion_reports_how_many_items_it_removed() {
    let items = vec![
        AgentPatch {
            project: "autospec",
            issue: 1,
            patch_nonempty: false, // in flight
            has_pr: true,          // and has a PR: removed by two conditions
            issue_closed: false,
            attempted: false,
        },
        AgentPatch {
            project: "autospec",
            issue: 2,
            patch_nonempty: true,
            has_pr: false,
            issue_closed: true, // closed: removed
            attempted: false,
        },
        AgentPatch {
            project: "autospec",
            issue: 3,
            patch_nonempty: true,
            has_pr: false,
            issue_closed: false,
            attempted: true, // already attempted: removed
        },
        AgentPatch {
            project: "autospec",
            issue: 4,
            patch_nonempty: true,
            has_pr: false,
            issue_closed: false,
            attempted: false, // the only candidate
        },
    ];

    let report = conversion_spec().select(&items);
    assert_eq!(report.considered(), 4);
    assert_eq!(report.removed_by("finished_patches"), Some(1));
    assert_eq!(report.removed_by("have_pr"), Some(1));
    assert_eq!(report.removed_by("closed_issue"), Some(1));
    assert_eq!(report.removed_by("attempted"), Some(1));
    // The double-removal counts once under each condition but once excluded.
    assert_eq!(report.excluded(), 3);
    assert_eq!(report.candidates(), 1);
    assert!(report.reconciles());
    assert_eq!(report.verdict(), SelectionVerdict::Work);
}

#[test]
fn an_item_removed_by_two_conditions_counts_under_each() {
    let items = vec![AgentPatch {
        project: "autospec",
        issue: 9,
        patch_nonempty: false,
        has_pr: true,
        issue_closed: true,
        attempted: true,
    }];
    let report = conversion_spec().select(&items);
    assert_eq!(report.removed_by("finished_patches"), Some(1));
    assert_eq!(report.removed_by("have_pr"), Some(1));
    assert_eq!(report.removed_by("closed_issue"), Some(1));
    assert_eq!(report.removed_by("attempted"), Some(1));
    assert_eq!(report.excluded(), 1);
    assert_eq!(report.candidates(), 0);
    assert!(report.reconciles());
}

#[test]
fn a_missing_exclusion_has_no_count() {
    let report = conversion_spec().select(&[]);
    assert_eq!(report.removed_by("not_a_condition"), None);
}

#[test]
fn the_line_always_carries_the_denominator() {
    // `candidates=0` alone is unfalsifiable; the line format makes the
    // denominator structural, so no rendering path can drop it.
    let report = conversion_spec().select(&[]);
    assert!(report.line().starts_with("considered=0 "));
    assert!(report.line().ends_with("-> candidates=0"));
}

// --- Rule 3: a zero with and without a denominator are different statements

#[test]
fn a_zero_with_the_denominator_is_evidence_the_backlog_is_drained() {
    let items = vec![AgentPatch {
        project: "autospec",
        issue: 14,
        patch_nonempty: true,
        has_pr: false,
        issue_closed: false,
        attempted: true, // the only reason it is not a candidate
    }];
    let report = conversion_spec().select(&items);
    assert_eq!(report.candidates(), 0);
    assert_eq!(report.verdict(), SelectionVerdict::Drained);
    assert_eq!(
        report.statement(),
        "evidence the backlog is drained (1 considered, every one removed by a named condition)"
    );
}

#[test]
fn a_zero_without_inputs_is_not_drained() {
    // Defect 3 of #4257: the backlog was reported "drained" at 2 when it
    // held 78, because the filter was from a previous run and the zero was
    // reported without saying what it was a zero of. An empty scope is not
    // a drained backlog.
    let report: SelectionReport = conversion_spec().select(&[]);
    assert_eq!(report.considered(), 0);
    assert_eq!(report.candidates(), 0);
    assert_eq!(report.verdict(), SelectionVerdict::NoInputs);
    assert_eq!(
        report.statement(),
        "the query never ran or the scope matched nothing — not 'drained'"
    );
}

#[test]
fn non_zero_is_work_not_drain_either_way() {
    let items = vec![AgentPatch {
        project: "autospec",
        issue: 4,
        patch_nonempty: true,
        has_pr: false,
        issue_closed: false,
        attempted: false,
    }];
    let report = conversion_spec().select(&items);
    assert_eq!(report.candidates(), 1);
    assert_eq!(report.verdict(), SelectionVerdict::Work);
    assert_eq!(report.statement(), "1 candidate(s) remain");
}

// --- The shipped defects, instantiated -------------------------------------

#[test]
fn issue_numbers_collide_across_projects() {
    // Defect 1: the glob spanned four projects. `issue-14` is InferWeave's
    // finished patch and `issue-1` is the autospec dispatcher's; converting
    // by bare number opens the wrong project's patch. The scope names one
    // project, so the selector only ever considers that project's items.
    let all = vec![
        AgentPatch {
            project: "inferweave",
            issue: 14,
            patch_nonempty: true,
            has_pr: false,
            issue_closed: false,
            attempted: false,
        },
        AgentPatch {
            project: "autospec",
            issue: 1,
            patch_nonempty: true,
            has_pr: false,
            issue_closed: false,
            attempted: false,
        },
        AgentPatch {
            project: "inferweave",
            issue: 1,
            patch_nonempty: true,
            has_pr: false,
            issue_closed: false,
            attempted: false,
        },
        AgentPatch {
            project: "autospec",
            issue: 14,
            patch_nonempty: true,
            has_pr: false,
            issue_closed: false,
            attempted: false,
        },
    ];

    // The caller honors the scope the spec names: one project's out/.
    let scoped: Vec<AgentPatch> = all
        .iter()
        .filter(|p| p.project == "autospec")
        .copied()
        .collect();
    let report = conversion_spec().select(&scoped);
    assert_eq!(report.considered(), 2);
    assert_eq!(report.candidates(), 2);
    // Neither InferWeave patch is in the denominator at all: the collision
    // is out of the query, not filtered out of it.
    assert!(report.reconciles());
}

#[test]
fn in_flight_work_is_not_a_candidate() {
    // Defect 2: a directory appears the moment an agent starts, so counting
    // issue directories offered in-flight work as a candidate and a whole
    // pass printed `SKIP: no patch`. The finished_patches exclusion removes
    // it and reports the removal.
    let items = vec![
        AgentPatch {
            project: "autospec",
            issue: 7,
            patch_nonempty: false, // the agent is still working
            has_pr: false,
            issue_closed: false,
            attempted: false,
        },
        AgentPatch {
            project: "autospec",
            issue: 8,
            patch_nonempty: true, // finished
            has_pr: false,
            issue_closed: false,
            attempted: false,
        },
    ];
    let report = conversion_spec().select(&items);
    assert_eq!(report.removed_by("finished_patches"), Some(1));
    assert_eq!(report.candidates(), 1);
    assert_eq!(report.verdict(), SelectionVerdict::Work);
}

#[test]
fn the_spec_is_the_artifact_a_rebuild_of_it_is_the_same_predicate() {
    // Defect 3: a filter from a previous run was applied without being
    // re-derived. When the predicate is an artifact, reconstructing it from
    // the written spec is deterministic: same spec, same line, same
    // comment block.
    let spec_a = conversion_spec();
    let spec_b = conversion_spec();
    assert_eq!(spec_a.render(), spec_b.render());
    assert_eq!(spec_a.exclusion_names(), spec_b.exclusion_names());

    let items = vec![
        AgentPatch {
            project: "autospec",
            issue: 1,
            patch_nonempty: true,
            has_pr: true,
            issue_closed: false,
            attempted: false,
        },
        AgentPatch {
            project: "autospec",
            issue: 2,
            patch_nonempty: false,
            has_pr: false,
            issue_closed: false,
            attempted: false,
        },
    ];
    assert_eq!(spec_a.select(&items).line(), spec_b.select(&items).line());
}
