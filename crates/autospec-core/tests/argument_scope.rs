//! Argument parsers and environment-scoped runs (issue #4292).
//!
//! The regression tests instantiate the configuration the incident
//! produced: a project-scoped tool whose scope comes from environment
//! variables (`R=` / `OUT=` / `SEEN=`), whose argument loop had no
//! `*)` branch, run per-project in a loop. `convselect.sh iw` was
//! accepted, ignored, and scoped to autospec anyway, so both runs
//! printed byte-identical counts — and InferWeave's real candidate
//! (issue #288) sat behind the swallowed argument.

use autospec_core::argument_scope::{
    identical_output_line, identical_output_pairs, per_project_findings, unreported_scope_runs,
    validate_rejection_message, EnvScope, Rejection, RunOutput, StrictParser,
};

/// The scope mechanism the tool actually used: environment variables,
/// never positional arguments.
const MECHANISM: &str = "scope with R=/OUT=/SEEN= env vars, not positionally";

/// The counts both runs printed in the incident — byte-identical, which
/// is the signal.
const IDENTICAL_COUNTS: &str =
    "considered=100 finished_patches=3 closed_issue=0 candidates=0 retry-held=2";

fn convselect_parser() -> StrictParser {
    StrictParser::new(["--retry-held", "--commit"], MECHANISM)
}

/// The incident loop: two projects, one of them addressed positionally
/// (`convselect.sh iw`), which the parser swallowed. The loop intended
/// distinct scopes for the two runs, both printed the autospec counts
/// (byte-identical), and neither printed its scope.
fn incident_runs() -> Vec<RunOutput> {
    vec![
        RunOutput {
            name: "autospec".to_string(),
            scope: EnvScope::new([
                ("R", "/quobyte/metabolomicsgrp/it/llm/repos/autospec"),
                ("OUT", "out/autospec-conv"),
                ("SEEN", "out/autospec-conv/seen.tsv"),
            ]),
            output: IDENTICAL_COUNTS.to_string(),
        },
        // The `iw` never reached the scope: the run's output is still the
        // autospec counts, not InferWeave's.
        RunOutput {
            name: "inferweave".to_string(),
            scope: EnvScope::new([
                ("R", "/quobyte/metabolomicsgrp/it/llm/repos/inferweave"),
                ("OUT", "out/iw-conv"),
                ("SEEN", "out/iw-conv/seen.tsv"),
            ]),
            output: IDENTICAL_COUNTS.to_string(),
        },
    ]
}

// --- Invariant 1: the `*)` catch-all --------------------------------------

#[test]
fn an_unknown_argument_is_rejected_not_swallowed() {
    // The incident invocation: `convselect.sh iw`.
    let parser = convselect_parser();
    let err = parser.parse(&["iw"]).unwrap_err();
    assert_eq!(
        err,
        Rejection {
            argument: "iw".to_string(),
            mechanism: MECHANISM.to_string(),
        }
    );
}

#[test]
fn known_arguments_still_parse() {
    let parser = convselect_parser();
    let ok = parser.parse(&["--retry-held", "--commit"]).unwrap();
    assert_eq!(ok, vec!["--retry-held".to_string(), "--commit".to_string()]);
    assert!(parser.parse(&[]).unwrap().is_empty());
}

#[test]
fn the_first_unknown_argument_is_the_rejection() {
    let parser = convselect_parser();
    let err = parser
        .parse(&["--commit", "bogus", "also-bogus"])
        .unwrap_err();
    assert_eq!(err.argument, "bogus");
}

// --- Invariant 2: the rejection names the mechanism ------------------------

#[test]
fn the_rejection_line_names_the_argument_and_the_mechanism() {
    let parser = convselect_parser();
    let err = parser.parse(&["iw"]).unwrap_err();
    let line = err.line();
    assert!(
        line.contains("iw"),
        "line must name the offending argument: {line}"
    );
    assert!(
        line.contains(MECHANISM),
        "line must name the correct mechanism: {line}"
    );
    // The line the parser builds passes the check on a hand-written one.
    assert!(validate_rejection_message(&line, "iw", MECHANISM).is_empty());
}

#[test]
fn a_hand_written_rejection_that_drops_the_mechanism_is_flagged() {
    let findings = validate_rejection_message("error: unknown argument 'iw'", "iw", MECHANISM);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("REJECTION_MISSING_MECHANISM"));
}

#[test]
fn a_hand_written_rejection_that_drops_both_is_flagged_twice() {
    let findings = validate_rejection_message("error: unknown argument", "iw", MECHANISM);
    assert_eq!(findings.len(), 2);
    assert!(findings[0].starts_with("REJECTION_MISSING_ARGUMENT"));
    assert!(findings[1].starts_with("REJECTION_MISSING_MECHANISM"));
}

