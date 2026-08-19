use std::fs;
use std::process::Command;

use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::coordination::RemoteIssue;

use super::resilience::acquire_test_lifecycle;
use super::tier2::Tier2Scan;
use super::tier2_publisher::{confirm_and_acknowledge, create_issue_arguments, publication_plan};
use super::tier2_receipts::{acknowledge_tier2_publication, record_tier2, Tier2Progress};
use super::tier2_receipts_tests::{
    observation, proposal, seed_tier_two_cursor, store, survives, TempRoot, REPO,
};

const ISOLATED_DIRECT_POST_TEST: &str = "commands::autonomous::tier2_publisher_tests::tier2_publisher_builds_one_direct_post_with_all_queue_labels_isolated";

fn seed_produced(root: &TempRoot, keys: &[&str]) {
    let proposals = keys.iter().map(|key| proposal(key)).collect::<Vec<_>>();
    let verdicts = keys.iter().map(|key| survives(key)).collect::<Vec<_>>();
    seed_tier_two_cursor(root);
    assert_eq!(
        record_tier2(
            root.path(),
            REPO,
            Tier2Scan::Complete(observation(proposals, verdicts)),
        ),
        Ok(Tier2Progress::Produced(keys.len() as u64))
    );
}

#[test]
fn tier2_publisher_plans_only_missing_stable_key_markers() {
    let root = TempRoot::new();
    seed_produced(&root, &["first-gap", "second-gap"]);

    let initial = publication_plan(root.path(), REPO, &[]).expect("publication plan");
    assert_eq!(
        initial
            .iter()
            .map(|draft| draft.stable_key.as_str())
            .collect::<Vec<_>>(),
        ["first-gap", "second-gap"]
    );
    let existing = RemoteIssue::closed(
        41,
        initial[0].title.clone(),
        initial[0].body.clone(),
        vec!["auto-implement".to_string(), "origin:self".to_string()],
        "autospec",
    );
    let retry = publication_plan(root.path(), REPO, &[existing]).expect("retry plan");

    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0].stable_key, "second-gap");
}

#[test]
fn tier2_publisher_renders_machine_checkable_issue_contract() {
    let root = TempRoot::new();
    seed_produced(&root, &["cover-order-book"]);

    let draft = publication_plan(root.path(), REPO, &[])
        .expect("publication plan")
        .pop()
        .expect("one draft");

    assert_eq!(draft.title, "feat: cover-order-book");
    assert!(draft.body.contains("<!-- autospec:tier2-publication:v1:"));
    assert!(draft.body.contains("<!-- autospec:tier2-receipt:v1:"));
    assert!(draft
        .body
        .contains("Implement `cover-order-book` for `Cargo.toml`."));
    assert!(draft
        .body
        .contains("- [ ] `cover-order-book` has `1` passing behavior scenario."));
    assert!(draft.body.contains("`tests/cover-order-book.rs`"));
    assert!(draft.body.contains("```bash\ncargo test\n```"));
    assert_eq!(
        draft.labels,
        [
            "auto-implement",
            "origin:self",
            "ctx:32k",
            "reasoning:medium"
        ]
    );

    let body_path = root.path().join("issue.md");
    fs::write(&body_path, &draft.body).expect("write issue body");
    let linter = Command::new("bash")
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/lint-issue.sh"))
        .arg(&body_path)
        .output()
        .expect("run issue linter");
    assert!(
        linter.status.success(),
        "{}",
        String::from_utf8_lossy(&linter.stdout)
    );
}

#[test]
fn tier2_publisher_rejects_duplicate_publication_markers() {
    let root = TempRoot::new();
    seed_produced(&root, &["one-gap"]);
    let draft = publication_plan(root.path(), REPO, &[])
        .expect("publication plan")
        .pop()
        .expect("one draft");
    let existing = [41, 42]
        .into_iter()
        .map(|number| {
            RemoteIssue::closed(
                number,
                draft.title.clone(),
                draft.body.clone(),
                vec!["auto-implement".to_string(), "origin:self".to_string()],
                "autospec",
            )
        })
        .collect::<Vec<_>>();

    assert!(publication_plan(root.path(), REPO, &existing)
        .expect_err("duplicate markers fail closed")
        .contains("occurs on 2 issues"));
}

