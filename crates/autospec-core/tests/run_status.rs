//! The run-status vocabulary, and the two ways it was broken in #4206.
//!
//! A verdict guard matched `BUILD-FAILED`, `UNKNOWN-NO-BASELINE` and
//! `UNKNOWN-NO-FMT-BASELINE`; the runner writes `BUILD-FAIL`, and the third name
//! does not exist. The guard therefore matched nothing it was written to match, and
//! reported every run as passing. In the same corpus, 183 of 183 runs recorded
//! `status=VERIFIED` beside a `test_rc` of 101: a verdict written alongside its
//! fields instead of derived from them, and a label that separated nothing.
//!
//! These tests hold the four invariants from that issue: a match list is asserted
//! against the vocabulary it claims to implement, a verdict is derived from the
//! fields beside it, a constant label is reported as carrying no information, and a
//! shared vocabulary has one definition that consumers are checked against.

use autospec_core::run_status::{
    audit_canonicalising_match_list, audit_match_list, audit_population, canonical_status, emitted,
    entry, gate_statuses, is_declared, label_distribution, unknown_status_refusal, vocabulary,
    Derivation, EntryKind, RunEvidence, RunRecord, Status, VerdictCheck, VOCABULARY_PATH,
};

// ---------------------------------------------------------------------------
// Invariant 4: one definition, and the file is it.
// ---------------------------------------------------------------------------

#[test]
fn the_vocabulary_parses_and_has_the_expected_shape() {
    // The header row names the columns; it must not become an entry.
    assert!(
        entry("name").is_none(),
        "the header row of {VOCABULARY_PATH} parsed as a vocabulary entry"
    );
    assert_eq!(emitted().len(), 8, "emitted statuses: {:?}", emitted());
    assert_eq!(aliases().len(), 5);
    assert_eq!(gate_statuses().len(), 2);
    assert_eq!(vocabulary().len(), 15);
}

fn aliases() -> Vec<&'static str> {
    vocabulary()
        .iter()
        .filter(|e| e.kind == EntryKind::Alias)
        .map(|e| e.name)
        .collect()
}

#[test]
fn every_emitted_entry_is_its_own_canonical() {
    for e in vocabulary().iter().filter(|e| e.kind == EntryKind::Emitted) {
        assert_eq!(
            e.canonical.map(|s| s.as_str()),
            Some(e.name),
            "emitted entry {} must be its own canonical",
            e.name
        );
        assert_eq!(canonical_status(e.name).map(|s| s.as_str()), Some(e.name));
    }
}

#[test]
fn every_alias_resolves_to_a_declared_canonical() {
    // An alias must resolve to a name that means something, but not necessarily to
    // something the runner emits: `UNKNOWN-NO-FMT-BASELINE` was written by a gate
    // that conflated its own `UNKNOWN-NO-BASELINE` with the runner's `FMT-DIRTY`,
    // and resolves to the gate status it was mistaken for.
    for e in vocabulary().iter().filter(|e| e.kind == EntryKind::Alias) {
        let target = e
            .canonical
            .unwrap_or_else(|| panic!("alias {} resolves to nothing", e.name));
        let resolved = entry(target.as_str())
            .unwrap_or_else(|| panic!("alias {} resolves to undeclared {:?}", e.name, target));
        assert!(
            matches!(resolved.kind, EntryKind::Emitted | EntryKind::Gate),
            "alias {} resolves to {}, which is itself an alias",
            e.name,
            target.as_str()
        );
        assert_ne!(target.as_str(), e.name, "alias {} points at itself", e.name);
    }
}

#[test]
fn gate_entries_are_their_own_canonical_and_never_emitted() {
    for e in vocabulary().iter().filter(|e| e.kind == EntryKind::Gate) {
        assert_eq!(e.canonical.map(|s| s.as_str()), Some(e.name));
        assert!(
            !emitted().contains(&e.name),
            "gate-only status {} must not be listed as emitted",
            e.name
        );
    }
}

