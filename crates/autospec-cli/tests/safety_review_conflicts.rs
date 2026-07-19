#[path = "support/safety_review.rs"]
mod safety_review;

use std::fs;

use safety_review::{issue_json, SafetyReviewFixture};

const SAFE_TITLE: &str = "Add a Rust command.";
const SAFE_BODY: &str = "## Goal\nAdd one typed Rust command with a regression test.";

fn review(fixture: &SafetyReviewFixture) -> std::process::Output {
    fixture
        .command()
        .args([
            "queue",
            "review-safety",
            "--repo",
            "test/repo",
            "--limit",
            "1",
        ])
        .output()
        .expect("review command starts")
}

#[test]
fn review_safety_preserves_a_concurrent_human_hold_as_a_conflict() {
    let fixture = SafetyReviewFixture::new();
    let reviewed_body = format!(
        "{SAFE_BODY}\n\n## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n"
    );
    fixture.write_issue(1, SAFE_TITLE, SAFE_BODY, &["auto-implement"]);
    fs::write(
        &fixture.patched_issue,
        issue_json(1, SAFE_TITLE, &reviewed_body, &["auto-implement"]),
    )
    .expect("write patched issue");
    fs::write(
        &fixture.reviewed_issue,
        issue_json(
            1,
            SAFE_TITLE,
            &reviewed_body,
            &["auto-implement", "safety:reviewed", "autospec:needs-human"],
        ),
    )
    .expect("write held reread issue");

    let output = review(&fixture);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"pass\":0"), "stdout={stdout}");
    assert!(stdout.contains("\"conflicted\":1"), "stdout={stdout}");
}

#[test]
fn review_safety_counts_malformed_rereads_as_conflicts_without_writeback() {
    let fixture = SafetyReviewFixture::new();
    fixture.write_issue(1, SAFE_TITLE, SAFE_BODY, &["auto-implement"]);
    fs::write(&fixture.current_issue, "not JSON\n").expect("write malformed reread");

    let output = review(&fixture);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"conflicted\":1"));
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(!calls.contains("PATCH"), "calls={calls}");
    assert!(!calls.contains("/labels"), "calls={calls}");
}

#[test]
fn review_safety_counts_invalid_reviewed_evidence_as_a_conflict() {
    let fixture = SafetyReviewFixture::new();
    fixture.write_issue(
        1,
        SAFE_TITLE,
        SAFE_BODY,
        &["auto-implement", "safety:reviewed"],
    );

    let output = review(&fixture);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"conflicted\":1"), "stdout={stdout}");
    assert!(stdout.contains("\"skipped\":0"), "stdout={stdout}");
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(!calls.contains("PATCH"), "calls={calls}");
    assert!(!calls.contains("/labels"), "calls={calls}");
}

#[test]
fn review_safety_skips_classification_drafts() {
    let fixture = SafetyReviewFixture::new();
    fixture.write_issue(
        1,
        SAFE_TITLE,
        SAFE_BODY,
        &["auto-implement", "needs-classify"],
    );

    let output = review(&fixture);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"skipped\":1"));
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(!calls.contains("\nPOST\n"), "calls={calls}");
}
