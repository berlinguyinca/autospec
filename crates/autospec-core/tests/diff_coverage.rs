//! Regression tests for the diff-coverage gate (issue #4003).
//!
//! Incident configuration: an agent produced 291 lines of new routing logic
//! — a routing filter, a suspension state machine, a background recovery
//! loop — and no test file. The whole gate set passed (format clean, build
//! ok, vet ok, suite green), because every gate asked "did anything break?"
//! and nothing did: the new code was simply never executed. The suite would
//! have been equally green had the filter been written to return the wrong
//! workers.
//!
//! The four acceptance cases are the tests that must fail if any of the
//! invariants regresses: an uncovered function refuses and is named; the
//! same patch with a test passes; a docs-only patch is exempt without a
//! human deciding; and a new test that passes against the unmodified tree
//! refuses as a vacuous test.

use autospec_core::diff_coverage::{
    diff_coverage, gate, is_executable, vacuous_tests, AddedLine, GateVerdict, LineRef,
    PreChangeRun,
};

fn line(file: &str, line_no: usize, text: &str, symbol: Option<&str>) -> AddedLine {
    AddedLine {
        file: file.to_string(),
        line_no,
        text: text.to_string(),
        symbol: symbol.map(String::from),
    }
}

/// The incident, scaled down: the routing filter the agent added — a
/// comment, the function signature, one executable statement, the closing
/// brace. The executable lines are 42 and 43.
fn router_patch() -> Vec<AddedLine> {
    vec![
        line(
            "internal/gateway/router.go",
            41,
            "// route_to_worker decides whether inference traffic reaches w.",
            Some("route_to_worker"),
        ),
        line(
            "internal/gateway/router.go",
            42,
            "func route_to_worker(w *worker, req *Request) bool {",
            Some("route_to_worker"),
        ),
        line(
            "internal/gateway/router.go",
            43,
            "return !w.suspended && w.ready(req)",
            Some("route_to_worker"),
        ),
        line(
            "internal/gateway/router.go",
            44,
            "}",
            Some("route_to_worker"),
        ),
    ]
}

/// AC4: a patch adding an uncovered function is rejected, and the
/// refusal names the function.
#[test]
fn uncovered_function_is_refused_and_named() {
    let verdict = gate(&router_patch(), &[], &[]);
    assert!(
        !verdict.passed(),
        "the gate must refuse, got: {}",
        verdict.line()
    );
    let GateVerdict::Refused {
        coverage,
        uncovered,
        vacuous,
    } = &verdict
    else {
        panic!("gate must refuse, got: {}", verdict.line())
    };
    assert!(vacuous.is_empty());
    assert_eq!(
        uncovered,
        &[
            LineRef {
                file: "internal/gateway/router.go".into(),
                line_no: 42,
                symbol: Some("route_to_worker".into()),
            },
            LineRef {
                file: "internal/gateway/router.go".into(),
                line_no: 43,
                symbol: Some("route_to_worker".into()),
            },
        ],
        "the comment and the closing brace are not executable; the two \
         statement lines are, in patch order"
    );
    assert_eq!(
        verdict.line(),
        "diff coverage: 0/2 executable lines covered — uncovered: \
         internal/gateway/router.go:42 (route_to_worker), \
         internal/gateway/router.go:43 (route_to_worker) — gate refused: \
         2 uncovered line(s)",
        "the refusal names the uncovered lines and the function"
    );
    assert_eq!(coverage.executable, 2);
    assert_eq!(coverage.covered, 0);
}

/// AC4: the same patch with a test that executes the new lines is accepted.
#[test]
fn same_patch_with_a_test_is_accepted() {
    let executed = vec![
        ("internal/gateway/router.go".to_string(), 42),
        ("internal/gateway/router.go".to_string(), 43),
    ];
    let pre_change = vec![PreChangeRun {
        test: "TestRouteToWorker".to_string(),
        failed: true,
    }];
    let verdict = gate(&router_patch(), &executed, &pre_change);
    assert!(verdict.passed(), "got: {}", verdict.line());
    assert_eq!(
        verdict.line(),
        "diff coverage: 2/2 executable lines covered — gate passed"
    );
}

/// AC4: a docs-only patch is accepted with no coverage demanded, and no
/// human decides the exemption.
#[test]
fn docs_only_patch_is_exempt() {
    let patch = vec![
        line(
            "docs/specs/2026-08-02-routing-filter-design.md",
            12,
            "The filter suspends a worker after three consecutive timeouts.",
            None,
        ),
        line(
            "docs/specs/2026-08-02-routing-filter-design.md",
            13,
            "",
            None,
        ),
        line(
            "internal/gateway/router.go",
            60,
            "// recovery re-checks suspended workers",
            None,
        ),
        line("internal/gateway/router.go", 61, "}", None),
    ];
    let verdict = gate(&patch, &[], &[]);
    assert!(
        matches!(verdict, GateVerdict::Exempt),
        "got: {}",
        verdict.line()
    );
    assert!(verdict.passed());
    assert_eq!(
        verdict.line(),
        "diff coverage: no executable lines added — exempt"
    );
}

