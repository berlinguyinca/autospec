//! Negative evidence and tool coverage (issue #4344).
//!
//! The incident: four `gh` code-search queries over a private repository
//! each returned zero hits — `'v1/models'`, `'chat/completions'`, `axum`,
//! `TcpListener` — consistent with each other and with the hypothesis that
//! the repository had no HTTP server. The conclusion was about to be that a
//! second gateway should be built in the *other* repository, beside one that
//! already existed. The repository in fact contains `openai_api.rs`,
//! `proxy.rs`, `daemon.rs`, `admin_http.rs`, and a `[[bin]]`. GitHub does
//! not index private repositories for code search, and returns zero rather
//! than an error — so an unavailable index and an empty repository produce
//! the same answer.
//!
//! The regression tests run in the configuration the bug required: four
//! consistent zeros from one search tool over a source it does not index.
//! The controls: the same zeros answered by a directory listing hold, and
//! the same zeros through a tool a control proves can produce a positive
//! hold.

use autospec_core::negative_evidence::{
    classify, Control, CoverageLimit, GroupVerdict, HoldsBasis, Method, NegativeReport, Observation,
};

/// The incident: four zeros from GitHub code search over a private
/// repository, all Search, none controlled.
fn incident_zeros() -> Vec<Observation> {
    vec![
        zero("gh code search", "acme/gateway", "v1/models"),
        zero("gh code search", "acme/gateway", "chat/completions"),
        zero("gh code search", "acme/gateway", "axum"),
        zero("gh code search", "acme/gateway", "TcpListener"),
    ]
}

fn zero(tool: &str, source: &str, query: &str) -> Observation {
    Observation {
        tool: tool.into(),
        source: source.into(),
        query: query.into(),
        method: Method::Search,
    }
}

/// The remedy the directory listing provides: the source enumerated
/// directly, where it is present or it errors.
fn incident_listing() -> Vec<Observation> {
    vec![
        list("ls", "acme/gateway/src", "openai_api.rs"),
        list("ls", "acme/gateway/src", "proxy.rs"),
        list("ls", "acme/gateway/src", "daemon.rs"),
        list("ls", "acme/gateway/src", "admin_http.rs"),
    ]
}

fn list(tool: &str, source: &str, query: &str) -> Observation {
    Observation {
        tool: tool.into(),
        source: source.into(),
        query: query.into(),
        method: Method::Enumeration,
    }
}

/// The control: a term certain to be in the source, run through the same
/// tool. Over the private repository it returns zero — the index is the
/// finding.
fn zero_control() -> Control {
    Control {
        tool: "gh code search".into(),
        source: "acme/gateway".into(),
        query: "openai_api".into(),
        hits: 0,
    }
}

/// The same control through a tool that does index the source: it returns
/// a positive, and the negatives it sits beside hold.
fn positive_control() -> Control {
    Control {
        tool: "rg".into(),
        source: "acme/gateway".into(),
        query: "Cargo.toml".into(),
        hits: 1,
    }
}

#[test]
fn the_incident_four_consistent_zeros_are_one_untrusted_observation() {
    let report = classify(&incident_zeros(), &[]);

    // Four queries, one tool, one source: one observation, not four.
    assert_eq!(report.weight(), 1);
    assert!(!report.groups().is_empty());
    assert_eq!(report.groups().len(), 1);

    // Not evidence of absence: the investigation must not terminate.
    assert!(report.any_untrusted());
    match &report.groups()[0] {
        GroupVerdict::CoverageUnverified {
            tool,
            source,
            observations,
        } => {
            assert_eq!(tool, "gh code search");
            assert_eq!(source, "acme/gateway");
            assert_eq!(*observations, 4);
        }
        other => panic!("expected CoverageUnverified, got {other:?}"),
    }

    let line = &report.lines()[0];
    assert!(line.starts_with("WARN:"), "{line}");
    assert!(line.contains("no positive control"), "{line}");
    assert!(!line.contains("hold"), "{line}");
}

#[test]
fn the_control_the_investigator_never_ran_makes_the_index_the_finding() {
    let controls = vec![zero_control()];
    let report = classify(&incident_zeros(), &controls);

    assert!(report.any_untrusted());
    match &report.groups()[0] {
        GroupVerdict::ToolIsTheFinding {
            tool,
            source,
            control,
            observations,
        } => {
            assert_eq!(tool, "gh code search");
            assert_eq!(source, "acme/gateway");
            assert_eq!(control, "openai_api");
            assert_eq!(*observations, 4);
        }
        other => panic!("expected ToolIsTheFinding, got {other:?}"),
    }

    let line = &report.lines()[0];
    assert!(line.starts_with("FAIL:"), "{line}");
    assert!(line.contains("index is the finding"), "{line}");
    // The line itself names the weight error: four zeros are one observation.
    assert!(line.contains("they are one observation, not 4"), "{line}");
}

