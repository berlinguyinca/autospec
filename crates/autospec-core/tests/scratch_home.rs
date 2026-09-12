//! Home of recurring automation (issue #4387).
//!
//! The incident: the patch-to-PR conversion pass — selecting candidate
//! patches, applying them, running the gate, opening PRs, recording HELD
//! reasons — was a set of shell scripts in a session temp directory. It
//! encoded real accumulated knowledge: which failures are worth holding vs
//! discarding, how to extract the true failing-test set, which conflicts can
//! be safely unioned. None of it was in a repository. The same directory
//! also accumulated ~60 git worktrees and dozens of ad-hoc logs, so the real
//! tooling was indistinguishable from debris. When the scratch directory was
//! cleared the capability was simply gone, along with every lesson embedded
//! in it.
//!
//! The regression test reconstructs the incident: a spec for the pass that
//! names its scripts under `/tmp/conv/` with no repository home and no test.
//! The remedies: the same spec that names `scripts/` homes and a `tests/`
//! path is clean; an mktemp template (one task, one lifetime) is exempt; a
//! deliberately one-shot script carries `linter:allow-SCRATCH_HOME <reason>`.

use autospec_core::scratch_home::{
    is_repo_home_candidate, is_scratch_template, is_test_candidate, lint_spec_scratch_home,
    scratch_tool_paths, ScratchHomeFinding, SCRATCH_HOME_RULE_ID,
};

/// The incident, as a spec: a recurring operational process whose
/// implementation lives in a session temp directory, no home, no test.
fn incident_spec() -> &'static str {
    "# patch-to-PR conversion pass\n\n\
     ## Goal\n\n\
     Run the daily conversion: select candidate patches, apply them, run the\n\
     gate, open PRs, and record HELD reasons.\n\n\
     ## Implementation\n\n\
     The pass is a set of shell scripts in the session temp directory:\n\
     /tmp/conv/convpass.sh selects, /tmp/conv/gate.sh gates, and\n\
     /tmp/conv/open_pr.sh opens. Conflicts and HELD notes go to /tmp/conv.log.\n"
}

#[test]
fn incident_spec_is_a_design_defect() {
    let findings = lint_spec_scratch_home(incident_spec());
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule_id(), SCRATCH_HOME_RULE_ID);
    assert_eq!(
        findings[0].scratch_tools,
        vec![
            "/tmp/conv/convpass.sh".to_owned(),
            "/tmp/conv/gate.sh".to_owned(),
            "/tmp/conv/open_pr.sh".to_owned(),
        ]
    );
    assert!(!findings[0].has_repo_home);
    assert!(!findings[0].has_test);
    assert_eq!(findings[0].missing(), vec!["no repository home", "no test"]);
}

#[test]
fn incident_debris_is_not_counted_as_tooling() {
    // The HELD-note log and the worktree debris are not tool paths.
    let tools = scratch_tool_paths("notes to /tmp/conv.log; worktrees under /tmp/wt-*");
    assert!(tools.is_empty());
}

#[test]
fn promoted_spec_is_clean() {
    let source = "\
The session copies /tmp/conv/convpass.sh for the first throwaway run only.
The implementation lives at scripts/convpass.sh, scripts/convpass-gate.sh,
and scripts/convpass-open-pr.sh; tests/convpass.bats pins the behavior.
";
    assert!(lint_spec_scratch_home(source).is_empty());
}

#[test]
fn home_without_test_is_still_a_finding() {
    let source = "Move /tmp/conv/convpass.sh to scripts/convpass.sh.\n";
    let findings = lint_spec_scratch_home(source);
    assert_eq!(findings.len(), 1);
    assert!(matches!(
        &findings[0],
        ScratchHomeFinding {
            has_repo_home: true,
            has_test: false,
            ..
        }
    ));
    assert_eq!(findings[0].missing(), vec!["no test"]);
}

#[test]
fn test_path_is_not_an_implementation_home() {
    // A `tests/` segment names a test, not a home: the finding must still
    // name the missing repository home.
    let source = "/tmp/conv/convpass.sh is pinned by tests/convpass.bats.\n";
    let findings = lint_spec_scratch_home(source);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].missing(), vec!["no repository home"]);
}

#[test]
fn mktemp_template_is_one_task_lifetime() {
    let source = "Each worker writes /tmp/conv.XXXXXX/run.sh and removes it on exit.\n";
    assert!(lint_spec_scratch_home(source).is_empty());
    assert!(is_scratch_template("/tmp/conv.XXXXXX/run.sh"));
}

#[test]
fn deliberately_one_shot_script_is_escaped_with_reason() {
    let source = "\
/tmp/once.sh is a one-shot migration run exactly once on 2026-09-11.
linter:allow-SCRATCH_HOME discarded after the rollout, kept nowhere
";
    assert!(lint_spec_scratch_home(source).is_empty());
}

#[test]
fn bare_escape_marker_is_rejected() {
    let source = "linter:allow-SCRATCH_HOME\n/tmp/once.sh\n";
    let findings = lint_spec_scratch_home(source);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].scratch_tools, vec!["/tmp/once.sh".to_owned()]);
}

#[test]
fn rust_home_and_rust_test_are_recognized() {
    assert!(is_repo_home_candidate(
        "crates/autospec-core/src/scratch_home.rs"
    ));
    assert!(is_test_candidate(
        "crates/autospec-core/tests/scratch_home.rs"
    ));
    let source = "
Implementation: crates/autospec-core/src/scratch_home.rs, tested by
crates/autospec-core/tests/scratch_home.rs; the session copy at
/tmp/autospec-core/scratch_home.rs was throwaway.
";
    assert!(lint_spec_scratch_home(source).is_empty());
}