/// AC4: a patch whose new test passes against the unmodified tree is
/// rejected as a vacuous test — even when the coverage of the diff is
/// complete.
#[test]
fn new_test_that_passes_pre_change_is_rejected_as_vacuous() {
    let executed = vec![
        ("internal/gateway/router.go".to_string(), 42),
        ("internal/gateway/router.go".to_string(), 43),
    ];
    let pre_change = vec![PreChangeRun {
        test: "TestRouteToWorker".to_string(),
        failed: false,
    }];
    let verdict = gate(&router_patch(), &executed, &pre_change);
    assert!(
        !verdict.passed(),
        "a test that passes without the change is a defect in the test: {}",
        verdict.line()
    );
    match &verdict {
        GateVerdict::Refused {
            uncovered, vacuous, ..
        } => {
            assert!(uncovered.is_empty(), "the coverage half is complete");
            assert_eq!(vacuous, &["TestRouteToWorker".to_string()]);
        }
        other => panic!("gate must refuse as vacuous, got: {}", other.line()),
    }
    assert!(
        verdict
            .line()
            .contains("1 vacuous test(s): TestRouteToWorker"),
        "the refusal names the vacuous test: {}",
        verdict.line()
    );
}

/// Invariant 1: the measurement is of the diff, not of the project. Lines
/// the patch did not add are outside it by construction, and a project that
/// is 99% covered still fails the gate on one uncovered new line.
#[test]
fn coverage_is_of_the_diff_not_of_the_project() {
    let executed = vec![("internal/gateway/router.go".to_string(), 42)];
    let coverage = diff_coverage(&router_patch(), &executed);
    assert_eq!(coverage.executable, 2);
    assert_eq!(coverage.covered, 1);
    assert_eq!(coverage.uncovered.len(), 1);
    assert_eq!(
        coverage.line(),
        "diff coverage: 1/2 executable lines covered — uncovered: \
         internal/gateway/router.go:43 (route_to_worker)"
    );
}

/// Invariant 2: the classification is mechanical. Blank lines, comments
/// (line, block, continuation, hash), structure-only lines, and doc/spec
/// files are not executable; everything else is.
#[test]
fn is_executable_classifies_each_line_mechanically() {
    assert!(
        !is_executable(&line("internal/gateway/router.go", 1, "", None)),
        "blank"
    );
    assert!(
        !is_executable(&line(
            "internal/gateway/router.go",
            2,
            "// a line comment",
            None
        )),
        "line comment"
    );
    assert!(
        !is_executable(&line(
            "internal/gateway/router.go",
            3,
            "/* block open",
            None
        )),
        "block comment open"
    );
    assert!(
        !is_executable(&line(
            "internal/gateway/router.go",
            4,
            " * continuation",
            None
        )),
        "block comment continuation"
    );
    assert!(
        !is_executable(&line(
            "internal/gateway/router.go",
            5,
            "*/ block close",
            None
        )),
        "block comment close"
    );
    assert!(
        !is_executable(&line("scripts/probe.sh", 6, "# a hash comment", None)),
        "hash comment"
    );
    assert!(
        !is_executable(&line("internal/gateway/router.go", 7, "};", None)),
        "structure only"
    );
    assert!(
        !is_executable(&line(
            "docs/specs/design.md",
            8,
            "Code-looking text in a spec.",
            None
        )),
        "doc file"
    );
    assert!(
        !is_executable(&line("README.txt", 9, "release notes line", None)),
        "spec text file"
    );
    assert!(
        is_executable(&line(
            "internal/gateway/router.go",
            10,
            "return !w.suspended && w.ready(req)",
            None
        )),
        "a statement is executable"
    );
    assert!(
        is_executable(&line("Makefile", 11, "build: go build ./...", None)),
        "no extension: over-classifying is the safe direction"
    );
}

/// Invariant 3: only a recorded pre-change failure clears a new test. The
/// vacuous list is exactly the tests that passed there.
#[test]
fn vacuous_tests_lists_only_the_ones_that_passed_pre_change() {
    let runs = vec![
        PreChangeRun {
            test: "TestRouteToWorker".into(),
            failed: true,
        },
        PreChangeRun {
            test: "TestSuspensionStateMachine".into(),
            failed: false,
        },
        PreChangeRun {
            test: "TestRecoveryLoop".into(),
            failed: false,
        },
    ];
    assert_eq!(
        vacuous_tests(&runs),
        vec![
            "TestSuspensionStateMachine".to_string(),
            "TestRecoveryLoop".to_string()
        ],
        "in recorded order; the test that failed pre-change is not vacuous"
    );
}

/// Invariant 3 applies to every new test, including one in a patch that
/// otherwise needs no coverage: a docs-only patch whose new test passes
/// pre-change is still refused.
#[test]
fn docs_only_patch_with_a_vacuous_test_is_still_refused() {
    let patch = vec![line(
        "docs/specs/design.md",
        1,
        "The filter is idempotent.",
        None,
    )];
    let pre_change = vec![PreChangeRun {
        test: "TestFilterIdempotent".into(),
        failed: false,
    }];
    let verdict = gate(&patch, &[], &pre_change);
    assert!(!verdict.passed(), "got: {}", verdict.line());
    assert!(
        verdict
            .line()
            .contains("1 vacuous test(s): TestFilterIdempotent"),
        "got: {}",
        verdict.line()
    );
}

/// A line whose symbol the parser did not extract is named by position
/// only — the refusal still names it.
#[test]
fn uncovered_line_without_a_symbol_is_named_by_position() {
    let patch = vec![line(
        "internal/gateway/router.go",
        71,
        "workers.Suspend(w)",
        None,
    )];
    let verdict = gate(&patch, &[], &[]);
    assert!(
        verdict.line().contains("internal/gateway/router.go:71"),
        "got: {}",
        verdict.line()
    );
}
