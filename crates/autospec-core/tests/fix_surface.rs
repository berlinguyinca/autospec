//! A fix is verified against the surface the defect lived on, not the diff
//! (#3905).
//!
//! The fixtures are the populated cases: the real #3903 patch (a new guard
//! script wired into a script in a different repository — a check that ran
//! nowhere) and the real #3904 repair (the step added to
//! `.github/workflows/rust.yml`, verified by reading the parsed workflow).
//! The positive cases pin the holds firing on the #3903 shape; the negative
//! cases pin them *not* firing once the invocation is visible — a check that
//! has only ever seen the empty case proves nothing either way.

use autospec_core::fix_surface::{
    is_fix_label, review_fix_surface, CheckAddition, FixSurfacePolicy, FixSurfaceReview, HoldKind,
    PatchRecord, HOLD_REASON,
};

fn kinds(review: &FixSurfaceReview) -> Vec<HoldKind> {
    review.holds.iter().map(|hold| hold.kind).collect()
}

/// The real #3903 patch: a new guard script, nothing else. Exercised positive
/// and negative, wired into a script in a different repository.
const DIFF_3903: &str = r#"diff --git a/scripts/check-catalog-count-parity.sh b/scripts/check-catalog-count-parity.sh
new file mode 100755
index 00000000..930cd1ee
--- /dev/null
+++ b/scripts/check-catalog-count-parity.sh
@@ -0,0 +1,64 @@
+#!/usr/bin/env bash
+# The validation catalog's size is asserted as a LITERAL in three test files
+# across two crates, so they cannot share a constant. Adding one check means
+# bumping every copy, and missing one breaks main in a way that then fails
+# every subsequent check-adding patch -- a cascade, not a single failure.
+#
+# That happened twice in one day. The first miss held seven patches falsely;
+# the fix measured the cost and filed it (#3892); an hour later a NINTH site
+# (crates/autospec-cli/tests/validation_parity.rs) was found the same way, and
+# held three more. A description of the hazard did not prevent the second
+# occurrence, so this is the mechanical control: divergence fails here, with
+# the file list, instead of surfacing as an unrelated-looking test failure.
+#
+# It asserts CONSISTENCY, not any particular value -- bumping the catalog is
+# expected and fine; bumping it in some copies and not others is the defect.
+set -uo pipefail
+cd "$(dirname "$0")/.."
+
+python3 - <<'PYCHK'
+import re, sys, collections
+
+# (file, regex capturing the literal, label) -- each label must agree everywhere.
+SITES = [
+    ("crates/autospec-core/tests/validation_runner.rs",  r'assert_eq!\(full\.ids\(\)\.len\(\),\s*(\d+)\)',            "full_plan"),
+    ("crates/autospec-core/tests/validation_runner.rs",  r'assert_eq!\(full\.unique_ids\(\)\.len\(\),\s*(\d+)\)',     "unique"),
+    ("crates/autospec-core/tests/validation_catalog.rs", r'assert_eq!\(calls\.len\(\),\s*(\d+)\)',                    "full_plan"),
+    ("crates/autospec-core/tests/validation_catalog.rs", r'BTreeSet<_>>\(\)\.len\(\),\s*(\d+)\)',                     "unique"),
+    ("crates/autospec-cli/tests/validation_parity.rs",   r'assert_eq!\(full\.ids\(\)\.len\(\),\s*(\d+)\)',            "full_plan"),
+    ("crates/autospec-cli/tests/validation_parity.rs",   r'assert_eq!\(full\.unique_ids\(\)\.len\(\),\s*(\d+)\)',     "unique"),
+]
+
+seen = collections.defaultdict(list)
+missing = []
+for path, pattern, label in SITES:
+    try:
+        text = open(path).read()
+    except FileNotFoundError:
+        missing.append(path); continue
+    m = re.search(pattern, text)
+    if not m:
+        missing.append(f"{path} :: {label}"); continue
+    seen[label].append((path, int(m.group(1))))
+
+bad = False
+for path in missing:
+    print(f"FAIL: catalog count site not found: {path}")
+    print("      the assertion moved or was renamed; update SITES in this script")
+    bad = True
+
+for label, entries in sorted(seen.items()):
+    values = {v for _, v in entries}
+    if len(values) > 1:
+        bad = True
+        print(f"FAIL: catalog '{label}' count disagrees across files: {sorted(values)}")
+        for path, v in entries:
+            print(f"        {v:>5}  {path}")
+        print("      adding a check means bumping EVERY copy; one missed copy breaks main")
+        print("      and then falsely holds every subsequent check-adding patch")
+
+if not bad:
+    summary = ", ".join(f"{label}={entries[0][1]}" for label, entries in sorted(seen.items()))
+    print(f"ok:   catalog count literals agree across {len(SITES)} sites ({summary})")
+sys.exit(1 if bad else 0)
+PYCHK
"#;

