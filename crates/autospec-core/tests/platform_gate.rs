//! Regression tests for `autospec_core::platform_gate` (issue #4312).
//!
//! The incident the invariants were extracted from: a macOS-gated test file
//! (#4306) with five compile errors sat merged and hidden because the
//! `macos-test` job of the `rust-suites` workflow had been red 0/40 — a
//! platform job that is the *sole* verification of the code behind its cfg
//! gate, and a pass rate that was all-failing and conspicuous on its own if
//! anyone looked. #4311 repeated the same shape on Windows. These tests
//! reconstruct the configuration the bug required: an all-failing rate, a
//! red platform job, and a patch touching a macOS-gated surface on a Linux
//! host.

use autospec_core::merge_gate::PassRate;
use autospec_core::platform_gate::{
    classify_failure, local_authority, parse_platform_predicate, patch_surfaces, rate_alarm,
    GateFailureKind, Platform, PlatformDeferral, RateAlarm,
};
use std::collections::BTreeMap;

/// The `rust-suites` job map: each platform and the job that solely
/// verifies it.
fn jobs() -> BTreeMap<Platform, String> {
    let mut m = BTreeMap::new();
    m.insert(Platform::Linux, "build-test".to_owned());
    m.insert(Platform::Macos, "macos-test".to_owned());
    m.insert(Platform::Windows, "windows-test".to_owned());
    m.insert(Platform::FreeBSD, "freebsd-test".to_owned());
    m
}

/// The #4306-shaped patch: adds a macOS-gated module.
const MACOS_PATCH: &str = "\
diff --git a/src/commit_rust.rs b/src/commit_rust.rs
index 1111111..2222222 100644
--- a/src/commit_rust.rs
+++ b/src/commit_rust.rs
@@ -10,6 +10,10 @@
 context line
+#[cfg(target_os = \"macos\")]
+mod macos_only {
+    fn do_new_thing() {}
+}
";

/// A patch that touches a macOS-gated module only via context lines.
const CONTEXT_PATCH: &str = "\
--- a/src/commit_rust.rs
+++ b/src/commit_rust.rs
@@ -1,3 +1,4 @@
 #[cfg(target_os = \"macos\")]
 mod macos_only {
+    fn do_new_thing() {}
 }
";

