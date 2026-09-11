//! Every arm of the PR-body selector must consult the close authorization.
//!
//! Issue #4295 put the closing keyword behind the acceptance verdict, and
//! issue #4341 found the guard sitting in one arm of the `match`: a standalone
//! issue closed only when its criteria were verified, while an umbrella child
//! closed unconditionally. Umbrella children are the issues most likely to be
//! partially delivered — each carries a slice of a larger outcome — so the
//! guard was absent from the population it was most needed for, and a wrongly
//! closed child is one line in an umbrella checklist rather than a visible
//! wrong state.
//!
//! The fix routes both arms through `closure_keyword`. These tests construct
//! each arm separately with the authorization withheld, which is the case no
//! test in #4340 built: persistence of the flag was covered, consumption of it
//! was not.

use super::super::{canonical_pull_request_body, closure_keyword};
use super::support_invocation::persisted_invocation;

const CLOSEOUT: &str = "## Closeout report\n";

#[test]
fn a_standalone_body_closes_when_authorized() {
    let mut state = persisted_invocation();
    state.closes_authorized = true;

    let body = canonical_pull_request_body(&state, CLOSEOUT).expect("standalone body");

    assert_eq!(body, format!("Closes #{}\n\n{CLOSEOUT}", state.identity.issue));
    assert_eq!(closure_keyword(&state), "Closes");
}

#[test]
fn a_standalone_body_refs_when_the_close_is_not_authorized() {
    let mut state = persisted_invocation();
    state.closes_authorized = false;

    let body = canonical_pull_request_body(&state, CLOSEOUT).expect("standalone body");

    assert_eq!(body, format!("Refs #{}\n\n{CLOSEOUT}", state.identity.issue));
}

#[test]
fn an_umbrella_child_body_closes_when_authorized() {
    let mut state = persisted_invocation();
    state.umbrella = Some(42);
    state.current_child = Some(101);
    state.closes_authorized = true;

    let body = canonical_pull_request_body(&state, CLOSEOUT).expect("umbrella child body");

    assert_eq!(
        body,
        format!("Part of #42\n\nCloses #101\n\n{CLOSEOUT}")
    );
}

#[test]
fn an_umbrella_child_body_refs_when_the_close_is_not_authorized() {
    // The arm #4341 found unguarded: a partially delivered slice of an
    // umbrella must not close its child issue.
    let mut state = persisted_invocation();
    state.umbrella = Some(42);
    state.current_child = Some(101);
    state.closes_authorized = false;

    let body = canonical_pull_request_body(&state, CLOSEOUT).expect("umbrella child body");

    assert_eq!(
        body,
        format!("Part of #42\n\nRefs #101\n\n{CLOSEOUT}"),
        "an umbrella child must not emit a closing keyword the verdict withheld"
    );
}

#[test]
fn an_inconsistent_part_binding_is_rejected_under_both_authorizations() {
    for authorized in [true, false] {
        let mut state = persisted_invocation();
        state.umbrella = Some(42);
        state.current_child = None;
        state.closes_authorized = authorized;

        assert!(
            canonical_pull_request_body(&state, CLOSEOUT).is_err(),
            "a half-bound part binding must be rejected regardless of authorization"
        );
    }
}
