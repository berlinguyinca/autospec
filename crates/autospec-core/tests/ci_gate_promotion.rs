//! A patch may not promote a CI gate to blocking by citing vibes (#3863).
//!
//! The fixtures are the InferWeave #231 shape: `deploy.yaml` adds
//! `reproducible-build` to the `needs:` list of the `publish` job while that job
//! has no successful run on `main`, which is a decision that turns every
//! downstream job red and was argued from first principles instead of settled by
//! looking. The positive cases pin the hold firing on that patch; the negative
//! cases pin it *not* firing on a removal, a reorder, or a promotion whose
//! passing run is cited — because a check that has only ever seen the empty case
//! proves nothing either way.

use autospec_core::ci_gate_promotion::{
    dependency_changes, review_gate_promotions, GateEvidence, GatePromotionPolicy,
    GatePromotionReview, HoldKind, JobDependency, SuccessfulRun, DEFAULT_WORKFLOW_PREFIX,
    HOLD_REASON, UNRESOLVED_DEPENDENCY, UNRESOLVED_JOB,
};

const DEPLOY: &str = ".github/workflows/deploy.yaml";
const GATE: &str = "reproducible-build";
const PASSING_RUN: &str = "4692135711";

/// `publish` gains a second dependency. The gate it promotes has never run.
const PROMOTION_PATCH: &str = r#"diff --git a/.github/workflows/deploy.yaml b/.github/workflows/deploy.yaml
index 3f1a2bc..9de4f10 100644
--- a/.github/workflows/deploy.yaml
+++ b/.github/workflows/deploy.yaml
@@ -12,7 +12,7 @@ jobs:
   publish:
-    needs: changes
+    needs: [changes, reproducible-build]
     runs-on: ubuntu-latest
     steps:
       - uses: actions/checkout@v4
"#;

/// The same edge expressed as a block sequence, added whole.
const BLOCK_PROMOTION_PATCH: &str = r#"diff --git a/.github/workflows/deploy.yaml b/.github/workflows/deploy.yaml
index 3f1a2bc..b7c0011 100644
--- a/.github/workflows/deploy.yaml
+++ b/.github/workflows/deploy.yaml
@@ -40,6 +40,9 @@ jobs:
   release:
     runs-on: ubuntu-latest
+    needs:
+      - changes
+      - reproducible-build
     steps:
       - uses: actions/checkout@v4
       - run: ./scripts/release.sh
"#;

/// The edge is dropped rather than added.
const REMOVAL_PATCH: &str = r#"diff --git a/.github/workflows/deploy.yaml b/.github/workflows/deploy.yaml
index 9de4f10..3f1a2bc 100644
--- a/.github/workflows/deploy.yaml
+++ b/.github/workflows/deploy.yaml
@@ -12,7 +12,7 @@ jobs:
   publish:
-    needs: [changes, reproducible-build]
+    needs: changes
     runs-on: ubuntu-latest
     steps:
       - uses: actions/checkout@v4
"#;

/// Same edges, different order.
const REORDER_PATCH: &str = r#"diff --git a/.github/workflows/deploy.yaml b/.github/workflows/deploy.yaml
index 9de4f10..5aa1c22 100644
--- a/.github/workflows/deploy.yaml
+++ b/.github/workflows/deploy.yaml
@@ -12,7 +12,7 @@ jobs:
   publish:
-    needs: [changes, reproducible-build]
+    needs: [reproducible-build, changes]
     runs-on: ubuntu-latest
     steps:
       - uses: actions/checkout@v4
"#;

/// The `needs:` line lands in a hunk that never shows the job it belongs to.
const UNATTRIBUTED_PATCH: &str = r#"diff --git a/.github/workflows/deploy.yaml b/.github/workflows/deploy.yaml
index 3f1a2bc..c2d3e45 100644
--- a/.github/workflows/deploy.yaml
+++ b/.github/workflows/deploy.yaml
@@ -31,6 +31,7 @@
     permissions:
       contents: read
+    needs: reproducible-build
     runs-on: ubuntu-latest
     timeout-minutes: 30
     steps:
"#;