/// A patch that deletes a Windows-gated line.
const DELETED_PATCH: &str = "\
--- a/src/win_only.rs
+++ b/src/win_only.rs
@@ -1,3 +1,2 @@
-#[cfg(target_os = \"windows\")]
 mod w {
     fn f() {}
";

/// A patch touching only non-platform cfg surfaces.
const NON_PLATFORM_PATCH: &str = "\
--- a/src/x.rs
+++ b/src/x.rs
@@ -1,3 +1,5 @@
+#[cfg(feature = \"serde\")]
+pub mod serde_shim {}
+#[cfg(target_arch = \"x86_64\")]
+pub mod simd_path {}
";

/// A patch with no cfg surface at all.
const PLAIN_PATCH: &str = "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,3 @@
 fn main() {
+    let x = 1;
     let y = 2;
";

/// Extract `(predicate, job names)` pairs from a Hold, for assertions.
fn holds_for(d: &PlatformDeferral) -> Vec<(String, Vec<String>)> {
    match d {
        PlatformDeferral::Hold { surfaces, .. } => surfaces
            .iter()
            .map(|s| (s.predicate.clone(), s.jobs.clone()))
            .collect(),
        PlatformDeferral::None => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Invariant 3: the local gate is not authority for surfaces it cannot compile
// ---------------------------------------------------------------------------

#[test]
fn macos_gated_patch_on_linux_host_holds_for_the_macos_job() {
    let d = local_authority(Platform::Linux, &jobs(), MACOS_PATCH);
    assert!(
        d.must_hold(),
        "a patch adding a macOS-gated module on Linux must hold"
    );
    assert_eq!(
        holds_for(&d),
        vec![(
            "target_os = \"macos\"".to_owned(),
            vec!["macos-test".to_owned()]
        )]
    );
}

#[test]
fn patch_touching_a_surface_the_host_compiles_never_holds() {
    // A patch that only adds a linux-gated module compiles on the linux host.
    let patch = format!(
        "+#[cfg({})]\n+mod only_here {{}}\n",
        "target_os = \"linux\""
    );
    let d = local_authority(Platform::Linux, &jobs(), &patch);
    assert!(
        !d.must_hold(),
        "host-compilable surface must not hold: {d:?}"
    );
}

#[test]
fn context_line_touch_counts_as_touching_the_surface() {
    let d = local_authority(Platform::Linux, &jobs(), CONTEXT_PATCH);
    assert!(
        d.must_hold(),
        "context-line touch of a macOS-gated module must hold"
    );
    assert_eq!(
        holds_for(&d),
        vec![(
            "target_os = \"macos\"".to_owned(),
            vec!["macos-test".to_owned()]
        )]
    );
}

#[test]
fn deleted_gated_line_counts_as_touching_the_surface() {
    let d = local_authority(Platform::Linux, &jobs(), DELETED_PATCH);
    assert!(d.must_hold(), "deleting a windows-gated line must hold");
    assert_eq!(
        holds_for(&d),
        vec![(
            "target_os = \"windows\"".to_owned(),
            vec!["windows-test".to_owned()]
        )]
    );
}

#[test]
fn plain_patch_never_holds() {
    let d = local_authority(Platform::Linux, &jobs(), PLAIN_PATCH);
    assert!(!d.must_hold(), "a patch with no cfg surface must not hold");
}

#[test]
fn deferral_line_names_host_and_jobs() {
    let d = local_authority(Platform::Linux, &jobs(), MACOS_PATCH);
    let line = d.line();
    assert!(line.starts_with("hold:"), "deferral line: {line}");
    assert!(line.contains("linux"), "line names the host: {line}");
    assert!(
        line.contains("target_os = \"macos\""),
        "line names the predicate: {line}"
    );
    assert!(line.contains("macos-test"), "line names the job: {line}");

    let plain = local_authority(Platform::Linux, &jobs(), PLAIN_PATCH);
    assert!(
        plain.line().starts_with("no platform deferral"),
        "plain line: {}",
        plain.line()
    );
}

#[test]
fn job_names_are_resolved_from_the_map_not_the_default() {
    let mut m = jobs();
    m.insert(Platform::Macos, "macos-latest-suite".to_owned());
    let d = local_authority(Platform::Linux, &m, MACOS_PATCH);
    assert_eq!(
        holds_for(&d),
        vec![(
            "target_os = \"macos\"".to_owned(),
            vec!["macos-latest-suite".to_owned()]
        )],
        "deferral must carry the renamed job from the map"
    );
}

// ---------------------------------------------------------------------------
// Invariant 1: a platform job's failure is a coverage loss, not a flaky check
// ---------------------------------------------------------------------------

#[test]
fn platform_job_failure_is_coverage_loss() {
    match classify_failure("macos-test", &jobs()) {
        GateFailureKind::CoverageLoss { job, platform } => {
            assert_eq!(job, "macos-test");
            assert_eq!(platform, Platform::Macos);
        }
        other => panic!("macos-test must classify as CoverageLoss, got {other:?}"),
    }
}

#[test]
fn ordinary_job_failure_is_ordinary() {
    // main-builds is a linux job not in the platform map: an ordinary flake.
    match classify_failure("main-builds", &jobs()) {
        GateFailureKind::Ordinary { job } => assert_eq!(job, "main-builds"),
        other => panic!("main-builds must classify as Ordinary, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Invariant 2: the alarm fires on the rate, not on individual runs
// ---------------------------------------------------------------------------

#[test]
fn all_failing_rate_alarms() {
    let mut rate = PassRate::new("rust-suites", "main");
    for _ in 0..40 {
        rate.record(false);
    }
    assert_eq!(rate_alarm(&rate), RateAlarm::AllFailing, "0/40 must alarm");
    let line = RateAlarm::AllFailing.line(&rate, Some(Platform::Macos));
    assert!(line.starts_with("ALARM:"), "alarm line: {line}");
    assert!(line.contains("0/40"), "alarm names the rate: {line}");
    assert!(
        line.contains("macos") && line.contains("total loss of coverage"),
        "alarm names the lost coverage: {line}"
    );
}

#[test]
fn partial_rate_is_quiet() {
    let mut rate = PassRate::new("rust-suites", "main");
    for _ in 0..39 {
        rate.record(false);
    }
    rate.record(true);
    assert_eq!(rate_alarm(&rate), RateAlarm::Quiet, "39/40 must not alarm");
}

#[test]
fn empty_rate_is_quiet_unknown_not_red() {
    let rate = PassRate::new("rust-suites", "main");
    assert_eq!(
        rate_alarm(&rate),
        RateAlarm::Quiet,
        "0/0 is unknown, not red"
    );
}

// ---------------------------------------------------------------------------
// The predicate parser claims only what it can classify (invariant 5)
// ---------------------------------------------------------------------------

#[test]
fn parse_classifies_each_supported_form() {
    let cases = [
        ("target_os = \"linux\"", vec![Platform::Linux]),
        ("target_os = \"macos\"", vec![Platform::Macos]),
        ("target_os = \"windows\"", vec![Platform::Windows]),
        ("target_os = \"freebsd\"", vec![Platform::FreeBSD]),
        (
            "target_family = \"unix\"",
            vec![Platform::Linux, Platform::Macos, Platform::FreeBSD],
        ),
        ("target_family = \"windows\"", vec![Platform::Windows]),
        (
            "unix",
            vec![Platform::Linux, Platform::Macos, Platform::FreeBSD],
        ),
        ("windows", vec![Platform::Windows]),
        ("not(unix)", vec![Platform::Windows]),
        (
            "not(windows)",
            vec![Platform::Linux, Platform::Macos, Platform::FreeBSD],
        ),
        (
            "not(target_os = \"macos\")",
            vec![Platform::Linux, Platform::Windows, Platform::FreeBSD],
        ),
    ];
    for (pred, expected) in cases {
        let s = parse_platform_predicate(pred)
            .unwrap_or_else(|| panic!("predicate {pred:?} must classify"));
        assert_eq!(s.predicate, pred);
        assert_eq!(s.compiles_on, expected, "predicate {pred:?}");
    }
}

#[test]
fn parse_returns_none_for_what_it_cannot_classify() {
    let cases = [
        "feature = \"serde\"",
        "target_arch = \"x86_64\"",
        "target_os = \"ios\"",
        "any(unix, windows)",
        "all(unix, not(windows))",
        "unix and windows",
        "not(any(unix, windows))",
        "not(unix",
        "",
    ];
    for pred in cases {
        assert!(
            parse_platform_predicate(pred).is_none(),
            "predicate {pred:?} must NOT classify (invariant 5)"
        );
    }
}

#[test]
fn patch_surfaces_dedupes_and_preserves_first_seen_order() {
    let patch = "\
+#[cfg(unix)]
+mod a {}
 #[cfg(unix)]
 mod b {}
+#[cfg(windows)]
+mod c {}
";
    let surfaces = patch_surfaces(patch);
    let preds: Vec<&str> = surfaces.iter().map(|s| s.predicate.as_str()).collect();
    assert_eq!(preds, vec!["unix", "windows"], "deduped, first-seen order");
}

#[test]
fn cfg_macro_invocation_is_not_a_surface() {
    // cfg!(...) is a runtime branch, not a compile-time surface.
    let patch = "+    if cfg!(target_os = \"macos\") {\n+        return;\n+    }\n";
    assert!(
        patch_surfaces(patch).is_empty(),
        "cfg!() invocation must not count as a surface"
    );
}

// ---------------------------------------------------------------------------
// Incident reconstruction: a red platform job cannot be ignored
// ---------------------------------------------------------------------------

#[test]
fn incident_reconstruction_red_platform_job_cannot_be_ignored() {
    // The gate has been red 0/40 on the default branch.
    let mut rate = PassRate::new("rust-suites", "main");
    for _ in 0..40 {
        rate.record(false);
    }
    // Invariant 2: the rate alone is loud.
    let alarm = rate_alarm(&rate);
    assert_eq!(alarm, RateAlarm::AllFailing);
    assert!(alarm
        .line(&rate, Some(Platform::Macos))
        .starts_with("ALARM:"));

    // Invariant 1: the red job is the sole verification of the macOS surface.
    match classify_failure("macos-test", &jobs()) {
        GateFailureKind::CoverageLoss { platform, .. } => {
            assert_eq!(platform, Platform::Macos);
        }
        other => panic!("macos-test must be a coverage loss, got {other:?}"),
    }

    // Invariant 3: the converter's local gate (Linux) must not authorise a
    // merge of a patch touching a macOS-gated surface.
    let d = local_authority(Platform::Linux, &jobs(), MACOS_PATCH);
    assert!(d.must_hold());
    assert_eq!(
        holds_for(&d),
        vec![(
            "target_os = \"macos\"".to_owned(),
            vec!["macos-test".to_owned()]
        )]
    );

    // Control: a patch with no platform surface never holds, and a non-
    // platform job's failure is ordinary.
    assert!(!local_authority(Platform::Linux, &jobs(), PLAIN_PATCH).must_hold());
    match classify_failure("main-builds", &jobs()) {
        GateFailureKind::Ordinary { .. } => {}
        other => panic!("main-builds must be ordinary, got {other:?}"),
    }
}
