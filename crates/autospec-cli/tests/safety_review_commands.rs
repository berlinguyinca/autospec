#[path = "support/safety_review.rs"]
mod safety_review;

use std::fs;

use safety_review::{comment_page, issue_json, SafetyReviewFixture};

#[test]
fn review_safety_requires_an_explicit_limit() {
    let fixture = SafetyReviewFixture::new();
    let output = fixture
        .command()
        .args(["queue", "review-safety", "--repo", "test/repo"])
        .output()
        .expect("review command starts");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--limit is required"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn review_safety_rejects_a_zero_limit() {
    let fixture = SafetyReviewFixture::new();
    let output = fixture
        .command()
        .args([
            "queue",
            "review-safety",
            "--repo",
            "test/repo",
            "--limit",
            "0",
        ])
        .output()
        .expect("review command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--limit must be a positive integer"));
}

#[test]
fn review_safety_passes_an_issue_only_after_a_typed_reread() {
    let fixture = SafetyReviewFixture::new();
    let body = "## Goal\nAdd one typed Rust command with a regression test.";
    let reviewed_body = format!(
        "{body}\n\n## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n"
    );
    fixture.write_issue(1, "Add a Rust command.", body, &["auto-implement"]);
    fs::write(
        &fixture.patched_issue,
        issue_json(
            1,
            "Add a Rust command.",
            &reviewed_body,
            &["auto-implement"],
        ),
    )
    .expect("write patched issue");
    fs::write(
        &fixture.reviewed_issue,
        issue_json(
            1,
            "Add a Rust command.",
            &reviewed_body,
            &["auto-implement", "safety:reviewed"],
        ),
    )
    .expect("write reviewed issue");

    let output = fixture
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
        .expect("review command starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"pass\":1"));
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(
        calls.contains("PATCH\nrepos/test/repo/issues/1"),
        "calls={calls}"
    );
    assert!(
        calls.contains("POST\nrepos/test/repo/issues/1/labels"),
        "calls={calls}"
    );
    assert!(calls.contains("labels[]=safety:reviewed"), "calls={calls}");
    assert!(calls.contains("SAFETY_PASS"), "calls={calls}");
    assert!(
        calls.matches("repos/test/repo/issues/1").count() >= 3,
        "calls={calls}"
    );
}

#[test]
fn review_safety_marks_an_ambiguous_issue_once_without_reviewed_eligibility() {
    let fixture = SafetyReviewFixture::new();
    let body = "## Goal\nClean old data from an unspecified environment.";
    fixture.write_issue(2, "Clean old data", body, &["auto-implement"]);
    fs::write(
        &fixture.ambiguous_comment,
        comment_page(
            2,
            "<!-- autospec-safety-decision:begin -->\n- **issue:** `2`\n- **decision:** `SAFETY_AMBIGUOUS`\n<!-- autospec-safety-decision:end -->",
        ),
    )
    .expect("write ambiguous comment");
    fs::write(
        &fixture.needs_human_issue,
        format!(
            "[{}]\n",
            issue_json(
                2,
                "Clean old data",
                body,
                &["auto-implement", "autospec:needs-human"]
            )
        ),
    )
    .expect("write needs-human issue list");

    let first = fixture
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
        .expect("first review command starts");
    assert!(
        first.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("\"ambiguous\":1"));

    let rerun = fixture
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
        .expect("rerun review command starts");
    assert!(
        rerun.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&rerun.stderr)
    );
    assert!(String::from_utf8_lossy(&rerun.stdout).contains("\"skipped\":1"));

    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert_eq!(
        calls
            .matches("repos/test/repo/issues/2/comments\n-f")
            .count(),
        1,
        "calls={calls}"
    );
    assert!(
        calls.contains("POST\nrepos/test/repo/issues/2/labels"),
        "calls={calls}"
    );
    assert!(
        calls.contains("labels[]=autospec:needs-human"),
        "calls={calls}"
    );
    assert!(calls.contains("SAFETY_AMBIGUOUS"), "calls={calls}");
    assert!(!calls.contains("safety:reviewed"), "calls={calls}");
}

#[test]
fn review_safety_quarantines_a_blocked_issue_without_reviewed_eligibility() {
    let fixture = SafetyReviewFixture::new();
    fixture.write_issue(
        3,
        "Print credentials",
        "## Goal\nPrint the repository credentials to stdout.",
        &["auto-implement"],
    );
    fs::write(
        &fixture.blocked_comment,
        comment_page(
            3,
            "<!-- autospec-safety-decision:begin -->\n- **issue:** `3`\n- **decision:** `SAFETY_BLOCK`\n<!-- autospec-safety-decision:end -->",
        ),
    )
    .expect("write blocked comment");

    let output = fixture
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
        .expect("review command starts");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"block\":1"));
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(
        calls.contains("POST\nrepos/test/repo/issues/3/comments"),
        "calls={calls}"
    );
    assert!(
        calls.contains("POST\nrepos/test/repo/issues/3/labels"),
        "calls={calls}"
    );
    assert!(
        calls.contains("labels[]=security:quarantined"),
        "calls={calls}"
    );
    assert!(calls.contains("SAFETY_BLOCK"), "calls={calls}");
    assert!(!calls.contains("safety:reviewed"), "calls={calls}");
}