/// The real #3904 repair: the step that actually invokes the script, added to
/// the workflow of this repository.
const DIFF_3904: &str = r#"diff --git a/.github/workflows/rust.yml b/.github/workflows/rust.yml
index dd3b283b..3b5be73e 100644
--- a/.github/workflows/rust.yml
+++ b/.github/workflows/rust.yml
@@ -130,6 +130,13 @@ jobs:
         # ran, and `Validate repository` / `Build` below had not executed in CI for as long.
         run: cargo test --workspace --no-fail-fast

+      - name: Catalog count parity
+        # The catalog size is a literal in three test files across two crates.
+        # Bumping some and not others breaks main and then falsely holds every
+        # subsequent check-adding patch. Twice in one day (#3891/#3892 held
+        # seven patches, #3902 held three). Cheap, so it runs before validate.
+        run: bash scripts/check-catalog-count-parity.sh
+
       - name: Validate repository
         run: cargo run -p autospec-cli -- validate
"#;

/// #3903 as it landed: script only, no registration in this repository.
const RECORD_3903: &str =
    "validate: make the catalog-count divergence mechanical, not documented (#3903)";

/// The real #3903 commit body. It exercises the script ("Exercised both
/// ways"), and it even names the script in prose — but neither is an
/// `Invocation read-back:` block, and nothing in the patch registers it.
const BODY_3903: &str = r#"The catalog's size is asserted as a LITERAL in three test files across two
crates, so they cannot share a constant. Adding one check means bumping every
copy, and missing one breaks main in a way that then falsely holds every
subsequent check-adding patch -- a cascade, not a single failure.

