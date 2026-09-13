//! Integration tests for `autospec_core::process_termination` (issue
//! #4448): the bracketing contract and the exit-144 regression — a
//! pattern kill that must reach its target and stop short of the
//! killer's own session.

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::Command;

use autospec_core::process_termination::{bracket_pattern, kill_matching, BracketError};
use nix::sys::signal::Signal;
use nix::unistd::getpid;

#[test]
fn brackets_the_first_character_of_a_plain_pattern() {
    assert_eq!(
        bracket_pattern("cargo.*test").unwrap(),
        "[c]argo.*test"
    );
    assert_eq!(
        bracket_pattern("autospec worker").unwrap(),
        "[a]utospec worker"
    );
}

#[test]
fn bracketing_is_idempotent_on_an_already_bracketed_pattern() {
    assert_eq!(
        bracket_pattern("[c]argo.*test").unwrap(),
        "[c]argo.*test"
    );
    assert_eq!(bracket_pattern("[a-z]+").unwrap(), "[a-z]+");
}

#[test]
fn an_empty_pattern_refuses_to_match_everything() {
    assert_eq!(bracket_pattern(""), Err(BracketError::Empty));
}

#[test]
fn metacharacter_first_chars_are_not_silently_rewritten() {
    for ch in ['^', '(', ')', '*', '+', '?', '|', '{', ']'] {
        let pattern = format!("{ch}foo");
        let err = bracket_pattern(&pattern).unwrap_err();
        assert!(
            matches!(err, BracketError::FirstCharNotBracketable { ch: c } if c == ch),
            "{pattern}: {err:?}"
        );
    }
}

#[test]
fn punctuation_first_chars_are_bracketed_as_literals() {
    // `[-]leading` and `[.]dot` are one-character classes naming the
    // literal character; bracketing them changes nothing about what a
    // sensible caller meant, and it keeps them self-safe.
    assert_eq!(bracket_pattern("-leading").unwrap(), "[-]leading");
    assert_eq!(bracket_pattern(".dot").unwrap(), "[.]dot");
}

#[test]
fn the_bracketed_form_cannot_match_its_own_argv_text() {
    // The invariant that makes the bracketed form safe to run: the
    // killer's argv carries the bracketed text, and the bracketed regex
    // expects the original character where the text has `[`. Structurally:
    let raw = "cargo.*test";
    let b = bracket_pattern(raw).unwrap();
    assert_eq!(b.as_bytes()[0], b'[');
    assert_eq!(b.as_bytes()[1], raw.as_bytes()[0]);
    assert_eq!(b.as_bytes()[2], b']');
    assert_eq!(&b[3..], &raw[1..]);
    // and the bracketed text does not begin with the character the
    // bracketed regex requires at position zero.
    assert_ne!(b.chars().next(), Some(raw.chars().next().unwrap()));
}

#[test]
fn a_pattern_matching_nothing_reports_zero_instead_of_succeeding_silently() {
    let report = kill_matching("no-such-process-autospec-4448", Signal::SIGTERM).unwrap();
    assert!(
        !report.matched_any(),
        "expected zero matches, got {:?}",
        report
    );
    assert!(report.killed.is_empty());
    assert!(report.failed.is_empty());
    assert!(
        report.line().contains("false negative"),
        "{}",
        report.line()
    );
}

#[test]
fn kill_matching_terminates_the_sentinel_and_never_the_own_session() {
    // The exit-144 regression, as a test: the sentinel's command line
    // carries the plain marker, and — when the suite runs with a filter
    // containing the marker — the test process's own argv carries it
    // too. The kill must reach the sentinel and stop short of the
    // session; a broken exclusion kills the test process itself, which
    // is how the bug always announced itself (a dead reporter).
    let marker = "autospec-4448-sentinel";
    // A compound command keeps the marker in the sentinel's argv: a
    // bare `bash -c 'sleep 30 # marker'` would exec-optimize into
    // `sleep 30` and the marker (the only thing that makes it a
    // target) would vanish from the process table with it.
    let mut child = Command::new("bash")
        .args(["-c", &format!("for i in 1 2 3 4 5 6; do sleep 1; done # {marker}")])
        .spawn()
        .expect("spawning the sentinel must succeed");
    let sentinel_pid = child.id();

    // Give the sentinel a moment to appear in the process table.
    std::thread::sleep(std::time::Duration::from_millis(250));

    let self_pid = getpid().as_raw() as u32;
    let report = kill_matching(marker, Signal::SIGTERM).expect("the kill must run");

    // The session — including this test process — is never a target.
    assert!(
        !report.killed.contains(&self_pid),
        "the kill reached its own process: {report:?}"
    );
    for pid in &report.killed {
        assert!(
            !report.excluded.contains(pid),
            "a session pid was both excluded and killed: {report:?}"
        );
    }
    // The sentinel was a target and is gone.
    assert!(
        report.killed.contains(&sentinel_pid),
        "the sentinel ({sentinel_pid}) was not killed: {report:?}"
    );
    // The sentinel must die *from the signal we sent*. `wait` reaps it
    // (an unreaped zombie would read as "alive" to a pid probe); it is
    // bounded by the sentinel's own 6-second self-expiry, so a kill that
    // misses costs the test 6 seconds and a failure, not a hang.
    let status = child.wait().expect("waiting on the sentinel must succeed");
    assert_eq!(
        status.signal(),
        Some(Signal::SIGTERM as i32),
        "the sentinel {sentinel_pid} did not die from SIGTERM (status: {status:?}); the kill \
         missed it: {report:?}"
    );
}