/// The dependency is an expression, so no job with observable runs is named.
const COMPUTED_DEPENDENCY_PATCH: &str = r#"diff --git a/.github/workflows/deploy.yaml b/.github/workflows/deploy.yaml
index 3f1a2bc..d4e5f67 100644
--- a/.github/workflows/deploy.yaml
+++ b/.github/workflows/deploy.yaml
@@ -12,6 +12,7 @@ jobs:
   publish:
+    needs: ${{ fromJSON(needs.matrix.outputs.gate) }}
     runs-on: ubuntu-latest
     steps:
       - uses: actions/checkout@v4
"#;

/// One hunk tightens a gate, another loosens one, in the same file.
const TWO_JOB_PATCH: &str = r#"diff --git a/.github/workflows/deploy.yaml b/.github/workflows/deploy.yaml
index 3f1a2bc..e5f6789 100644
--- a/.github/workflows/deploy.yaml
+++ b/.github/workflows/deploy.yaml
@@ -12,7 +12,7 @@ jobs:
   publish:
-    needs: changes
+    needs: [changes, reproducible-build]
     runs-on: ubuntu-latest
     steps:
       - uses: actions/checkout@v4
@@ -40,7 +40,7 @@ jobs:
   docs:
-    needs: [changes, reproducible-build]
+    needs: changes
     runs-on: ubuntu-latest
     steps:
       - uses: actions/checkout@v4
"#;

/// The same edit in a file no CI runner reads.
const DOCS_SITE_PATCH: &str = r#"diff --git a/docs/site/netlify.yaml b/docs/site/netlify.yaml
index 3f1a2bc..9de4f10 100644
--- a/docs/site/netlify.yaml
+++ b/docs/site/netlify.yaml
@@ -12,7 +12,7 @@ jobs:
   publish:
-    needs: changes
+    needs: [changes, reproducible-build]
     runs-on: ubuntu-latest
"#;

/// A new workflow whose own job gates on an existing one.
const NEW_WORKFLOW_PATCH: &str = r#"diff --git a/.github/workflows/nightly.yaml b/.github/workflows/nightly.yaml
new file mode 100644
index 0000000..12ab3cd
--- /dev/null
+++ b/.github/workflows/nightly.yaml
@@ -0,0 +1,9 @@
+name: nightly
+on:
+  schedule:
+    - cron: "17 3 * * *"
+jobs:
+  audit:
+    needs: reproducible-build
+    runs-on: ubuntu-latest
+    steps:
+      - uses: actions/checkout@v4
"#;

fn policy() -> GatePromotionPolicy {
    GatePromotionPolicy::default()
}

fn run_on_main() -> SuccessfulRun {
    SuccessfulRun::new(GATE, PASSING_RUN, "main")
}

fn review(patch: &str, evidence: &GateEvidence) -> GatePromotionReview {
    review_gate_promotions(patch, &policy(), evidence)
}

fn promotion(workflow: &str, job: &str, needs: &str) -> JobDependency {
    JobDependency::new(workflow, job, needs)
}

#[test]
fn policy_defaults_to_main_and_the_github_workflows_directory() {
    let policy = GatePromotionPolicy::default();
    assert_eq!(policy.target_branch, "main");
    assert_eq!(
        policy.workflow_prefixes,
        vec![DEFAULT_WORKFLOW_PREFIX.to_string()]
    );
    assert!(policy.covers(DEPLOY));
    assert!(policy.covers(".github/workflows/ci.yml"));
    assert!(!policy.covers(".github/workflows/notes.md"));
    assert!(!policy.covers("docs/site/netlify.yaml"));
}

// --- positive: the check fires ---------------------------------------------------

#[test]
fn holds_a_promotion_that_cites_no_run() {
    let review = review(PROMOTION_PATCH, &GateEvidence::new());
    assert!(review.held(), "expected a hold: {}", review.hold_lines());
    assert_eq!(review.holds.len(), 1);
    let hold = &review.holds[0];
    assert_eq!(hold.code, HOLD_REASON);
    assert_eq!(hold.kind, HoldKind::NoSuccessfulRun);
    assert_eq!(hold.workflow, DEPLOY);
    assert_eq!(hold.job, "publish");
    assert_eq!(hold.needs, GATE);
    assert_eq!(review.releases, Vec::new());
    assert_eq!(review.patch_parse_error, None);
}

