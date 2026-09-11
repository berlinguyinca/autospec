//! Regression tests for `autospec_core::construction_sites` (issue #4348).
//!
//! The incident: while converting #4312, a field was added to
//! `PersistedInvocation` and every initializer a Linux `cargo build
//! --workspace --all-targets` could see was updated. One initializer lives
//! behind `#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]`,
//! so the next run red-flagged `macos-test` and `freebsd-test` while
//! `main-builds` stayed green — precisely the shape #4312's rule describes.
//! The rule was correct and freshly written; it did not fire because nothing
//! in the workflow asked the question. These tests reconstruct the
//! configuration the bug required: a shared struct with a construction site
//! gated to platforms other than the build host, and assert the check names
//! that site and both remedies.

use autospec_core::construction_sites::{
    invisible_to_host, predicate_platforms, visibility, ConstructionSite, SiteVisibility,
};
use autospec_core::platform_gate::Platform;

/// The #4348 incident: `PersistedInvocation` has seven construction sites in
/// the tree (a text search finds all of them). Six compile on the Linux build
/// host; one lives behind the platform gate and does not.
fn incident_sites() -> Vec<ConstructionSite> {
    // The ungated sites the Linux build can see.
    let mut sites = vec![
        ConstructionSite {
            location: "executor_bridge.rs:1340".into(),
            cfg: None,
        },
        ConstructionSite {
            location: "executor_bridge.rs:9689".into(),
            cfg: None,
        },
        ConstructionSite {
            location: "tests/support_invocation.rs:131".into(),
            cfg: None,
        },
        ConstructionSite {
            location: "tests/support_invocation.rs:180".into(),
            cfg: None,
        },
        ConstructionSite {
            location: "tests/runtime_fixture.rs:371".into(),
            cfg: None,
        },
        ConstructionSite {
            location: "tests/worktree_post.rs:414".into(),
            cfg: None,
        },
    ];
    // The one the Linux build cannot see: gated to the other platforms.
    sites.push(ConstructionSite {
        location: "portability/direct_attempt/tests.rs:51".into(),
        cfg: Some("any(target_os = \"macos\", target_os = \"freebsd\", windows)".into()),
    });
    sites
}

// ---------------------------------------------------------------------------
// Invariant 2: enumerate by text; the build's view is one platform's
// ---------------------------------------------------------------------------

#[test]
fn incident_site_is_invisible_to_the_linux_build() {
    let sites = incident_sites();
    assert_eq!(sites.len(), 7, "the text search found all seven sites");
    let v = visibility(Platform::Linux, "PersistedInvocation", &sites);
    assert!(
        v.must_hold(),
        "a platform-gated site must hold the local build's authority"
    );
    match &v {
        SiteVisibility::Hold {
            host,
            total,
            invisible,
            ..
        } => {
            assert_eq!(*host, Platform::Linux);
            assert_eq!(*total, 7);
            assert_eq!(invisible.len(), 1, "exactly one site is invisible to Linux");
            assert_eq!(
                invisible[0].location,
                "portability/direct_attempt/tests.rs:51"
            );
        }
        other => panic!("expected Hold, got {other:?}"),
    }
}

#[test]
fn invisible_to_host_returns_only_the_gated_sites() {
    let sites = incident_sites();
    let inv = invisible_to_host(Platform::Linux, &sites);
    assert_eq!(inv.len(), 1, "only the gated site is invisible to Linux");
    assert_eq!(inv[0].location, "portability/direct_attempt/tests.rs:51");
}

#[test]
fn on_a_host_the_site_is_gated_to_the_check_clears() {
    // The same seven sites, checked from macOS: the gated site compiles there,
    // so nothing holds.
    let sites = incident_sites();
    let v = visibility(Platform::Macos, "PersistedInvocation", &sites);
    assert!(
        !v.must_hold(),
        "on macos the gated site compiles, so nothing holds: {v:?}"
    );
    assert!(
        v.line().starts_with("no platform-invisible"),
        "line: {}",
        v.line()
    );
}