#[test]
fn the_directory_listing_the_investigator_never_read_makes_the_negative_hold() {
    let report = classify(&incident_listing(), &[]);

    assert!(!report.any_untrusted());
    match &report.groups()[0] {
        GroupVerdict::Holds {
            basis,
            observations,
            ..
        } => {
            assert_eq!(*basis, HoldsBasis::Enumerated);
            assert_eq!(*observations, 4);
        }
        other => panic!("expected Holds(Enumerated), got {other:?}"),
    }
    let line = &report.lines()[0];
    assert!(line.starts_with("OK:"), "{line}");
    assert!(line.contains("enumerated directly"), "{line}");
}

#[test]
fn a_search_whose_control_returns_positive_holds() {
    let zeros = vec![
        zero("rg", "acme/gateway", "axum"),
        zero("rg", "acme/gateway", "TcpListener"),
    ];
    let report = classify(&zeros, &[positive_control()]);

    assert!(!report.any_untrusted());
    match &report.groups()[0] {
        GroupVerdict::Holds { basis, .. } => assert_eq!(*basis, HoldsBasis::Controlled),
        other => panic!("expected Holds(Controlled), got {other:?}"),
    }
}

#[test]
fn a_control_over_another_source_covers_nothing() {
    // The control must run over the same source a negative depends on: a
    // positive elsewhere says nothing about this repository.
    let other = Control {
        tool: "gh code search".into(),
        source: "acme/public-repo".into(),
        query: "README".into(),
        hits: 3,
    };
    let report = classify(&incident_zeros(), &[other]);
    assert!(report.any_untrusted());
    assert!(matches!(
        report.groups()[0],
        GroupVerdict::CoverageUnverified { .. }
    ));
}

#[test]
fn zeros_from_two_tools_are_two_observations() {
    // Independent tools can fail independently — but only if each is
    // controlled on its own terms.
    let zeros = vec![
        zero("gh code search", "acme/gateway", "axum"),
        zero("rg", "acme/gateway", "axum"),
    ];
    let report = classify(&zeros, &[positive_control()]);
    assert_eq!(report.weight(), 2);
    assert!(report.any_untrusted());
    let lines = report.lines();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().any(|l| l.starts_with("OK:")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("WARN:")), "{lines:?}");
}

#[test]
fn the_coverage_limit_is_recorded_where_the_tool_will_be_read() {
    let limit = CoverageLimit::new(
        "gh code search",
        "does not index private repositories; returns zero, not an error",
    )
    .unwrap();
    assert_eq!(
        limit.line(),
        "coverage limit: gh code search — does not index private repositories; returns zero, not an error"
    );
    // A limit with an unnamed tool, or with no stated limit, is a
    // placeholder, not a record.
    assert!(CoverageLimit::new("", "returns zero, not an error").is_none());
    assert!(CoverageLimit::new("gh code search", "").is_none());
}

#[test]
fn the_incident_end_to_end_from_almost_duplicate_build_to_void_zeros() {
    // Reconstructed end to end: the investigator's four zeros, the control
    // that was never run, and the listing that was never read.
    let zeros = incident_zeros();
    let before = classify(&zeros, &[]);
    assert!(before.any_untrusted());
    assert_eq!(before.weight(), 1);

    let with_control = classify(&zeros, &[zero_control()]);
    assert!(with_control.any_untrusted());
    assert!(matches!(
        with_control.groups()[0],
        GroupVerdict::ToolIsTheFinding { .. }
    ));

    let listing = incident_listing();
    let after = classify(&listing, &[]);
    assert!(!after.any_untrusted());

    // The report the investigation should have produced: the same four
    // queries, the same tool, one observation — and it is void.
    let lines = with_control.lines();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("4 zero(s) are void"), "{}", lines[0]);
}

#[test]
fn a_zero_control_voids_the_whole_group_even_when_enumeration_was_tried() {
    // A tool that cannot see what it is told is in a source has proven
    // nothing about that source — the group is void as a whole.
    let zeros = vec![
        zero("gh code search", "acme/gateway", "axum"),
        list("gh code search", "acme/gateway", "src/"),
    ];
    let report = classify(&zeros, &[zero_control()]);
    assert!(report.any_untrusted());
    assert!(matches!(
        report.groups()[0],
        GroupVerdict::ToolIsTheFinding {
            observations: 2,
            ..
        }
    ));
    let line = &report.lines()[0];
    assert!(line.contains("2 zero(s) are void"), "{line}");
}

#[test]
fn methods_carry_labels_for_records() {
    assert_eq!(Method::Search.label(), "search");
    assert_eq!(Method::Enumeration.label(), "enumeration");
}

#[test]
fn empty_report_is_neutral() {
    let report: NegativeReport = classify(&[], &[]);
    assert!(!report.any_untrusted());
    assert_eq!(report.weight(), 0);
    assert!(report.lines().is_empty());
}