#[test]
fn the_hold_names_the_job_and_states_the_evidence_that_clears_it() {
    let review = review(PROMOTION_PATCH, &GateEvidence::new());
    let message = review.hold_lines();
    assert!(message.contains(HOLD_REASON), "{message}");
    assert!(message.contains("publish"), "{message}");
    assert!(message.contains(GATE), "{message}");
    assert!(message.contains(DEPLOY), "{message}");
    assert!(
        message.contains(&format!("cite a successful run of `{GATE}` on `main`")),
        "{message}"
    );
    assert!(message.contains("run id or URL"), "{message}");
    // The distinction the InferWeave thread needed spelled out.
    assert!(message.contains("is not evidence"), "{message}");
    assert!(message.contains("propose the promotion"), "{message}");
}

#[test]
fn holds_a_block_sequence_promotion_against_every_job_it_adds() {
    let review = review(BLOCK_PROMOTION_PATCH, &GateEvidence::new());
    assert_eq!(
        review.promotions,
        vec![
            promotion(DEPLOY, "release", "changes"),
            promotion(DEPLOY, "release", GATE)
        ]
    );
    assert!(review.held());
    assert!(review
        .holds
        .iter()
        .all(|hold| hold.code == HOLD_REASON && hold.job == "release"));
}

#[test]
fn holds_a_promotion_of_a_gate_that_only_passed_on_another_branch() {
    let evidence = GateEvidence::new()
        .with_run(SuccessfulRun::new(
            GATE,
            "4692135700",
            "feat/reproducible-build",
        ))
        .citing("run 4692135700");
    let review = review(PROMOTION_PATCH, &evidence);
    assert!(review.held());
    assert_eq!(review.holds[0].kind, HoldKind::NoSuccessfulRun);
    assert!(review.holds[0].message.contains("other branches"));
    assert!(!review.holds[0].message.contains("cites none of them"));
}

#[test]
fn holds_a_promotion_when_a_passing_run_exists_but_nothing_cites_it() {
    let review = review(
        PROMOTION_PATCH,
        &GateEvidence::new().with_run(run_on_main()),
    );
    assert!(review.held());
    assert_eq!(review.holds[0].kind, HoldKind::NotCited);
    assert!(review.holds[0].message.contains(PASSING_RUN));
    assert!(review.holds[0].message.contains("cites none of them"));
}

#[test]
fn holds_a_promotion_whose_citation_names_some_other_run() {
    let evidence = GateEvidence::new()
        .with_run(run_on_main())
        .citing("run 1234567890 passed, so this is fine now");
    let review = review(PROMOTION_PATCH, &evidence);
    assert!(review.held());
    assert_eq!(review.holds[0].kind, HoldKind::NotCited);
    assert!(review.holds[0].message.contains("1234567890"));
    assert!(review.holds[0].message.contains("not one of them"));
}

#[test]
fn an_in_principle_argument_cites_nothing() {
    // The wording the InferWeave patch used. It is prose, not a run reference,
    // so it must not clear the hold.
    let evidence = GateEvidence::new()
        .with_run(run_on_main())
        .citing("reproducible-build passes now that the lockfile is checked in");
    let review = review(PROMOTION_PATCH, &evidence);
    assert!(review.held());
    assert_eq!(review.holds[0].kind, HoldKind::NotCited);
}

#[test]
fn a_yaml_comment_in_the_patch_is_not_evidence_until_the_run_is_observed() {
    // The comment adds no run reference to the evidence set, and even a run
    // reference would need an observed passing run behind it.
    let patch = PROMOTION_PATCH.replace(
        "+    needs: [changes, reproducible-build]\n",
        "+    # gate evidence: run 4692135711\n+    needs: [changes, reproducible-build]\n",
    );
    let review = review(&patch, &GateEvidence::new().with_run(run_on_main()));
    assert!(review.held());
    assert_eq!(review.holds[0].kind, HoldKind::NotCited);
}

#[test]
fn holds_a_dependency_whose_job_the_patch_does_not_show() {
    let evidence = GateEvidence::new()
        .with_run(run_on_main())
        .citing(format!("run {PASSING_RUN}").as_str());
    let review = review(UNATTRIBUTED_PATCH, &evidence);
    assert!(review.held());
    assert_eq!(review.holds[0].kind, HoldKind::JobUnresolved);
    assert_eq!(review.holds[0].job, UNRESOLVED_JOB);
    assert!(!review.promotions[0].attributed());
    // Even with a cited passing run, an unattributable promotion stays held:
    // there is no job to have passed.
    assert!(review.holds[0]
        .message
        .contains("not visible in the patch context"));
}