#[test]
fn all_visible_sites_clear() {
    let sites = vec![
        ConstructionSite {
            location: "a.rs:1".into(),
            cfg: None,
        },
        ConstructionSite {
            location: "b.rs:2".into(),
            cfg: None,
        },
    ];
    let v = visibility(Platform::Linux, "Foo", &sites);
    assert!(!v.must_hold(), "all-visible sites must clear: {v:?}");
    assert!(
        v.line().starts_with("no platform-invisible"),
        "line: {}",
        v.line()
    );
}

// ---------------------------------------------------------------------------
// Invariant 3: the check is also the procedure — it names both remedies
// ---------------------------------------------------------------------------

#[test]
fn hold_line_names_host_struct_site_and_both_remedies() {
    let v = visibility(Platform::Linux, "PersistedInvocation", &incident_sites());
    let line = v.line();
    assert!(line.starts_with("hold:"), "line: {line}");
    assert!(line.contains("linux"), "names the host: {line}");
    assert!(
        line.contains("PersistedInvocation"),
        "names the struct: {line}"
    );
    assert!(
        line.contains("portability/direct_attempt/tests.rs:51"),
        "names the site: {line}"
    );
    assert!(
        line.contains("grep 'PersistedInvocation {'"),
        "names the text remedy: {line}"
    );
    assert!(
        line.contains("widening the cfg to include the host and building"),
        "names the widen remedy: {line}"
    );
}

#[test]
fn hold_line_reports_the_denominator() {
    // The line always leads with how many of how many are invisible, so a
    // "1 of 7" is evidence the text search ran, not a bare "1".
    let line = visibility(Platform::Linux, "PersistedInvocation", &incident_sites()).line();
    assert!(
        line.contains("1 of 7"),
        "the denominator is reported: {line}"
    );
}

// ---------------------------------------------------------------------------
// The predicate evaluator claims only what it can classify
// ---------------------------------------------------------------------------

#[test]
fn predicate_classifies_platform_terms_and_compositions() {
    let cases = [
        ("target_os = \"linux\"", vec![Platform::Linux]),
        ("target_os = \"macos\"", vec![Platform::Macos]),
        (
            "any(target_os = \"macos\", target_os = \"freebsd\", windows)",
            // BTreeSet order is `Platform::Ord` (Linux, Macos, Windows,
            // FreeBSD), the same order the simple-term case returns.
            vec![Platform::Macos, Platform::Windows, Platform::FreeBSD],
        ),
        (
            "any(target_os = \"macos\", windows)",
            vec![Platform::Macos, Platform::Windows],
        ),
        // The real gate on `mod tests` (test is platform-neutral and skipped):
        // the platform scope is the intersection of the platform terms.
        (
            "all(test, any(target_os = \"macos\", target_os = \"freebsd\"))",
            vec![Platform::Macos, Platform::FreeBSD],
        ),
        (
            "not(target_os = \"macos\")",
            vec![Platform::Linux, Platform::Windows, Platform::FreeBSD],
        ),
        (
            "not(any(target_os = \"macos\", windows))",
            vec![Platform::Linux, Platform::FreeBSD],
        ),
        (
            "unix",
            vec![Platform::Linux, Platform::Macos, Platform::FreeBSD],
        ),
        ("windows", vec![Platform::Windows]),
    ];
    for (pred, expected) in cases {
        let got =
            predicate_platforms(pred).unwrap_or_else(|| panic!("predicate {pred:?} must classify"));
        assert_eq!(got, expected, "predicate {pred:?}");
    }
}

#[test]
fn predicate_returns_none_for_non_platform_gates() {
    let cases = [
        "feature = \"serde\"",
        "test",
        "target_arch = \"x86_64\"",
        "any(feature = \"a\", feature = \"b\")", // no platform term
        "all(feature = \"a\")",                  // no platform term
        "not(test)",                             // body is not a platform predicate
        "",
    ];
    for pred in cases {
        assert!(
            predicate_platforms(pred).is_none(),
            "predicate {pred:?} must NOT classify to a platform set"
        );
    }
}

// ---------------------------------------------------------------------------
// A platform scope is decided per host; ungated and host-gated sites never
// hold
// ---------------------------------------------------------------------------

