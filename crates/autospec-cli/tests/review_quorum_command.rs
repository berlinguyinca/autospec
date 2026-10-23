//! `autospec review-quorum`: exit codes follow the head-bound quorum status (T06).

use std::process::Command;

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

fn run(input: &str) -> (Option<i32>, String, String) {
    let dir = temp_dir("review-quorum");
    let file = dir.join("input.json");
    std::fs::write(&file, input).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["review-quorum", "--input", file.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn input(head: &str, vote_commit: &str, verdict: &str) -> String {
    format!(
        r#"{{"target":"o/r#1","head":"{head}","policy":{{"required":1,"pr_author":"author"}},
        "votes":[{{"reviewer":"r1","target":"o/r#1","verdict":"{verdict}","commit":"{vote_commit}","receipt_digest":"d1"}}]}}"#
    )
}

#[test]
fn satisfied_exits_zero_stale_exits_one_blocked_exits_three() {
    let (a, b) = ("a".repeat(40), "b".repeat(40));
    let (code, out, _) = run(&input(&a, &a, "lgtm"));
    assert_eq!(code, Some(0));
    assert!(out.contains("\"status\":\"satisfied\""));
    let (code, out, _) = run(&input(&b, &a, "lgtm"));
    assert_eq!(code, Some(1));
    assert!(out.contains("\"rereview_required\":true"));
    assert_eq!(run(&input(&a, &a, "blocked")).0, Some(3));
}

#[test]
fn malformed_input_is_a_diagnostic_not_a_pass() {
    let (code, _, err) = run(
        r#"{"target":"o/r#1","head":"abc","policy":{"required":1,"pr_author":"a"},"votes":[]}"#,
    );
    assert_eq!(code, Some(2));
    assert!(err.contains("head"));
    assert_eq!(
        run(r#"{"target":"t","head":"x","policy":{"required":1,"pr_author":"a"},"votes":[],"extra":1}"#).0,
        Some(2)
    );
}

#[test]
fn a_policy_without_pr_author_is_refused() {
    let head = "a".repeat(40);
    let body =
        format!(r#"{{"target":"o/r#1","head":"{head}","policy":{{"required":1}},"votes":[]}}"#);
    assert_eq!(run(&body).0, Some(2));
}
