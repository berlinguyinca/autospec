//! Exit-status guards: distinguishing "the tool said no" from "the tool
//! errored" (issue #4009).
//!
//! The incident: the conversion pass skipped an issue that already had a
//! branch or PR with
//! `headRefName | grep -qE "…(\\b|-)"`, reading every non-zero exit as "no
//! match, proceed". In an interactive shell that is right for `grep` (no
//! match is `1`). In the runner, `grep` was a shell *function* that exited
//! `2` on input it could not parse, and the guard read that `2` as the same
//! "no match" — so the issue was dispatched while its PR sat open.
//!
//! The fix makes "no match" and "error" different readings, and the guard
//! fails closed on the second. The four acceptance criteria, in the order the
//! incident produced them:
//!
//! * a non-zero status that is not the tool's documented negative status is an
//!   *error*, never a passing precondition (invariant 1);
//! * a tool whose behaviour varies by implementation is bound by absolute path
//!   or asserted once at startup (invariant 2);
//! * a guard's pattern is portable — no `\b`, `\d`, `\s` (invariant 3);
//! * a guard whose tool errors fails closed while a guard whose tool reports
//!   no match proceeds, and the two cases are distinguishable (AC4).

use autospec_core::exit_guard::{
    binding_is_safe, binding_note, decide, pattern_is_portable, portable_pattern_violations,
    read_exit, ExitReading, GuardVerdict, ToolBinding, ToolContract,
};

/// The `grep` contract the #4009 guard was written against: `0` a match, `1`
/// no match, everything else (notably the wrapper function's `2`) an error.
fn grep() -> ToolContract {
    ToolContract::GREP
}

// --- Invariant 1: a status is a match, a no-match, or an error ------------

#[test]
fn a_documented_match_status_is_a_match() {
    let reading = read_exit(0, grep());
    assert_eq!(reading, ExitReading::Matched, "{reading:?}");
    assert!(!reading.proceeds(), "{reading:?}");
    assert!(!reading.is_error(), "{reading:?}");
}

#[test]
fn a_documented_no_match_status_is_a_no_match() {
    let reading = read_exit(1, grep());
    assert_eq!(reading, ExitReading::NoMatch, "{reading:?}");
    assert!(reading.proceeds(), "{reading:?}");
    assert!(!reading.is_error(), "{reading:?}");
}

#[test]
fn the_incidents_exit_two_is_an_error_not_a_no_match() {
    // The exact defect: the runner's `grep` wrapper exited 2 on input it
    // could not parse. Under the old shared exit path this was read as "no
    // match" and the guard proceeded. Under the contract it is an error.
    let reading = read_exit(2, grep());
    assert_eq!(reading, ExitReading::Errored { status: 2 }, "{reading:?}");
    assert!(reading.is_error(), "{reading:?}");
    assert!(
        !reading.proceeds(),
        "exit 2 must not be treated as a pass: {reading:?}"
    );
}

#[test]
fn any_other_non_zero_status_is_also_an_error() {
    // Not just `2`: `137` (SIGKILL), `127` (not found), `255` are all errors.
    for status in [3, 127, 128, 137, 255] {
        let reading = read_exit(status, grep());
        assert_eq!(reading, ExitReading::Errored { status }, "{reading:?}");
        assert!(reading.is_error(), "{reading:?}");
        assert!(!reading.proceeds(), "{reading:?}");
    }
}

#[test]
fn the_contract_decides_which_non_zero_status_is_a_no_match() {
    // The invariant is about the *documented* negative status, not the number
    // 1 specifically. A tool whose contract documents 3 as "no" reads 3 as a
    // no-match and 1 as an error.
    let contract = ToolContract {
        match_status: 0,
        no_match_status: 3,
    };
    assert_eq!(read_exit(3, contract), ExitReading::NoMatch);
    assert_eq!(read_exit(1, contract), ExitReading::Errored { status: 1 });
    assert_eq!(read_exit(0, contract), ExitReading::Matched);
}

#[test]
fn is_documented_covers_exactly_the_two_answered_statuses() {
    assert!(grep().is_documented(0));
    assert!(grep().is_documented(1));
    assert!(
        !grep().is_documented(2),
        "2 is the error case, not a documented answer"
    );
    assert!(!grep().is_documented(137));
}

// --- The decision: proceed vs block, and the two blocks stay distinct ------