#[test]
fn site_gated_to_the_host_is_visible() {
    let sites = vec![ConstructionSite {
        location: "a.rs:1".into(),
        cfg: Some("target_os = \"linux\"".into()),
    }];
    let v = visibility(Platform::Linux, "Foo", &sites);
    assert!(
        !v.must_hold(),
        "a host-gated site compiles on the host: {v:?}"
    );
}

#[test]
fn site_gated_to_another_platform_is_invisible() {
    let sites = vec![ConstructionSite {
        location: "a.rs:1".into(),
        cfg: Some("target_os = \"macos\"".into()),
    }];
    let v = visibility(Platform::Linux, "Foo", &sites);
    assert!(
        v.must_hold(),
        "a macos-gated site is invisible to a linux build: {v:?}"
    );
}

#[test]
fn a_not_gate_excludes_its_named_platform() {
    let sites = vec![ConstructionSite {
        location: "a.rs:1".into(),
        cfg: Some("not(target_os = \"linux\")".into()),
    }];
    assert!(
        visibility(Platform::Linux, "Foo", &sites).must_hold(),
        "not(linux) excludes the linux host"
    );
    assert!(
        !visibility(Platform::Macos, "Foo", &sites).must_hold(),
        "not(linux) compiles on macos"
    );
}

#[test]
fn a_pure_feature_gate_is_not_a_platform_restriction() {
    // A feature gate does not determine a platform set: the check is about
    // platform gates, so the site is not flagged.
    let sites = vec![ConstructionSite {
        location: "a.rs:1".into(),
        cfg: Some("feature = \"serde\"".into()),
    }];
    let v = visibility(Platform::Linux, "Foo", &sites);
    assert!(
        !v.must_hold(),
        "a feature-gated site is not a platform concern: {v:?}"
    );
}

#[test]
fn a_unix_gated_site_holds_only_on_windows() {
    let sites = vec![ConstructionSite {
        location: "a.rs:1".into(),
        cfg: None,
    }];
    // Control: an ungated site never holds, on any host.
    for host in [
        Platform::Linux,
        Platform::Macos,
        Platform::Windows,
        Platform::FreeBSD,
    ] {
        assert!(
            !visibility(host, "Foo", &sites).must_hold(),
            "an ungated site compiles on {host:?}"
        );
    }
    // A `unix`-gated site compiles on every host except Windows.
    let unix = vec![ConstructionSite {
        location: "a.rs:1".into(),
        cfg: Some("unix".into()),
    }];
    for host in [
        Platform::Linux,
        Platform::Macos,
        Platform::Windows,
        Platform::FreeBSD,
    ] {
        let holds = visibility(host, "Foo", &unix).must_hold();
        if host == Platform::Windows {
            assert!(holds, "unix excludes windows: {host:?}");
        } else {
            assert!(!holds, "unix compiles on {host:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Incident reconstruction: the rule that only a document enforced
// ---------------------------------------------------------------------------

#[test]
fn incident_reconstruction_the_build_sees_one_platform_the_text_sees_all() {
    // The build host is Linux. The text search (grep 'PersistedInvocation {')
    // finds seven sites; the build compiles six.
    let sites = incident_sites();
    let v = visibility(Platform::Linux, "PersistedInvocation", &sites);

    // Invariant 2: the difference between the text's view (7) and the
    // build's view (6) is exactly the site the build cannot see, reported
    // with its denominator.
    let line = v.line();
    assert!(line.contains("1 of 7"), "denominator reported: {line}");

    // Invariant 3: the check is also the procedure — it names both remedies.
    assert!(
        line.contains("grep 'PersistedInvocation {'"),
        "text remedy: {line}"
    );
    assert!(
        line.contains("widening the cfg to include the host and building"),
        "widen remedy: {line}"
    );

    // Control: on the platform the site is gated to, the build sees it and
    // the check clears — the hold is platform-specific, not a blanket block.
    let on_macos = visibility(Platform::Macos, "PersistedInvocation", &sites);
    assert!(
        !on_macos.must_hold(),
        "on macos the gated site compiles, so nothing holds: {on_macos:?}"
    );
}