// --- Invariant 3: identical output across distinct inputs ------------------

#[test]
fn the_incident_identical_counts_are_a_defect() {
    let runs = incident_runs();
    let pairs = identical_output_pairs(&runs);
    assert_eq!(
        pairs,
        vec![("autospec".to_string(), "inferweave".to_string())]
    );
}

#[test]
fn distinct_counts_are_not_flagged() {
    let autospec_scope = EnvScope::new([("R", "repos/autospec"), ("OUT", "out/autospec-conv")]);
    let iw_scope = EnvScope::new([("R", "repos/inferweave"), ("OUT", "out/iw-conv")]);
    let runs = vec![
        RunOutput {
            name: "autospec".to_string(),
            scope: autospec_scope,
            output: IDENTICAL_COUNTS.to_string(),
        },
        // The fixed invocation: InferWeave's real candidate (#288) shows
        // up, so the output differs.
        RunOutput {
            name: "inferweave".to_string(),
            scope: iw_scope,
            output:
                "considered=40 finished_patches=1 closed_issue=0 candidates=1 retry-held=0\n288"
                    .to_string(),
        },
    ];
    assert!(identical_output_pairs(&runs).is_empty());
}

#[test]
fn identical_output_under_the_same_scope_is_an_idempotent_rerun() {
    let scope = EnvScope::new([("R", "repos/autospec"), ("OUT", "out/autospec-conv")]);
    let runs = vec![
        RunOutput {
            name: "first".to_string(),
            scope: scope.clone(),
            output: IDENTICAL_COUNTS.to_string(),
        },
        RunOutput {
            name: "second".to_string(),
            scope,
            output: IDENTICAL_COUNTS.to_string(),
        },
    ];
    assert!(identical_output_pairs(&runs).is_empty());
}

#[test]
fn the_identical_output_line_names_both_runs() {
    let line = identical_output_line("autospec", "inferweave");
    assert!(line.contains("autospec"));
    assert!(line.contains("inferweave"));
}

// --- Invariant 4: a scoped tool prints its scope on every run --------------

#[test]
fn the_scope_line_is_deterministic_in_var_insertion_order() {
    let a = EnvScope::new([("R", "r1"), ("OUT", "o1"), ("SEEN", "s1")]);
    let b = EnvScope::new([("SEEN", "s1"), ("R", "r1"), ("OUT", "o1")]);
    assert_eq!(a.line(), b.line());
    assert_eq!(a.line(), "scope: OUT=o1 R=r1 SEEN=s1");
}

#[test]
fn a_run_that_prints_its_scope_reports_it() {
    let scope = EnvScope::new([("R", "r1"), ("OUT", "o1")]);
    let output = format!("{}\n{IDENTICAL_COUNTS}", scope.line());
    assert!(scope.reported_in(&output));
}

#[test]
fn a_run_that_does_not_print_its_scope_is_named() {
    let runs = incident_runs();
    assert_eq!(
        unreported_scope_runs(&runs),
        vec!["autospec".to_string(), "inferweave".to_string()]
    );
}

// --- The combined per-project loop check -----------------------------------

#[test]
fn the_incident_loop_produces_findings() {
    let runs = incident_runs();
    let findings = per_project_findings(&runs);
    // One identical-output pair + two runs that never printed their scope.
    assert_eq!(findings.len(), 3);
    assert!(findings[0].starts_with("WARN: autospec and inferweave produced byte-identical output"));
    assert!(findings[1].contains("'autospec' did not report its scope"));
    assert!(findings[2].contains("'inferweave' did not report its scope"));
}

#[test]
fn the_fixed_loop_produces_no_findings() {
    let autospec_scope = EnvScope::new([("R", "repos/autospec"), ("OUT", "out/autospec-conv")]);
    let iw_scope = EnvScope::new([("R", "repos/inferweave"), ("OUT", "out/iw-conv")]);
    let runs = vec![
        RunOutput {
            name: "autospec".to_string(),
            scope: autospec_scope.clone(),
            output: format!("{}\n{IDENTICAL_COUNTS}", autospec_scope.line()),
        },
        RunOutput {
            name: "inferweave".to_string(),
            scope: iw_scope.clone(),
            output: format!(
                "{}\nconsidered=40 finished_patches=1 closed_issue=0 candidates=1 retry-held=0\n288",
                iw_scope.line()
            ),
        },
    ];
    assert!(per_project_findings(&runs).is_empty());
}