#[test]
fn tier2_publisher_rejects_a_marker_without_publication_labels() {
    let root = TempRoot::new();
    seed_produced(&root, &["one-gap"]);
    let draft = publication_plan(root.path(), REPO, &[])
        .expect("publication plan")
        .pop()
        .expect("one draft");
    let existing = RemoteIssue::closed(
        41,
        draft.title,
        draft.body,
        vec!["auto-implement".to_string()],
        "autospec",
    );

    assert!(
        publication_plan(root.path(), REPO, std::slice::from_ref(&existing))
            .expect_err("untrusted marker fails closed")
            .contains("missing origin:self")
    );

    let unqueued = RemoteIssue::open(
        41,
        existing.title.clone(),
        existing.body.clone(),
        vec!["origin:self".to_string()],
        "autospec",
    );
    assert!(publication_plan(root.path(), REPO, &[unqueued])
        .expect_err("open marker without queue label fails closed")
        .contains("missing auto-implement"));

    // Successful finalization removes `auto-implement`, so replaying the produced
    // receipt must not restart-crash after the issue has already closed.
    let finalized = RemoteIssue::closed(
        41,
        existing.title,
        existing.body,
        vec!["origin:self".to_string()],
        "autospec",
    );
    assert_eq!(
        publication_plan(root.path(), REPO, &[finalized]),
        Ok(Vec::new())
    );
}

#[test]
fn tier2_publisher_confirms_every_remote_marker_before_acknowledging() {
    let root = TempRoot::new();
    seed_produced(&root, &["first-gap", "second-gap", "third-gap"]);
    let lease = acquire_test_lifecycle(root.path(), REPO).expect("lifecycle lease");

    assert!(confirm_and_acknowledge(root.path(), REPO, &lease, &[])
        .expect_err("missing confirmation retains cursor")
        .contains("could not confirm 3"));
    assert_eq!(
        store(&root)
            .load_state()
            .expect("state")
            .expect("cursor")
            .current_tier(),
        NoWorkTier::Tier2
    );

    let confirmed = publication_plan(root.path(), REPO, &[])
        .expect("publication plan")
        .into_iter()
        .enumerate()
        .map(|(index, draft)| {
            RemoteIssue::open(
                index as u64 + 1,
                draft.title,
                draft.body,
                draft.labels.into_iter().map(str::to_string).collect(),
                "autospec",
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        confirm_and_acknowledge(root.path(), REPO, &lease, &confirmed),
        Ok(Tier2Progress::Advanced)
    );
}

#[test]
fn tier2_publisher_builds_one_direct_post_with_all_queue_labels() {
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--ignored",
            "--exact",
            ISOLATED_DIRECT_POST_TEST,
            "--nocapture",
        ])
        .output()
        .expect("run isolated Tier 2 publication test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let receipt = format!("test {ISOLATED_DIRECT_POST_TEST} ... ok");
    let receipt_count = stdout.lines().filter(|line| *line == receipt).count();
    assert!(
        output.status.success() && receipt_count == 1,
        "isolated Tier 2 publication emitted {receipt_count} exact receipts: stdout={stdout} stderr={stderr}"
    );
}

#[test]
#[ignore = "launched in isolation by the Tier 2 publication test"]
fn tier2_publisher_builds_one_direct_post_with_all_queue_labels_isolated() {
    let root = TempRoot::new();
    seed_produced(&root, &["one-gap"]);
    let draft = publication_plan(root.path(), REPO, &[])
        .expect("publication plan")
        .pop()
        .expect("one draft");

    let arguments = create_issue_arguments(REPO, &draft);

    assert_eq!(
        &arguments[..4],
        ["api", "--method", "POST", "repos/owner/repo/issues"]
    );
    for label in draft.labels {
        assert!(arguments.contains(&format!("labels[]={label}")));
    }
    assert!(
        arguments.contains(&"body=".to_string())
            || arguments.iter().any(|arg| arg.starts_with("body="))
    );
}

#[test]
fn tier2_publisher_advances_exact_produced_receipt_after_publication() {
    let root = TempRoot::new();
    seed_produced(&root, &["published-gap"]);
    let receipt = store(&root)
        .load_receipt(1, NoWorkTier::Tier2)
        .expect("receipt")
        .expect("sealed produced receipt");

    assert_eq!(
        acknowledge_tier2_publication(root.path(), REPO),
        Ok(Tier2Progress::Advanced)
    );
    let state = store(&root).load_state().expect("state").expect("cursor");
    assert_eq!(state.current_tier(), NoWorkTier::Tier3);
    assert_eq!(
        state
            .completed_receipts()
            .last()
            .map(|completed| (completed.tier, completed.digest.as_str())),
        Some((NoWorkTier::Tier2, receipt.digest()))
    );
}