#[test]
fn every_status_variant_is_declared() {
    // Invariant 4: a variant nobody declared cannot exist. The parser checks this
    // on every load for emitted and gate rows; this asserts the enum has no member
    // the file does not name.
    for status in [
        Status::Verified,
        Status::NewTestFailures,
        Status::TestTimeout,
        Status::NoOutput,
        Status::Timeout,
        Status::FmtDirty,
        Status::BuildFail,
        Status::TimeoutNoOutput,
        Status::UnknownNoBaseline,
        Status::NoTestDb,
    ] {
        assert!(
            is_declared(status.as_str()),
            "{} has no row in {VOCABULARY_PATH}",
            status.as_str()
        );
        let kinds: Vec<EntryKind> = vocabulary()
            .iter()
            .filter(|e| e.canonical == Some(status))
            .map(|e| e.kind)
            .collect();
        assert!(
            kinds.contains(&EntryKind::Emitted) || kinds.contains(&EntryKind::Gate),
            "{} resolves nothing: no emitted or gate row carries it",
            status.as_str()
        );
    }
}

#[test]
fn legacy_spellings_resolve_to_the_status_the_runner_writes() {
    // Invariant 1, retro-fixed: the guard matched these three. Two are real names
    // under a different spelling and resolve; the third never existed.
    assert_eq!(canonical_status("BUILD-FAILED"), Some(Status::BuildFail));
    assert_eq!(
        canonical_status("TESTS-DO-NOT-COMPILE"),
        Some(Status::BuildFail)
    );
    assert_eq!(
        canonical_status("UNKNOWN-NO-FMT-BASELINE"),
        Some(Status::UnknownNoBaseline)
    );
    assert_eq!(canonical_status("PASS"), Some(Status::Verified));
    assert_eq!(
        canonical_status("VERIFIED-ABSOLUTE"),
        Some(Status::Verified)
    );
    // The runner's own spelling resolves to itself, not through anything.
    assert_eq!(canonical_status("BUILD-FAIL"), Some(Status::BuildFail));
}

#[test]
fn an_undeclared_name_resolves_to_nothing_and_the_refusal_names_the_file() {
    // Refuse and name what would tell you: no default, no guess.
    assert_eq!(canonical_status("UNKNOWN-NO-FMT-BASELINE-TYPO"), None);
    assert_eq!(
        canonical_status("verified"),
        None,
        "the vocabulary is upper-case"
    );
    let refusal = unknown_status_refusal("convpass.sh", "UNKNOWN-NO-FMT-BASELINE-TYPO");
    assert!(refusal.contains("convpass.sh"), "{refusal}");
    assert!(refusal.contains(VOCABULARY_PATH), "{refusal}");
    assert!(refusal.contains("refusing"), "{refusal}");
}

// ---------------------------------------------------------------------------
// Invariant 1: a match list is asserted against the vocabulary, not beside it.
// ---------------------------------------------------------------------------

#[test]
fn the_4206_guard_match_list_is_audited_as_dead() {
    // The guard's actual list, verbatim. It must not pass the audit.
    let audit = audit_match_list(
        "convpass.sh",
        &[
            "BUILD-FAILED",
            "UNKNOWN-NO-BASELINE",
            "UNKNOWN-NO-FMT-BASELINE",
        ],
    );
    assert!(!audit.ok(), "the #4206 list must not satisfy the audit");
    // Every entry in the list is a name that exists somewhere and is never written
    // by the runner: BUILD-FAILED and UNKNOWN-NO-FMT-BASELINE are legacy aliases,
    // UNKNOWN-NO-BASELINE is a gate-only status. All three are dead literals.
    assert_eq!(audit.dead.len(), 3, "{:?}", audit.dead);
    for name in [
        "BUILD-FAILED",
        "UNKNOWN-NO-BASELINE",
        "UNKNOWN-NO-FMT-BASELINE",
    ] {
        assert!(
            audit.dead.iter().any(|d| d.starts_with(name)),
            "{name} should be dead: {:?}",
            audit.dead
        );
    }
    // All three ARE declared, so nothing lands in the undeclared bucket; the defect
    // is that they are declared as things the runner does not write.
    assert!(audit.undeclared.is_empty(), "{:?}", audit.undeclared);
    // And BUILD-FAIL — the thing that actually happened — is unmatched.
    assert!(
        audit.uncovered.contains(&"BUILD-FAIL"),
        "{:?}",
        audit.uncovered
    );
    assert!(audit.line().contains("never emits"), "{}", audit.line());
}