#[test]
fn decide_proceeds_only_on_a_no_match() {
    assert_eq!(decide(ExitReading::NoMatch), GuardVerdict::Proceed);
    assert_eq!(decide(ExitReading::Matched), GuardVerdict::AlreadyExists);
    assert_eq!(
        decide(ExitReading::Errored { status: 2 }),
        GuardVerdict::FailClosed { status: 2 }
    );
}

#[test]
fn a_matched_guard_is_blocked_as_already_exists_not_failed() {
    let verdict = decide(read_exit(0, grep()));
    assert_eq!(verdict, GuardVerdict::AlreadyExists, "{verdict:?}");
    assert!(!verdict.proceeds(), "{verdict:?}");
}

// --- AC4: error fails closed, no-match proceeds, and the two are distinct --

#[test]
fn an_error_status_fails_closed_and_a_no_match_proceeds() {
    // The two cases the issue requires the test to distinguish, driven from
    // the raw exit status exactly as a wrapper observes it.
    let no_match = decide(read_exit(1, grep()));
    let errored = decide(read_exit(2, grep()));

    assert_eq!(no_match, GuardVerdict::Proceed);
    assert!(no_match.proceeds(), "a no-match must proceed: {no_match:?}");

    assert_eq!(errored, GuardVerdict::FailClosed { status: 2 });
    assert!(
        !errored.proceeds(),
        "an error must fail closed, not proceed: {errored:?}"
    );
}

#[test]
fn an_error_and_a_no_match_are_distinguishable_states() {
    // One shared exit path is the defect: it collapses these two. Here they
    // are different variants, so the guard's log line — and this test — can
    // tell a real "no match" from a guard that misfired on a 2.
    let no_match = decide(read_exit(1, grep()));
    let errored = decide(read_exit(2, grep()));
    assert_ne!(
        no_match, errored,
        "no-match and error must not be the same state"
    );

    // The fail-closed line names the status; the no-match line does not.
    let no_match_line = no_match.line("grep");
    let errored_line = errored.line("grep");
    assert!(no_match_line.contains("no match"), "{no_match_line}");
    assert!(no_match_line.contains("proceed"), "{no_match_line}");
    assert!(errored_line.contains("fail closed"), "{errored_line}");
    assert!(errored_line.contains("exit 2"), "{errored_line}");
    assert_ne!(
        no_match_line, errored_line,
        "the two log lines must differ: {no_match_line} vs {errored_line}"
    );
}

#[test]
fn an_error_and_already_exists_are_also_distinct_blocks() {
    // "Block" has two different reasons: the thing exists, or the guard broke.
    // Conflating them is the same defect one direction over.
    let already = decide(read_exit(0, grep()));
    let errored = decide(read_exit(2, grep()));
    assert!(!already.proceeds());
    assert!(!errored.proceeds());
    assert_ne!(
        already, errored,
        "already-exists and fail-closed must not be the same state"
    );
    assert!(already.line("grep").contains("already exists"));
    assert!(errored.line("grep").contains("fail closed"));
}

// --- Invariant 2: bind the tool or assert its implementation ---------------

#[test]
fn an_absolute_path_binding_is_safe() {
    let binding = ToolBinding::AbsolutePath {
        path: "/usr/bin/grep".to_string(),
    };
    assert!(binding_is_safe(&binding), "{binding:?}");
    assert!(
        binding_note(&binding).contains("/usr/bin/grep"),
        "{}",
        binding_note(&binding)
    );
}

#[test]
fn an_asserted_lookup_is_safe_and_records_its_evidence() {
    let binding = ToolBinding::Asserted {
        name: "grep".to_string(),
        evidence: "resolved to /usr/bin/grep (GNU grep 3.11)".to_string(),
    };
    assert!(binding_is_safe(&binding), "{binding:?}");
    let note = binding_note(&binding);
    assert!(note.contains("asserted"), "{note}");
    assert!(note.contains("GNU grep 3.11"), "{note}");
}

#[test]
fn an_unasserted_path_lookup_is_not_safe() {
    let binding = ToolBinding::PathLookup {
        name: "grep".to_string(),
    };
    assert!(
        !binding_is_safe(&binding),
        "an unasserted PATH lookup is the risk: {binding:?}"
    );
    let note = binding_note(&binding);
    assert!(note.contains("unasserted"), "{note}");
    assert!(note.contains("absolute path"), "{note}");
}

