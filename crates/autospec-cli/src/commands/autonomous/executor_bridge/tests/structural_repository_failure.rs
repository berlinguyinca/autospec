// executor_bridge tests: a structurally unusable checkout — 4 cases.
//
// berlinguyinca/autospec#3148 left every fetch failure that was not a missing remote ref
// classified `TRANSIENT:`. A path git refuses to recognise as a repository is a wiring fault, so
// each retry re-ran the identical command against the identical path, burned the retry budget,
// and then paused under `retry_limit_exhausted` -- a reason that names the budget rather than the
// cause, so the operator was told the conductor gave up but never told why.

use super::support_base::GitFixture;
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;

#[test]
fn a_path_that_is_not_a_repository_is_not_transient() {
    // Deliberately outside the checkout: a directory under `target/` inherits the surrounding
    // worktree, so git would resolve it and the permanent condition would never arise.
    let root = std::env::temp_dir().join(format!(
        "autospec-structural-base-{}-{}",
        std::process::id(),
        "not-a-repository"
    ));
    fs::create_dir_all(&root).expect("create non-repository fixture");

    let error =
        bridge::resolve_base(&root, &BTreeMap::new()).expect_err("a non-repository must fail");

    fs::remove_dir_all(&root).expect("remove non-repository fixture");
    assert!(
        !error.starts_with("TRANSIENT:"),
        "a path git will not recognise cannot be fixed by retrying; error={error}"
    );
}

#[test]
fn a_healthy_repository_still_resolves_its_base() {
    // The classifier must not swallow the working case it sits in front of.
    let fixture = GitFixture::new("structural-healthy");

    bridge::resolve_base(&fixture.repo, &BTreeMap::new()).expect("a real checkout must resolve");
}

#[test]
fn the_structural_classifier_matches_git_wording() {
    assert!(bridge::base_fetch::structural_repository_failure(
        "git [\"fetch\", \"--quiet\", \"origin\", \"refs/heads/main:refs/remotes/origin/main\"] \
         failed: fatal: not a git repository (or any of the parent directories): .git"
    ));
    assert!(bridge::base_fetch::structural_repository_failure(
        "fatal: 'origin' does not appear to be a git repository"
    ));
    // Case-insensitive, so a future capitalisation change cannot silently reclassify it.
    assert!(bridge::base_fetch::structural_repository_failure(
        "FATAL: Not A Git Repository"
    ));
}

#[test]
fn a_genuine_transient_fetch_failure_is_still_retryable() {
    // Only a structurally unusable checkout is permanent here. A network or auth blip must stay
    // TRANSIENT or one bad second would become a hard stop needing a human.
    assert!(!bridge::base_fetch::structural_repository_failure(
        "git [\"fetch\"] failed: fatal: unable to access 'https://...': Could not resolve host"
    ));
    assert!(!bridge::base_fetch::structural_repository_failure(
        "git [\"fetch\"] failed: fatal: Authentication failed"
    ));
}