#[test]
fn an_invented_name_lands_in_undeclared_not_dead() {
    // The dead/undeclared split matters: a legacy spelling needs a rewrite, an
    // invented spelling means the consumer and the vocabulary have diverged.
    let audit = audit_match_list("invented.sh", &["BUILD-FAILDED"]);
    assert_eq!(audit.undeclared, vec!["BUILD-FAILDED"]);
    assert!(audit.dead.is_empty(), "{:?}", audit.dead);
}

#[test]
fn a_canonicalising_match_list_covers_through_aliases() {
    // A consumer that resolves first may match either spelling: an alias entry is
    // not dead for it, only for a consumer that matches the wire form verbatim.
    let audit = audit_canonicalising_match_list(
        "status_triage.rs",
        &["BUILD-FAILED", "BUILD-FAIL", "PASS", "TESTS-DO-NOT-COMPILE"],
    );
    assert!(audit.dead.is_empty(), "{}", audit.line());
    assert!(audit.undeclared.is_empty(), "{}", audit.line());
    assert!(
        !audit.ok(),
        "a partial list is not coverage: {}",
        audit.line()
    );
    // Covering every status through either spelling satisfies the audit.
    let full = audit_canonicalising_match_list(
        "status_triage.rs",
        &[
            "PASS", // -> VERIFIED
            "NEW-TEST-FAILURES",
            "TEST-TIMEOUT",
            "NO-OUTPUT",
            "TIMEOUT",
            "FMT-DIRTY",
            "BUILD-FAILED", // -> BUILD-FAIL
            "TIMEOUT-NO-OUTPUT",
        ],
    );
    assert!(full.ok(), "{}", full.line());
    // Dropping one status fails the audit even though nothing is dead.
    let gap = audit_canonicalising_match_list(
        "status_triage.rs",
        &[
            "PASS",
            "NEW-TEST-FAILURES",
            "TEST-TIMEOUT",
            "NO-OUTPUT",
            "TIMEOUT",
            "BUILD-FAILED",
            "TIMEOUT-NO-OUTPUT",
        ],
    );
    assert!(!gap.ok(), "FMT-DIRTY must be matched");
    assert_eq!(gap.uncovered, vec!["FMT-DIRTY"], "{}", gap.line());
    // Resolving does not make a name the runner never writes matchable.
    let gate = audit_canonicalising_match_list("gate", &["UNKNOWN-NO-BASELINE"]);
    assert!(!gate.ok(), "a gate-only name is dead against runner output");
    assert!(gate
        .dead
        .iter()
        .any(|d| d.starts_with("UNKNOWN-NO-BASELINE")));
}