#[test]
fn holds_a_computed_dependency_because_no_job_with_runs_is_named() {
    let evidence = GateEvidence::new()
        .with_run(run_on_main())
        .citing(format!("run {PASSING_RUN}").as_str());
    let review = review(COMPUTED_DEPENDENCY_PATCH, &evidence);
    assert!(review.held());
    assert_eq!(review.holds[0].kind, HoldKind::DependencyUnresolved);
    assert_eq!(review.holds[0].needs, UNRESOLVED_DEPENDENCY);
}

#[test]
fn holds_a_gate_promoted_by_a_brand_new_workflow() {
    let review = review(
        NEW_WORKFLOW_PATCH,
        &GateEvidence::new().with_run(SuccessfulRun::new(GATE, PASSING_RUN, "develop")),
    );
    assert!(review.held());
    assert_eq!(review.holds[0].workflow, ".github/workflows/nightly.yaml");
    assert_eq!(review.holds[0].job, "audit");
}

#[test]
fn holds_a_patch_that_is_not_a_readable_diff() {
    let review = review(
        "the deploy job now depends on reproducible-build",
        &GateEvidence::new(),
    );
    assert!(review.held());
    assert_eq!(review.holds[0].kind, HoldKind::PatchUnreadable);
    assert!(review.patch_parse_error.is_some());
}

#[test]
fn one_hold_is_recorded_per_promoted_edge() {
    let review = review(TWO_JOB_PATCH, &GateEvidence::new());
    assert_eq!(review.promotions, vec![promotion(DEPLOY, "publish", GATE)]);
    assert_eq!(review.releases, vec![promotion(DEPLOY, "docs", GATE)]);
    assert_eq!(review.holds.len(), 1);
    assert_eq!(review.holds[0].job, "publish");
}

// --- negative: the check stays out of the way ---------------------------------

#[test]
fn does_not_hold_a_promotion_whose_passing_run_is_cited() {
    let evidence = GateEvidence::new()
        .with_run(run_on_main())
        .citing(format!("run {PASSING_RUN}").as_str());
    let review = review(PROMOTION_PATCH, &evidence);
    assert!(review.clear(), "unexpected hold: {}", review.hold_lines());
    // The promotion is still reported: it is a decision, just an evidenced one.
    assert_eq!(review.promotions, vec![promotion(DEPLOY, "publish", GATE)]);
}

#[test]
fn accepts_a_citation_written_as_a_run_url() {
    let evidence = GateEvidence::new().with_run(run_on_main()).citing(&format!(
        "gate evidence: https://github.com/example/autospec/actions/runs/{PASSING_RUN}"
    ));
    assert!(review(PROMOTION_PATCH, &evidence).clear());
}

#[test]
fn accepts_a_citation_written_in_another_case() {
    let evidence = GateEvidence::new()
        .with_run(SuccessfulRun::new(GATE, "Run-ABC123", "main"))
        .citing("Gate evidence: RUN-abc123 (main, green)");
    let review = review(PROMOTION_PATCH, &evidence);
    assert!(review.clear(), "{}", review.hold_lines());
    assert_eq!(review.promotions[0].needs, GATE);
}

#[test]
fn does_not_hold_a_dependency_removal() {
    let review = review(REMOVAL_PATCH, &GateEvidence::new());
    assert!(review.clear(), "{}", review.hold_lines());
    assert!(review.promotions.is_empty());
    assert_eq!(review.releases, vec![promotion(DEPLOY, "publish", GATE)]);
}

#[test]
fn does_not_hold_a_reordered_needs_list() {
    let review = review(REORDER_PATCH, &GateEvidence::new());
    assert!(review.clear());
    assert!(review.promotions.is_empty());
    assert!(review.releases.is_empty());
}

