//! AC#4: a patch with defects in two different file types surfaces both.
//!
//! The checklist does not judge whether a change is correct — it puts the
//! checks that apply to each changed file in front of a reviewer. These tests
//! feed a real unified diff whose TypeScript file carries an unknown config key
//! and whose Containerfile carries a build-aborting step, and assert that both
//! files land in the checklist with the specific check that would catch each
//! defect, and that a removed comment/config value is surfaced next to the new
//! code.

use autospec_core::review_checklist::{
    build_review_checklist, is_complete, parse_unified_diff, FileType, RemovalKind,
};

/// The two-defect patch: a TypeScript config with an unknown key and a
/// Containerfile with a step that aborts the build.
const TWO_DEFECT_PATCH: &str = "\
diff --git a/src/config.ts b/src/config.ts
--- a/src/config.ts
+++ b/src/config.ts
@@ -1,4 +1,4 @@
 export const config = {
-  retries: 3,
+  buildId: \"abc123\",
   timeout: 30,
 };
diff --git a/app/Dockerfile b/app/Dockerfile
--- a/app/Dockerfile
+++ b/app/Dockerfile
@@ -2,4 +2,3 @@
 FROM node:20
-# install runtime deps
-RUN apt-get install -y curl
+RUN useradd --home-dir /var/lib/inferweave && touch /var/lib/inferweave/.ready
 COPY app /app
";

fn parsed() -> Vec<autospec_core::review_checklist::ChangedFile> {
    parse_unified_diff(TWO_DEFECT_PATCH)
}

#[test]
fn both_defective_file_types_surface_with_their_checks() {
    let files = parsed();
    assert_eq!(files.len(), 2);

    let checklist = build_review_checklist(&files);
    assert!(checklist.complete);
    assert!(is_complete(&checklist, &files));
    assert_eq!(checklist.files.len(), 2);

    let ts = checklist
        .files
        .iter()
        .find(|e| e.file_type == FileType::TypeScript)
        .expect("the TypeScript file is in the checklist");
    assert_eq!(ts.path, "src/config.ts");
    // The check that catches the unknown `buildId` key.
    assert!(ts.checks.iter().any(|c| c.contains("config key")));

    let build = checklist
        .files
        .iter()
        .find(|e| e.file_type == FileType::Containerfile)
        .expect("the Containerfile is in the checklist");
    assert_eq!(build.path, "app/Dockerfile");
    // The check that catches the build-aborting step.
    assert!(build.checks.iter().any(|c| c.contains("abort the build")));
}

#[test]
fn removed_comment_and_config_value_are_surfaced_next_to_the_change() {
    let files = parsed();
    let checklist = build_review_checklist(&files);

    let comment = checklist
        .removals
        .iter()
        .find(|r| r.kind == RemovalKind::Comment && r.path == "app/Dockerfile")
        .expect("the removed build comment is surfaced");
    assert_eq!(comment.content, "# install runtime deps");
    assert!(comment.adjacent_to_change);

    let config = checklist
        .removals
        .iter()
        .find(|r| r.kind == RemovalKind::ConfigValue && r.path == "src/config.ts")
        .expect("the removed config value is surfaced");
    assert_eq!(config.content, "  retries: 3,");
    assert!(config.adjacent_to_change);
}

#[test]
fn a_third_defective_file_type_extends_the_same_checklist() {
    // Add the workflow gate defect from the same change; the checklist must
    // grow to three files rather than stopping at the first two.
    let patch = format!(
        "{TWO_DEFECT_PATCH}\
diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml
--- a/.github/workflows/ci.yml
+++ b/.github/workflows/ci.yml
@@ -5,3 +5,3 @@
 jobs:
   publish:
-    needs: [changes]
+    needs: [changes, reproducible-build]
"
    );
    let files = parse_unified_diff(&patch);
    assert_eq!(files.len(), 3);

    let checklist = build_review_checklist(&files);
    assert!(checklist.complete);
    assert_eq!(checklist.files.len(), 3);

    let workflow = checklist
        .files
        .iter()
        .find(|e| e.file_type == FileType::Workflow)
        .expect("the workflow is in the checklist");
    assert_eq!(workflow.path, ".github/workflows/ci.yml");
    assert!(workflow.checks.iter().any(|c| c.contains("needs entry")));
}