That happened twice today from one root. The first miss held seven patches
falsely; fixing it measured the cost and filed #3892 (eight coordinated sites,
five of them literal counts). An hour later a NINTH site was found the same
way -- crates/autospec-cli/tests/validation_parity.rs -- and held three more
(#3902).

check-catalog-count-parity.sh asserts CONSISTENCY, not any value -- bumping the
catalog is expected; bumping it unevenly is the defect. On divergence it names
every site and its value, so the fix is the diff rather than a bisect.

Exercised both ways, reproducing the exact failure:

  even            -> ok:   catalog count literals agree across 6 sites
                           (full_plan=161, unique=156)
  one copy bumped -> FAIL: catalog 'full_plan' count disagrees: [161, 162]
                             161  .../validation_runner.rs
                             161  .../validation_catalog.rs
                             162  .../validation_parity.rs
                           rc=1

It also fails if an assertion is renamed or moved, rather than silently
checking fewer sites -- a guard that quietly stops looking is the failure it
exists to prevent.
"#;

/// The #3904 read-back: the invocation read from the PARSED workflow rather
/// than the diff, in the body grammar this module reads.
const READ_BACK_BODY: &str = r#"Adds the step to .github/workflows/rust.yml, before `validate`, and verified
by reading the PARSED workflow rather than the diff:

Invocation read-back:
  job=build-test step='Catalog count parity'
  run=bash scripts/check-catalog-count-parity.sh
  script present and executable
  ok: catalog count literals agree across 6 sites (full_plan=161, unique=156)
"#;

#[test]
fn fix_label_prefix_scope_and_bang() {
    let policy = FixSurfacePolicy::default();
    for title in [
        "fix: register the ninth site",
        "fix(core): register the ninth site",
        "fix!: register the ninth site",
        "Fix(scope): register the ninth site",
    ] {
        assert!(
            is_fix_label(title, &policy),
            "expected a fix label: {title}"
        );
    }
}

#[test]
fn fix_label_negatives() {
    let policy = FixSurfacePolicy::default();
    for title in [
        "feat: register the ninth site",
        "fixup: register the ninth site",
        "fixed: register the ninth site",
        "fix",
        "fix(",
        "",
    ] {
        assert!(
            !is_fix_label(title, &policy),
            "unexpected fix label: {title:?}"
        );
    }
}

#[test]
fn check_paths_cover_scripts_and_bats_only() {
    let policy = FixSurfacePolicy::default();
    for path in [
        "scripts/parity.sh",
        "scripts/tool.py",
        "tests/suites/catalog.bats",
    ] {
        assert!(policy.covers_check(path), "expected a check path: {path}");
    }
    for path in [
        "crates/autospec-core/tests/validation_parity.rs",
        "docs/memory/MEMORY.md",
        ".github/workflows/rust.yml",
        "src/check.rs",
    ] {
        assert!(!policy.covers_check(path), "unexpected check path: {path}");
    }
}

#[test]
fn populated_3903_script_with_no_invocation_is_held() {
    let record = PatchRecord::new(RECORD_3903, BODY_3903);
    let review = review_fix_surface(&record, DIFF_3903, &FixSurfacePolicy::default());

    assert!(review.held());
    assert_eq!(kinds(&review), vec![HoldKind::CheckUninvoked]);
    assert!(!review.labeled_fix);
    assert_eq!(review.check_additions.len(), 1);
    let addition = &review.check_additions[0];
    assert_eq!(addition.path, "scripts/check-catalog-count-parity.sh");
    // Exercised in the body and named in its prose — neither is an
    // invocation, so both halves of the evidence are absent.
    assert!(!addition.self_registered);
    assert!(!addition.read_back);
    assert!(!addition.invoked());
}

#[test]
fn populated_3904_registration_in_same_patch_clears() {
    // What a correct #3903 would have looked like: the script and the step
    // that invokes it, in one patch.
    let record = PatchRecord::new(RECORD_3903, BODY_3903);
    let combined = format!("{DIFF_3903}\n{DIFF_3904}");
    let review = review_fix_surface(&record, &combined, &FixSurfacePolicy::default());

    assert!(review.clear());
    assert_eq!(review.check_additions.len(), 1);
    let addition = &review.check_additions[0];
    assert!(addition.self_registered);
    assert!(!addition.read_back);
}

#[test]
fn populated_3904_parsed_config_read_back_clears() {
    let record = PatchRecord::new(
        "ci: actually run the catalog-count guard (#3904)",
        READ_BACK_BODY,
    );
    let review = review_fix_surface(&record, DIFF_3903, &FixSurfacePolicy::default());

    assert!(review.clear());
    let addition = &review.check_additions[0];
    assert!(!addition.self_registered);
    assert!(addition.read_back);
}

#[test]
fn fix_with_surface_and_gates_clears() {
    let record = PatchRecord::new(
        "fix: register the ninth validation_parity site (#3902)",
        "Fix surface: catalog count literals duplicated across 9 registration sites in two crates\n\
         Fix gates: cargo test -p autospec-core --test validation_runner, cargo test -p autospec-cli --test validation_parity, bash scripts/check-catalog-count-parity.sh\n",
    );
    let review = review_fix_surface(&record, "", &FixSurfacePolicy::default());

    assert!(review.clear());
    assert!(review.labeled_fix);
    assert_eq!(
        review.surface.as_deref(),
        Some("catalog count literals duplicated across 9 registration sites in two crates")
    );
    assert_eq!(review.gates.len(), 3);
    assert_eq!(
        review.gates[0],
        "cargo test -p autospec-core --test validation_runner"
    );
}

#[test]
fn fix_with_empty_body_is_held_for_surface_and_gates() {
    let record = PatchRecord::titled("fix: register the ninth site");
    let review = review_fix_surface(&record, "", &FixSurfacePolicy::default());

    assert!(review.held());
    assert_eq!(
        kinds(&review),
        vec![HoldKind::SurfaceUnrecorded, HoldKind::GatesUnrecorded]
    );
    assert!(review.surface.is_none());
    assert!(review.gates.is_empty());
}

#[test]
fn fix_with_surface_only_is_held_for_gates() {
    let record = PatchRecord::new(
        "fix: bump the four count literals",
        "Fix surface: catalog count literals duplicated across 9 registration sites in two crates\n",
    );
    let review = review_fix_surface(&record, "", &FixSurfacePolicy::default());

    assert_eq!(kinds(&review), vec![HoldKind::GatesUnrecorded]);
    assert!(review.surface.is_some());
}

#[test]
fn empty_surface_value_is_unrecorded() {
    let record = PatchRecord::new(
        "fix: bump the four count literals",
        "Fix surface:\nFix gates: cargo test\n",
    );
    let review = review_fix_surface(&record, "", &FixSurfacePolicy::default());

    assert_eq!(kinds(&review), vec![HoldKind::SurfaceUnrecorded]);
    assert!(review.gates.iter().any(|gate| gate == "cargo test"));
}

#[test]
fn n_a_gate_covers_nothing() {
    let record = PatchRecord::new(
        "fix: regenerate the artefact",
        "Fix surface: the web image build and its lockfile\nFix gates: n/a\n",
    );
    let review = review_fix_surface(&record, "", &FixSurfacePolicy::default());

    assert_eq!(kinds(&review), vec![HoldKind::GatesUnrecorded]);
    assert!(review.gates.is_empty());
}

#[test]
fn non_fix_patch_needs_no_surface_or_gates() {
    let record = PatchRecord::titled("feat: add the guard script");
    let review = review_fix_surface(&record, "", &FixSurfacePolicy::default());

    assert!(review.clear());
    assert!(!review.labeled_fix);
}

#[test]
fn cascade_declared_without_rerun_is_held() {
    let record = PatchRecord::new(
        "fix: register the ninth site (#3902)",
        "Fix surface: the 9 catalog count sites in two crates\nFix gates: cargo test --workspace --no-fail-fast\nCascade: yes\n",
    );
    let review = review_fix_surface(&record, "", &FixSurfacePolicy::default());

    assert!(review.cascade_declared);
    assert_eq!(kinds(&review), vec![HoldKind::HeldVerdictsUnrerun]);
    assert!(review.re_ran_held_verdicts.is_empty());
}

#[test]
fn cascade_with_rerun_verdicts_clears() {
    let record = PatchRecord::new(
        "fix: register the ninth site (#3902)",
        "Fix surface: the 9 catalog count sites in two crates\n\
         Fix gates: cargo test --workspace --no-fail-fast\n\
         Cascade: seven patches held against the red base\n\
         Re-ran held verdicts: validation_parity for #3907, validation_runner for #3908, validation_catalog for #3909\n",
    );
    let review = review_fix_surface(&record, "", &FixSurfacePolicy::default());

    assert!(review.cascade_declared);
    assert!(review.clear());
    assert_eq!(review.re_ran_held_verdicts.len(), 3);
}

#[test]
fn cascade_no_is_not_declared() {
    let record = PatchRecord::new(
        "fix: register the ninth site",
        "Fix surface: the 9 catalog count sites in two crates\nFix gates: cargo test --workspace\nCascade: no\n",
    );
    let review = review_fix_surface(&record, "", &FixSurfacePolicy::default());

    assert!(!review.cascade_declared);
    assert!(review.clear());
}

#[test]
fn cascade_re_ran_n_a_still_held() {
    let record = PatchRecord::new(
        "fix: register the ninth site",
        "Fix surface: the 9 catalog count sites in two crates\nFix gates: cargo test --workspace\nCascade: yes\nRe-ran held verdicts: n/a\n",
    );
    let review = review_fix_surface(&record, "", &FixSurfacePolicy::default());

    assert_eq!(kinds(&review), vec![HoldKind::HeldVerdictsUnrerun]);
    assert!(review.re_ran_held_verdicts.is_empty());
}

#[test]
fn modified_check_script_is_not_held() {
    let record = PatchRecord::titled("chore: tighten the parity guard");
    let diff = r#"diff --git a/scripts/check-catalog-count-parity.sh b/scripts/check-catalog-count-parity.sh
index 930cd1ee..a1b2c3d 100644
--- a/scripts/check-catalog-count-parity.sh
+++ b/scripts/check-catalog-count-parity.sh
@@ -15,3 +15,4 @@
 # expected and fine; bumping it in some copies and not others is the defect.
 set -uo pipefail
+set -o errexit
 cd "$(dirname "$0")/.."
"#;
    let review = review_fix_surface(&record, diff, &FixSurfacePolicy::default());

    assert!(review.clear());
    assert!(review.check_additions.is_empty());
}

#[test]
fn self_reference_inside_the_check_itself_is_not_invocation() {
    let record = PatchRecord::titled("chore: add the parity guard");
    let diff = r#"diff --git a/scripts/check-catalog-count-parity.sh b/scripts/check-catalog-count-parity.sh
new file mode 100755
index 00000000..930cd1ee
--- /dev/null
+++ b/scripts/check-catalog-count-parity.sh
@@ -0,0 +1,2 @@
+#!/usr/bin/env bash
+echo check-catalog-count-parity.sh
"#;
    let review = review_fix_surface(&record, diff, &FixSurfacePolicy::default());

    assert_eq!(kinds(&review), vec![HoldKind::CheckUninvoked]);
    assert!(!review.check_additions[0].self_registered);
}

#[test]
fn registration_line_in_another_file_clears() {
    // The registration line appears as context in another file of the patch:
    // a context line shows that file's current content, so it is proof of an
    // invocation that already exists.
    let record = PatchRecord::titled("chore: add the parity guard");
    let diff = r#"diff --git a/scripts/check-catalog-count-parity.sh b/scripts/check-catalog-count-parity.sh
new file mode 100755
index 00000000..930cd1ee
--- /dev/null
+++ b/scripts/check-catalog-count-parity.sh
@@ -0,0 +1,1 @@
+#!/usr/bin/env bash
diff --git a/tests/run-validation.sh b/tests/run-validation.sh
index 1111111..2222222 100644
--- a/tests/run-validation.sh
+++ b/tests/run-validation.sh
@@ -1,2 +1,3 @@
 #!/usr/bin/env bash
+cargo test --workspace --no-fail-fast
 ./check-catalog-count-parity.sh
"#;
    let review = review_fix_surface(&record, diff, &FixSurfacePolicy::default());

    assert!(review.clear());
    assert!(review.check_additions[0].self_registered);
}

#[test]
fn read_back_block_terminates_at_blank_line() {
    // The second block names the check, but the first block ends at the blank
    // line and is the one read: a check named only past the first block is
    // still uninvoked.
    let body = "Invocation read-back:\nrun=bash scripts/other.sh\n\nInvocation read-back:\nrun=bash scripts/check-catalog-count-parity.sh\n";
    let record = PatchRecord::new("chore: add the parity guard", body);
    let review = review_fix_surface(&record, DIFF_3903, &FixSurfacePolicy::default());

    assert_eq!(kinds(&review), vec![HoldKind::CheckUninvoked]);
    assert!(!review.check_additions[0].read_back);
}

#[test]
fn new_rust_integration_test_is_not_a_check_addition() {
    let record = PatchRecord::titled("test: pin the fix-surface holds");
    let diff = r#"diff --git a/crates/autospec-core/tests/fix_surface.rs b/crates/autospec-core/tests/fix_surface.rs
new file mode 100644
index 00000000..1234567
--- /dev/null
+++ b/crates/autospec-core/tests/fix_surface.rs
@@ -0,0 +1,3 @@
+// cargo discovers this file; no registration line exists to hold on.
+#[test]
+fn pin() {}
"#;
    let review = review_fix_surface(&record, diff, &FixSurfacePolicy::default());

    assert!(review.clear());
    assert!(review.check_additions.is_empty());
}

#[test]
fn unreadable_prose_patch_is_held_fail_closed() {
    let record = PatchRecord::titled("fix: register the ninth site");
    let review = review_fix_surface(
        &record,
        "definitely not a diff, just a sentence",
        &FixSurfacePolicy::default(),
    );

    assert_eq!(
        kinds(&review),
        vec![
            HoldKind::SurfaceUnrecorded,
            HoldKind::GatesUnrecorded,
            HoldKind::PatchUnreadable
        ]
    );
    assert!(review.diff_parse_error.is_some());
}

#[test]
fn malformed_hunk_header_is_held_fail_closed() {
    let record = PatchRecord::titled("chore: add the parity guard");
    let diff = r#"diff --git a/scripts/check-catalog-count-parity.sh b/scripts/check-catalog-count-parity.sh
new file mode 100755
index 00000000..930cd1ee
--- /dev/null
+++ b/scripts/check-catalog-count-parity.sh
@@ -x +1,2 @@
+#!/usr/bin/env bash
+echo done
"#;
    let review = review_fix_surface(&record, diff, &FixSurfacePolicy::default());

    assert_eq!(kinds(&review), vec![HoldKind::PatchUnreadable]);
    assert!(review.check_additions.is_empty());
}

#[test]
fn two_uninvoked_checks_hold_in_patch_order() {
    let record = PatchRecord::titled("chore: add two guards");
    let diff = r#"diff --git a/scripts/guard-a.sh b/scripts/guard-a.sh
new file mode 100755
index 00000000..1111111
--- /dev/null
+++ b/scripts/guard-a.sh
@@ -0,0 +1,1 @@
+#!/usr/bin/env bash
diff --git a/scripts/guard-b.sh b/scripts/guard-b.sh
new file mode 100755
index 00000000..2222222
--- /dev/null
+++ b/scripts/guard-b.sh
@@ -0,0 +1,1 @@
+#!/usr/bin/env bash
"#;
    let review = review_fix_surface(&record, diff, &FixSurfacePolicy::default());

    assert_eq!(
        kinds(&review),
        vec![HoldKind::CheckUninvoked, HoldKind::CheckUninvoked]
    );
    let paths: Vec<&str> = review
        .check_additions
        .iter()
        .map(|c| c.path.as_str())
        .collect();
    assert_eq!(paths, vec!["scripts/guard-a.sh", "scripts/guard-b.sh"]);
    assert!(review.hold_lines().contains("guard-a.sh"));
    assert!(review.hold_lines().contains("guard-b.sh"));
}

#[test]
fn fix_labeled_with_uninvoked_check_and_empty_body_is_held_three_ways() {
    let record = PatchRecord::titled("fix: add the guard");
    let review = review_fix_surface(&record, DIFF_3903, &FixSurfacePolicy::default());

    assert_eq!(
        kinds(&review),
        vec![
            HoldKind::SurfaceUnrecorded,
            HoldKind::GatesUnrecorded,
            HoldKind::CheckUninvoked
        ]
    );
}

#[test]
fn hold_lines_and_codes_follow_the_contract() {
    let record = PatchRecord::new(RECORD_3903, BODY_3903);
    let review = review_fix_surface(&record, DIFF_3903, &FixSurfacePolicy::default());

    assert!(review.held());
    assert!(!review.clear());
    assert_eq!(review.holds.len(), 1);
    let hold = &review.holds[0];
    assert_eq!(hold.code, HOLD_REASON);
    assert_eq!(hold.kind, HoldKind::CheckUninvoked);
    assert_eq!(HoldKind::CheckUninvoked.as_str(), "CHECK_UNINVOKED");
    assert_eq!(hold.line(), hold.message);
    assert!(hold
        .message
        .contains("scripts/check-catalog-count-parity.sh"));
    // hold_lines joins one line per hold; one hold, no separator needed.
    assert_eq!(review.hold_lines(), hold.message);
}

#[test]
fn custom_policy_remaps_labels_and_check_paths() {
    let policy = FixSurfacePolicy {
        fix_title_prefixes: vec!["repair".to_string()],
        check_path_prefixes: vec!["tools/checks/".to_string()],
        check_path_suffixes: vec![".bats".to_string()],
    };
    assert!(policy.labeled_as_fix("repair: register the site"));
    assert!(!policy.labeled_as_fix("fix: register the site"));
    assert!(policy.covers_check("tools/checks/gate.sh"));
    assert!(!policy.covers_check("scripts/gate.sh"));

    let diff = r#"diff --git a/tools/checks/gate.sh b/tools/checks/gate.sh
new file mode 100755
index 00000000..1111111
--- /dev/null
+++ b/tools/checks/gate.sh
@@ -0,0 +1,1 @@
+#!/usr/bin/env bash
"#;
    let record = PatchRecord::new(
        "repair: add gate",
        "Fix surface: the gate's own registration sites\nFix gates: cargo test --workspace\n",
    );
    let review = review_fix_surface(&record, diff, &policy);
    assert_eq!(kinds(&review), vec![HoldKind::CheckUninvoked]);
}

#[test]
fn check_addition_api_reports_invocation() {
    let uninvoked = CheckAddition {
        path: "scripts/guard.sh".to_string(),
        self_registered: false,
        read_back: false,
    };
    assert!(!uninvoked.invoked());
    assert!(CheckAddition {
        read_back: true,
        ..uninvoked.clone()
    }
    .invoked());
    assert!(CheckAddition {
        self_registered: true,
        ..uninvoked
    }
    .invoked());
}
