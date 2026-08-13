// executor_bridge tests: stale explore pin — 4 cases.
//
// berlinguyinca/autospec#2997. A `.autospec/explore-mode.json` pinning a sandbox branch that is
// not on the remote used to fail as `TRANSIENT: fetch executor base: ... couldn't find remote
// ref ...`. A branch that does not exist will not appear by retrying, so the bridge retried a
// permanent condition every cycle and the repository stayed wedged with no actionable signal --
// on autotrade a month-old pin from a dry run blocked every conductor for hours.

use crate::commands::autonomous::executor_bridge as bridge;
use super::support_base::GitFixture;
use std::collections::BTreeMap;
use std::fs;

fn write_explore_pin(repo: &std::path::Path, branch: &str, head_sha: &str) {
    let directory = repo.join(".autospec");
    fs::create_dir_all(&directory).expect("autospec dir");
    fs::write(
        directory.join("explore-mode.json"),
        format!(
            r#"{{"branch":"{branch}","slug":"auto-test","base":"main","head_sha":"{head_sha}","created_at":"2026-07-10T06:04:12Z"}}"#
        ),
    )
    .expect("write explore pin");
}

#[test]
fn a_pin_to_a_branch_that_is_not_on_the_remote_is_not_transient() {
    let fixture = GitFixture::new("explore-pin-missing");
    write_explore_pin(
        &fixture.repo,
        "autospec/explore/2026-07-10-auto-sy4i0i145288",
        "2fda4d77dc567d605eb54c90c2558aabb660bad9",
    );

    let error = bridge::resolve_base(&fixture.repo, &BTreeMap::new())
        .expect_err("a pin to a nonexistent branch must fail");

    assert!(
        !error.starts_with("TRANSIENT:"),
        "a missing branch will not appear by retrying; error={error}"
    );
}

#[test]
fn the_failure_names_the_file_and_the_remedy() {
    let fixture = GitFixture::new("explore-pin-remedy");
    write_explore_pin(
        &fixture.repo,
        "autospec/explore/2026-07-10-auto-sy4i0i145288",
        "2fda4d77dc567d605eb54c90c2558aabb660bad9",
    );

    let error = bridge::resolve_base(&fixture.repo, &BTreeMap::new()).expect_err("must fail");

    // The original message named only the refspec, so an operator had no way to connect it to a
    // month-old file in their repo.
    assert!(
        error.contains(".autospec/explore-mode.json"),
        "the operator must be told which file wedged them; error={error}"
    );
    assert!(
        error.contains("remove") || error.contains("repoint"),
        "the operator must be told what to do about it; error={error}"
    );
}

#[test]
fn a_genuine_transient_fetch_failure_is_still_retryable() {
    // Only "the ref is not there" is permanent. Network and auth failures must stay TRANSIENT or
    // a blip would become a hard stop.
    assert!(!bridge::base_fetch::missing_remote_ref(
        "git [\"fetch\"] failed: fatal: unable to access 'https://...': Could not resolve host"
    ));
    assert!(!bridge::base_fetch::missing_remote_ref(
        "git [\"fetch\"] failed: fatal: Authentication failed"
    ));
}

#[test]
fn the_missing_ref_classifier_matches_git_wording() {
    assert!(bridge::base_fetch::missing_remote_ref(
        "git [\"fetch\", \"--quiet\", \"origin\", \"refs/heads/x:refs/remotes/origin/x\"] failed: \
         fatal: couldn't find remote ref refs/heads/x"
    ));
    // Case-insensitive, so a future capitalisation change cannot silently reclassify it.
    assert!(bridge::base_fetch::missing_remote_ref(
        "FATAL: Couldn't Find Remote Ref refs/heads/x"
    ));
}