#[test]
fn does_not_hold_a_workflow_edit_that_changes_no_dependency() {
    let patch = r#"diff --git a/.github/workflows/deploy.yaml b/.github/workflows/deploy.yaml
index 3f1a2bc..7777777 100644
--- a/.github/workflows/deploy.yaml
+++ b/.github/workflows/deploy.yaml
@@ -12,7 +12,7 @@ jobs:
   publish:
     needs: changes
     runs-on: ubuntu-latest
-    timeout-minutes: 20
+    timeout-minutes: 45
     steps:
       - uses: actions/checkout@v4
"#;
    let review = review(patch, &GateEvidence::new());
    assert!(review.clear());
    assert!(review.promotions.is_empty());
}

#[test]
fn does_not_hold_files_outside_the_workflow_prefix() {
    let outside = review(DOCS_SITE_PATCH, &GateEvidence::new());
    assert!(outside.clear(), "{}", outside.hold_lines());
    assert!(outside.promotions.is_empty());
    // The same edge under the workflow prefix is a promotion.
    let under_workflows = DOCS_SITE_PATCH.replace("docs/site/netlify.yaml", DEPLOY);
    assert!(review(&under_workflows, &GateEvidence::new()).held());
}

#[test]
fn an_empty_patch_promotes_nothing() {
    let review = review("", &GateEvidence::new());
    assert!(review.clear());
    assert!(review.promotions.is_empty());
    assert!(review.releases.is_empty());
    assert_eq!(review.patch_parse_error, None);
}

// --- parsing -------------------------------------------------------------------

#[test]
fn dependency_changes_reads_the_workflow_files_a_patch_touches() {
    let changes = dependency_changes(PROMOTION_PATCH, &policy()).expect("patch parses");
    assert_eq!(changes.workflows, vec![DEPLOY.to_string()]);
    assert_eq!(changes.added, vec![promotion(DEPLOY, "publish", GATE)]);
    assert!(changes.removed.is_empty());
    assert!(!changes.is_empty());

    let untouched = dependency_changes(DOCS_SITE_PATCH, &policy()).expect("patch parses");
    assert!(untouched.workflows.is_empty());
    assert!(untouched.is_empty());
}

#[test]
fn dependency_changes_honours_extra_workflow_prefixes() {
    let mut policy = GatePromotionPolicy::default();
    policy.workflow_prefixes.push("docs/site/".to_string());
    let changes = dependency_changes(DOCS_SITE_PATCH, &policy).expect("patch parses");
    assert_eq!(
        changes.added,
        vec![promotion("docs/site/netlify.yaml", "publish", GATE)]
    );
}

#[test]
fn quoted_job_names_are_attributed_by_name() {
    let patch = r#"diff --git a/.github/workflows/deploy.yaml b/.github/workflows/deploy.yaml
index 3f1a2bc..8888888 100644
--- a/.github/workflows/deploy.yaml
+++ b/.github/workflows/deploy.yaml
@@ -20,6 +20,7 @@ jobs:
   "build and test":
+    needs: reproducible-build
     runs-on: ubuntu-latest
"#;
    let review = review(patch, &GateEvidence::new());
    assert_eq!(
        review.promotions,
        vec![promotion(DEPLOY, "build and test", GATE)]
    );
    assert!(review.held());
}

#[test]
fn a_passing_run_scoped_to_another_workflow_does_not_clear() {
    let scoped =
        SuccessfulRun::new(GATE, PASSING_RUN, "main").in_workflow(".github/workflows/other.yaml");
    let other_workflow = review(
        PROMOTION_PATCH,
        &GateEvidence::new()
            .with_run(scoped)
            .citing(format!("run {PASSING_RUN}").as_str()),
    );
    assert!(other_workflow.held());
    assert_eq!(other_workflow.holds[0].kind, HoldKind::NoSuccessfulRun);

    // Unscoped means the caller vouches for the job name being repo-unique.
    let unscoped = GateEvidence::new()
        .with_run(run_on_main())
        .citing(format!("run {PASSING_RUN}").as_str());
    assert!(review(PROMOTION_PATCH, &unscoped).clear());
}

#[test]
fn an_empty_run_reference_cites_nothing() {
    let run = SuccessfulRun::new(GATE, "", "main");
    assert!(!run.cited_by(&["".to_string(), "run 1".to_string()]));
    assert!(!run.cited_by(&[]));
}

#[test]
fn blank_citations_are_dropped() {
    let evidence = GateEvidence::new().citing("   ");
    assert!(evidence.citations.is_empty());
}