#[test]
fn failed_run_statuses_are_declared_and_canonical() {
    // The one shared list that already existed must be a subset of the vocabulary,
    // spelled the way the runner spells it.
    for name in autospec_core::dispatch_guard::FAILED_RUN_STATUSES {
        assert!(is_declared(name), "{name} is not in {VOCABULARY_PATH}");
        assert_eq!(
            canonical_status(name).map(|s| s.as_str()),
            Some(*name),
            "{name} is not the runner's own spelling"
        );
        assert!(
            emitted().contains(name),
            "{name} is not a status the runner emits"
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 2: a verdict is derived, never written alongside its fields.
// ---------------------------------------------------------------------------

#[test]
fn green_is_only_derivable_from_three_recorded_zeroes() {
    assert_eq!(
        RunEvidence::new(0, 0, 0).derive(),
        Derivation::Status(Status::Verified)
    );
    // A stage that recorded nothing says nothing, so it cannot support VERIFIED.
    let partial = RunEvidence {
        build_rc: Some(0),
        test_rc: Some(0),
        fmt_rc: None,
    };
    assert!(matches!(partial.derive(), Derivation::Insufficient(_)));
    assert_eq!(partial.missing(), vec!["fmt_rc"]);
    assert_eq!(
        autospec_core::run_status::check_verdict(Some("VERIFIED"), &partial),
        VerdictCheck::Undecidable {
            recorded: Status::Verified,
            missing: vec!["fmt_rc"],
        }
    );
}

#[test]
fn the_most_decisive_evidence_wins() {
    // fmt, then build, then tests: the same precedence triage uses.
    assert_eq!(
        RunEvidence::new(1, 1, 1).derive(),
        Derivation::Status(Status::FmtDirty)
    );
    assert_eq!(
        RunEvidence::new(1, 1, 0).derive(),
        Derivation::Status(Status::BuildFail)
    );
    assert_eq!(
        RunEvidence::new(0, 101, 0).derive(),
        Derivation::Status(Status::NewTestFailures)
    );
}

#[test]
fn a_verified_verdict_over_a_failing_test_rc_is_a_contradiction() {
    // The 183/183 regression: the record said VERIFIED and its own field said 101.
    let check =
        autospec_core::run_status::check_verdict(Some("VERIFIED"), &RunEvidence::new(0, 101, 0));
    match &check {
        VerdictCheck::Contradiction {
            recorded,
            derived,
            evidence,
        } => {
            assert_eq!(*recorded, Status::Verified);
            assert_eq!(*derived, Status::NewTestFailures);
            assert_eq!(evidence, "build_rc=0 test_rc=101 fmt_rc=0");
        }
        other => panic!("expected a contradiction, got {other:?}"),
    }
    assert!(check.is_defect());
    let line = check.line();
    assert!(line.contains("contradicted"), "{line}");
    assert!(line.contains("test_rc=101"), "{line}");
}

#[test]
fn an_alias_verdict_is_checked_as_the_status_it_means() {
    // PASS over a failing test run is the same contradiction, not a new category.
    let check = autospec_core::run_status::check_verdict(Some("PASS"), &RunEvidence::new(0, 1, 0));
    assert!(matches!(
        check,
        VerdictCheck::Contradiction {
            recorded: Status::Verified,
            derived: Status::NewTestFailures,
            ..
        }
    ));
}

#[test]
fn a_timeout_verdict_is_unverifiable_not_a_contradiction() {
    // Exit codes cannot speak about a run that never finished: unverifiable, and
    // not a defect. An invented name is a defect of a different kind.
    let evidence = RunEvidence::default();
    assert_eq!(
        autospec_core::run_status::check_verdict(Some("TIMEOUT"), &evidence),
        VerdictCheck::Unverifiable {
            recorded: Status::Timeout,
            reason: "a negative verdict needs the failing stage's exit code to be checkable",
        }
    );
    assert!(!check_is_defect(Some("TIMEOUT"), &evidence));
    assert!(check_is_defect(Some("BUILD-FAILDED"), &evidence));
    assert_eq!(
        autospec_core::run_status::check_verdict(Some("BUILD-FAILDED"), &evidence),
        VerdictCheck::Undeclared {
            recorded: "BUILD-FAILDED".to_string(),
        }
    );
    assert_eq!(
        autospec_core::run_status::check_verdict(None, &evidence),
        VerdictCheck::NoVerdict
    );
}

fn check_is_defect(recorded: Option<&str>, evidence: &RunEvidence) -> bool {
    autospec_core::run_status::check_verdict(recorded, evidence).is_defect()
}

// ---------------------------------------------------------------------------
// Invariant 3: a constant label carries no information, and says so.
// ---------------------------------------------------------------------------

#[test]
fn a_constant_label_over_a_population_is_degenerate() {
    let one = label_distribution(["VERIFIED"]);
    assert!(
        !one.is_degenerate(),
        "one record is one record, not a constant"
    );
    let flat = label_distribution(["VERIFIED", "VERIFIED"]);
    assert!(flat.is_degenerate());
    assert!(flat.line().contains("separates nothing"), "{}", flat.line());
    let varied = label_distribution(["VERIFIED", "BUILD-FAIL"]);
    assert!(!varied.is_degenerate());
    // Aliases count separately: the same status spelled two ways is itself news.
    let spelled_two_ways = label_distribution(["VERIFIED", "PASS", "VERIFIED"]);
    assert_eq!(spelled_two_ways.distinct(), 2);
    assert_eq!(spelled_two_ways.count_of("PASS"), 1);
    assert_eq!(spelled_two_ways.count_of("BUILD-FAIL"), 0);
    assert_eq!(spelled_two_ways.total, 3);
}

#[test]
fn the_cluster_population_is_untrustworthy() {
    // 183 runs, every one VERIFIED, every one test_rc=101. Either half alone reads
    // as plausible; together they mean the verdict is not evidence.
    let records: Vec<RunRecord> = (0..183)
        .map(|_| RunRecord::new("VERIFIED", 0, 101, 0))
        .collect();
    let audit = audit_population(&records);
    assert_eq!(audit.total, 183);
    assert_eq!(audit.consistent, 0);
    assert_eq!(audit.defects.len(), 183);
    assert!(audit.distribution.is_degenerate());
    assert!(!audit.trustworthy());
    assert!(audit.line().starts_with("WARN"), "{}", audit.line());
    assert!(
        audit.line().contains("label is constant"),
        "{}",
        audit.line()
    );
}

#[test]
fn a_trustworthy_population_is_audited_as_ok() {
    let records = vec![
        RunRecord::new("VERIFIED", 0, 0, 0),
        RunRecord::new("BUILD-FAIL", 1, 0, 0),
        RunRecord::new("NEW-TEST-FAILURES", 0, 1, 0),
    ];
    let audit = audit_population(&records);
    assert_eq!(audit.consistent, 3);
    assert!(audit.defects.is_empty());
    assert!(audit.trustworthy());
    assert!(audit.line().starts_with("ok"), "{}", audit.line());
}

// ---------------------------------------------------------------------------
// The vocabulary is the definition consumers are checked against.
// ---------------------------------------------------------------------------

/// Every status name a source file quotes verbatim. A scanner rather than a
/// review note: the defect was a list nobody checked against the vocabulary.
fn quoted_status_shaped_literals(path: &str) -> Vec<String> {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    scan_shaped(&text)
}

/// Names that look like a status and are not one, each with the reason.
const NON_STATUS_LITERALS: &[(&str, &str)] = &[
    (
        "AGENT-REPORTED-UNBUILT",
        "hold-reason token, not a run status",
    ),
    (
        "AGENT-REPORTED-UNFORMATTED",
        "hold-reason token, not a run status",
    ),
    ("INFRA-FAIL", "artifact class, not a run status"),
    ("LAUNCH-FAIL", "artifact class, not a run status"),
];

#[test]
fn no_decision_consumer_invents_a_status_name() {
    // Invariant 4: the vocabulary has one definition, and every consumer that
    // compares against a name is checked here. A name that is neither declared nor
    // explained below is a second, undocumented source of truth.
    for path in [
        "src/execution/status_triage.rs",
        "src/conversion_gate.rs",
        "src/dispatch_guard.rs",
        "../autospec-cli/src/commands/dispatch.rs",
    ] {
        for name in quoted_status_shaped_literals(path) {
            if is_declared(&name) {
                continue;
            }
            assert!(
                NON_STATUS_LITERALS.iter().any(|(n, _)| *n == name),
                "{path} matches \"{name}\", which {VOCABULARY_PATH} does not declare \
                 and which is not listed as a non-status token: add the row, or match \
                 through canonical_status"
            );
        }
    }
}

#[test]
fn the_scanner_detects_an_invented_name() {
    // The scan above is only a guard if it can fail. Here is the #4206 spelling
    // written into a file that is not allowed to invent one.
    let dir = std::env::temp_dir().join(format!("run-status-scan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("guard.rs");
    std::fs::write(
        &file,
        r#"fn decide(s: &str) { match s { Some("BUILD-FAILDED") => {}, _ => {} } }"#,
    )
    .unwrap();
    let found = scan_shaped(&std::fs::read_to_string(&file).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(found, vec!["BUILD-FAILDED".to_string()]);
    assert!(!is_declared("BUILD-FAILDED"));
}

fn scan_shaped(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != '"' {
            i += 1;
            continue;
        }
        let mut token = String::new();
        let mut j = i + 1;
        let mut closed = false;
        while j < bytes.len() {
            match bytes[j] {
                '\\' => {
                    token.push('\\');
                    j += 2;
                }
                '"' => {
                    closed = true;
                    break;
                }
                '\n' => break,
                c => {
                    token.push(c);
                    j += 1;
                }
            }
        }
        if !closed {
            i += 1;
            continue;
        }
        let shaped = token.len() > 3
            && token.contains('-')
            && token.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && token
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-');
        if shaped {
            found.push(token);
        }
        i = j + 1;
    }
    found
}

// ---------------------------------------------------------------------------
// The triage consumer, routed through the vocabulary.
// ---------------------------------------------------------------------------

use autospec_core::execution::status_triage::{
    triage, AgentHoldReason, AgentReport, GateBasis, TriageDecision,
};

fn report(
    status: Option<&str>,
    build_rc: Option<i32>,
    test_rc: Option<i32>,
    fmt_rc: Option<i32>,
) -> AgentReport {
    AgentReport {
        status: status.map(str::to_string),
        build_rc,
        test_rc,
        fmt_rc,
        fmt_files: None,
    }
}

#[test]
fn triage_routes_the_statuses_the_runner_actually_writes() {
    // Each of these names was previously unknown to triage: the run fell through
    // to the green arm, which ran the local gate over an unbuilt tree.
    assert_eq!(
        triage(&report(Some("BUILD-FAIL"), Some(1), None, Some(0))),
        TriageDecision::Hold {
            reason: AgentHoldReason::Unbuilt
        }
    );
    // FMT-DIRTY with a clean build: the recorded verdict is not trusted over a
    // local repair the stage can run itself (#4099). The caller formats and
    // re-checks before judging.
    assert_eq!(
        triage(&report(Some("FMT-DIRTY"), Some(0), Some(0), Some(1))),
        TriageDecision::FormatAndRecheck
    );
    assert_eq!(
        triage(&report(Some("NO-OUTPUT"), None, None, None)),
        TriageDecision::RaiseForReview {
            status: "NO-OUTPUT".to_string()
        }
    );
    assert_eq!(
        triage(&report(Some("TEST-TIMEOUT"), Some(0), None, Some(0))),
        TriageDecision::GateLocally {
            basis: GateBasis::AgentReportedTestFailure {
                status: Some("TEST-TIMEOUT".to_string())
            }
        }
    );
}

#[test]
fn triage_reaches_a_legacy_spelling_through_the_vocabulary() {
    // The old name must route to the same decision as the runner's own.
    let legacy = triage(&report(Some("BUILD-FAILED"), None, None, None));
    let current = triage(&report(Some("BUILD-FAIL"), None, None, None));
    assert_eq!(legacy, current);
    assert_eq!(
        legacy,
        TriageDecision::Hold {
            reason: AgentHoldReason::Unbuilt
        }
    );
    assert_eq!(
        triage(&report(Some("UNKNOWN-NO-FMT-BASELINE"), None, None, None)),
        TriageDecision::GateLocally {
            basis: GateBasis::NoBaseline
        }
    );
}

#[test]
fn a_verified_report_with_a_failing_test_rc_triages_on_the_code() {
    // The word loses to the exit code: a gate is run, the run is not held.
    let d = triage(&report(Some("VERIFIED"), Some(0), Some(101), Some(0)));
    assert_eq!(
        d,
        TriageDecision::GateLocally {
            basis: GateBasis::AgentReportedTestFailure {
                status: Some("VERIFIED".to_string())
            }
        }
    );
    // A no-baseline verdict keeps its own rule even beside a failing test_rc.
    assert_eq!(
        triage(&report(
            Some("UNKNOWN-NO-BASELINE"),
            Some(0),
            Some(1),
            Some(0)
        )),
        TriageDecision::GateLocally {
            basis: GateBasis::NoBaseline
        }
    );
    // Truly green still goes to the local gate.
    assert_eq!(
        triage(&report(Some("VERIFIED"), Some(0), Some(0), Some(0))),
        TriageDecision::GateLocally {
            basis: GateBasis::AgentGreen
        }
    );
}

#[test]
fn every_emitted_status_has_a_triage_route() {
    // No status may fall through to a default: the fall-through is what made the
    // unbuilt runs look convertible.
    for name in emitted() {
        let d = triage(&report(Some(name), None, None, None));
        match d {
            TriageDecision::Redispatch { .. }
            | TriageDecision::RaiseForReview { .. }
            | TriageDecision::Hold { .. }
            | TriageDecision::FormatAndRecheck
            | TriageDecision::GateLocally { .. } => {}
        }
    }
}