#[test]
fn a_shell_function_binding_is_not_safe_and_is_named_as_such() {
    // The #4009 defect, stated directly: `grep` was a shell function.
    let binding = ToolBinding::ShellFunction {
        name: "grep".to_string(),
    };
    assert!(!binding_is_safe(&binding), "{binding:?}");
    let note = binding_note(&binding);
    assert!(note.contains("shell function"), "{note}");
    assert!(note.contains("not the grep binary"), "{note}");
}

// --- Invariant 3: the pattern must be portable -----------------------------

#[test]
fn the_incidents_pattern_is_flagged_for_word_boundary() {
    // The conversion pass's pattern used `\b`, which is not POSIX.
    let pattern = "(issue-|conv/|cp/|probe/)$n(\\b|-)";
    let violations = portable_pattern_violations(pattern);
    assert_eq!(violations, vec!["\\b".to_string()], "{violations:?}");
    assert!(!pattern_is_portable(pattern), "{pattern}");
}

#[test]
fn each_non_posix_escape_is_flagged() {
    for ch in ['b', 'B', 'd', 'D', 's', 'S', 'w', 'W'] {
        let pattern = format!("x\\{ch}");
        let violations = portable_pattern_violations(&pattern);
        assert_eq!(
            violations,
            vec![format!("\\{ch}")],
            "pattern {pattern} -> {violations:?}"
        );
        assert!(!pattern_is_portable(&pattern), "{pattern}");
    }
}

#[test]
fn a_portable_pattern_passes() {
    // Explicit anchoring and POSIX classes, no backslash-escape shorthands.
    let pattern = "(^|[^A-Za-z0-9_])(issue-|conv/|cp/|probe/)";
    assert!(pattern_is_portable(pattern), "{pattern}");
    assert!(portable_pattern_violations(pattern).is_empty());
}

#[test]
fn duplicates_are_reported_once_in_order_of_first_appearance() {
    let violations = portable_pattern_violations("a\\b b\\d c\\s b\\b d\\w");
    assert_eq!(
        violations,
        vec![
            "\\b".to_string(),
            "\\d".to_string(),
            "\\s".to_string(),
            "\\w".to_string()
        ],
        "{violations:?}"
    );
}

#[test]
fn an_escaped_backslash_is_not_an_escape() {
    // `\\b` is a literal backslash followed by a literal `b`: the `b` is not a
    // word boundary and must not be flagged.
    assert!(
        pattern_is_portable("C:\\\\bin"),
        "C:\\\\bin should be portable"
    );
    assert!(portable_pattern_violations("C:\\\\bin").is_empty());
    // A lone trailing backslash is not an escape either.
    assert!(portable_pattern_violations("abc\\").is_empty());
}

#[test]
fn legitimate_posix_escapes_are_not_flagged() {
    // Escaped metacharacters are POSIX and must not trip the guard.
    for pattern in [
        "\\.", "\\(", "\\)", "\\[", "\\]", "\\\\", "\\\\\\\\", "a\\\\b",
    ] {
        assert!(
            pattern_is_portable(pattern),
            "{pattern} should be portable, got {:?}",
            portable_pattern_violations(pattern)
        );
    }
}

// --- Serde: the readings serialize to distinct, stable forms ---------------

#[test]
fn the_readings_serialize_to_distinct_stable_forms() {
    // A wrapper that records these in JSON must be able to tell the three
    // readings apart after a round trip.
    let cases = [
        (ExitReading::Matched, "\"matched\""),
        (ExitReading::NoMatch, "\"no_match\""),
        (
            ExitReading::Errored { status: 2 },
            "{\"errored\":{\"status\":2}}",
        ),
    ];
    for (reading, json) in cases {
        let got = serde_json::to_string(&reading).expect("serialize");
        assert_eq!(got, json, "{reading:?}");
        let back: ExitReading = serde_json::from_str(&got).expect("deserialize");
        assert_eq!(back, reading, "{reading:?}");
    }
}

#[test]
fn the_verdicts_serialize_to_distinct_stable_forms() {
    let cases = [
        (GuardVerdict::Proceed, "\"proceed\""),
        (GuardVerdict::AlreadyExists, "\"already_exists\""),
        (
            GuardVerdict::FailClosed { status: 2 },
            "{\"fail_closed\":{\"status\":2}}",
        ),
    ];
    for (verdict, json) in cases {
        let got = serde_json::to_string(&verdict).expect("serialize");
        assert_eq!(got, json, "{verdict:?}");
        let back: GuardVerdict = serde_json::from_str(&got).expect("deserialize");
        assert_eq!(back, verdict, "{verdict:?}");
    }
}
