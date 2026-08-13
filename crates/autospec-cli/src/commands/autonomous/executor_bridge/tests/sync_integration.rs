// executor_bridge tests: sync / integration — 18 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::super::{provision_issue_worktree, resolve_base};
use super::support_base::{git, git_stdout, test_root, GitFixture, TEST_SEQUENCE};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_recovers_supervisor_executable_from_argv_zero() {
    let executable = std::env::current_exe().expect("current test executable");

    let resolved = bridge::resolve_executor_supervisor_executable(
        Err("canonicalize executor supervisor executable: stale path".to_string()),
        Some(executable.as_os_str()),
    )
    .expect("resolve stable argv-zero fallback");

    assert_eq!(
        resolved,
        fs::canonicalize(executable).expect("canonical fallback")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_rejects_unrelated_argv_zero_fallback() {
    let fixture = test_root("supervisor-executable-unrelated");
    let unrelated = fixture.join("autospec");
    fs::write(&unrelated, b"unrelated binary").expect("write unrelated executable");

    let error = bridge::resolve_executor_supervisor_executable(
        Err("canonicalize executor supervisor executable: stale path".to_string()),
        Some(unrelated.as_os_str()),
    )
    .expect_err("reject unrelated argv-zero");

    assert!(
        error.contains("does not identify the running image"),
        "{error}"
    );
}

#[test]
fn autonomous_executor_bridge_prefers_primary_supervisor_executable() {
    let primary = std::env::current_exe().expect("current test executable");

    let resolved = bridge::resolve_executor_supervisor_executable(
        Ok(primary.clone()),
        Some(Path::new("/unrelated/argv-zero").as_os_str()),
    )
    .expect("resolve primary executable");

    assert_eq!(
        resolved,
        fs::canonicalize(primary).expect("canonical primary")
    );
}

#[test]
fn autonomous_executor_bridge_rejects_relative_argv_zero_fallback() {
    let error = bridge::resolve_executor_supervisor_executable(
        Err("canonicalize executor supervisor executable: stale path".to_string()),
        Some(Path::new("autospec").as_os_str()),
    )
    .expect_err("reject relative argv-zero");

    assert!(error.contains("not an absolute path"), "{error}");
}

#[test]
fn autonomous_executor_bridge_resolves_validated_base_precedence() {
    let fixture = GitFixture::new("base-precedence");
    let config_oid = fixture.branch("configured");
    let env_oid = fixture.branch("environment");
    let explore_oid = fixture.branch("autospec/explore-safe");
    fs::create_dir_all(fixture.repo.join(".autospec")).expect("create config directory");
    fs::write(
        fixture.repo.join(".autospec/autospec.yml"),
        "git:\n  base_branch: configured\n",
    )
    .expect("write base config");
    let env = BTreeMap::from([(
        "AUTOSPEC_BASE_BRANCH".to_string(),
        OsString::from("environment"),
    )]);

    let selected = resolve_base(&fixture.repo, &env).expect("environment base");
    assert_eq!(selected.base_ref, "origin/environment");
    assert_eq!(selected.base_oid, env_oid);

    fs::write(
        fixture.repo.join(".autospec/explore-mode.json"),
        format!(
            "{{\"branch\":\"autospec/explore-safe\",\"slug\":\"safe\",\"base\":\"main\",\"head_sha\":\"{explore_oid}\",\"created_at\":\"now\"}}\n"
        ),
    )
    .expect("write explore mode");
    let selected = resolve_base(&fixture.repo, &env).expect("explore base");
    assert_eq!(selected.base_ref, "origin/autospec/explore-safe");
    assert_eq!(selected.base_oid, explore_oid);
    assert!(selected.explore_mode);

    fs::remove_file(fixture.repo.join(".autospec/explore-mode.json")).expect("remove explore");
    let selected = resolve_base(&fixture.repo, &BTreeMap::new()).expect("configured base");
    assert_eq!(selected.base_oid, config_oid);
}

#[test]
fn autonomous_executor_bridge_rejects_explore_main_and_unverified_head() {
    let fixture = GitFixture::new("explore-reject");
    fixture.branch("autospec/explore-safe");
    fs::create_dir_all(fixture.repo.join(".autospec")).expect("create config directory");
    for body in [
        r#"{"branch":"main","head_sha":"0000000000000000000000000000000000000000"}"#,
        r#"{"branch":"autospec/missing","head_sha":"0000000000000000000000000000000000000000"}"#,
        r#"{"branch":"autospec/explore-safe","head_sha":"not-an-oid"}"#,
    ] {
        fs::write(fixture.repo.join(".autospec/explore-mode.json"), body)
            .expect("write invalid mode");
        assert!(resolve_base(&fixture.repo, &BTreeMap::new()).is_err());
    }
}

#[test]
fn autonomous_executor_bridge_accepts_fast_forwarded_explore_head() {
    let fixture = GitFixture::new("explore-fast-forward");
    let diverged_oid = fixture.branch("diverged");
    let recorded_oid = fixture.branch("autospec/explore-safe");
    fs::create_dir_all(fixture.repo.join(".autospec")).expect("create config directory");
    fs::write(
        fixture.repo.join(".autospec/explore-mode.json"),
        format!("{{\"branch\":\"autospec/explore-safe\",\"head_sha\":\"{recorded_oid}\"}}\n"),
    )
    .expect("write explore mode");

    git(&fixture.repo, &["checkout", "autospec/explore-safe"]);
    fs::write(fixture.repo.join("advanced.txt"), "advanced").expect("advance explore branch");
    git(&fixture.repo, &["add", "advanced.txt"]);
    git(&fixture.repo, &["commit", "-m", "advance explore branch"]);
    git(&fixture.repo, &["push", "origin", "autospec/explore-safe"]);
    let advanced_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    git(&fixture.repo, &["checkout", "main"]);

    let selected =
        resolve_base(&fixture.repo, &BTreeMap::new()).expect("fast-forwarded explore base");

    assert_eq!(selected.base_oid, advanced_oid);
    assert_ne!(selected.base_oid, recorded_oid);
    assert!(selected.explore_mode);

    fs::write(
        fixture.repo.join(".autospec/explore-mode.json"),
        format!("{{\"branch\":\"autospec/explore-safe\",\"head_sha\":\"{diverged_oid}\"}}\n"),
    )
    .expect("write diverged explore mode");
    let error = resolve_base(&fixture.repo, &BTreeMap::new())
        .expect_err("diverged explore head must fail closed");
    assert!(error.contains("explore head mismatch"), "{error}");
}

/// Advance `origin/main` from an independent clone so the fixture checkout
/// observes a real behind-the-remote integration base.
fn publish_advanced_main(fixture: &GitFixture, marker: &str) -> String {
    let publisher = fixture.root.join(format!("publisher-{marker}"));
    git(
        &fixture.root,
        &[
            "clone",
            fixture.root.join("remote.git").to_str().expect("remote"),
            publisher.to_str().expect("publisher path"),
        ],
    );
    git(
        &publisher,
        &["config", "user.email", "autospec@example.invalid"],
    );
    git(&publisher, &["config", "user.name", "Autospec Test"]);
    fs::write(publisher.join(format!("{marker}.txt")), marker).expect("write publisher file");
    git(&publisher, &["add", "."]);
    git(&publisher, &["commit", "-m", marker]);
    git(&publisher, &["push", "origin", "main"]);
    git_stdout(&publisher, &["rev-parse", "HEAD"])
}

fn stage_remote_advance(fixture: &GitFixture, marker: &str) -> (String, String) {
    let old_oid = git_stdout(&fixture.repo, &["rev-parse", "refs/remotes/origin/main"]);
    let publisher = fixture.root.join(format!("race-publisher-{marker}"));
    git(
        &fixture.root,
        &[
            "clone",
            fixture.root.join("remote.git").to_str().expect("remote"),
            publisher.to_str().expect("publisher path"),
        ],
    );
    git(
        &publisher,
        &["config", "user.email", "autospec@example.invalid"],
    );
    git(&publisher, &["config", "user.name", "Autospec Test"]);
    fs::write(publisher.join(format!("{marker}.txt")), marker).expect("write race marker");
    git(&publisher, &["add", "."]);
    git(&publisher, &["commit", "-m", marker]);
    let new_oid = git_stdout(&publisher, &["rev-parse", "HEAD"]);
    git(
        &publisher,
        &["push", "origin", "HEAD:refs/heads/race-candidate"],
    );
    (old_oid, new_oid)
}

#[test]
fn autonomous_executor_bridge_integration_sync_is_idempotent() {
    let fixture = GitFixture::new("integration-sync");
    assert_eq!(git_stdout(&fixture.repo, &["remote"]), "origin");
    let behind = git_stdout(&fixture.repo, &["rev-parse", "refs/heads/main"]);
    let advanced = publish_advanced_main(&fixture, "advanced");
    assert_ne!(behind, advanced);

    let first = bridge::synchronize_integration_base(&fixture.repo, "main").expect("first sync");
    let second = bridge::synchronize_integration_base(&fixture.repo, "main").expect("second sync");

    assert_eq!(first, advanced);
    assert_eq!(second, advanced);
    assert_eq!(
        git_stdout(&fixture.repo, &["rev-parse", "refs/heads/main"]),
        advanced
    );
    assert_eq!(git_stdout(&fixture.repo, &["rev-parse", "HEAD"]), advanced);
    git(
        &fixture.repo,
        &["merge-base", "--is-ancestor", &behind, &advanced],
    );
}

#[test]
fn autonomous_executor_bridge_integration_sync_rejects_current_dirty_state() {
    for path in ["README.md", "untracked.txt"] {
        let fixture = GitFixture::new(&format!("integration-sync-dirty-{path}"));
        fs::write(fixture.repo.join(path), "dirty").expect("write dirty file");
        let error = bridge::synchronize_integration_base(&fixture.repo, "main")
            .expect_err("dirty integration base must fail closed");
        assert!(error.contains("uncommitted work"), "{error}");
    }
}

#[test]
fn autonomous_executor_bridge_integration_sync_rejects_foreign_worktree() {
    let fixture = GitFixture::new("integration-sync-foreign-worktree");
    git(&fixture.repo, &["checkout", "--detach"]);
    let foreign = fixture.root.join("foreign");
    git(
        &fixture.repo,
        &["worktree", "add", foreign.to_str().unwrap(), "main"],
    );
    let error = bridge::synchronize_integration_base(&fixture.repo, "main")
        .expect_err("foreign integration checkout must fail closed");
    assert!(error.contains("another worktree"), "{error}");
}

#[test]
fn autonomous_executor_bridge_integration_sync_rejects_missing_local_ref() {
    let fixture = GitFixture::new("integration-sync-missing-local");
    git(&fixture.repo, &["checkout", "--detach"]);
    git(&fixture.repo, &["branch", "-D", "main"]);
    let error = bridge::synchronize_integration_base(&fixture.repo, "main")
        .expect_err("missing local integration base must fail closed");
    assert!(error.contains("no local branch"), "{error}");
}

#[test]
fn autonomous_executor_bridge_integration_sync_rejects_divergence() {
    let fixture = GitFixture::new("integration-sync-divergence");
    publish_advanced_main(&fixture, "advanced");
    fs::write(fixture.repo.join("local.txt"), "local").expect("write local file");
    git(&fixture.repo, &["add", "."]);
    git(&fixture.repo, &["commit", "-m", "local"]);
    let diverged = git_stdout(&fixture.repo, &["rev-parse", "refs/heads/main"]);

    let error = bridge::synchronize_integration_base(&fixture.repo, "main")
        .expect_err("diverged integration base must fail closed");

    assert!(error.contains("diverged from origin"), "{error}");
    assert_eq!(
        git_stdout(&fixture.repo, &["rev-parse", "refs/heads/main"]),
        diverged
    );
}

#[test]
fn autonomous_executor_bridge_integration_sync_conflict_fails_closed() {
    let fixture = GitFixture::new("integration-sync-conflict");
    let remote_oid = publish_advanced_main(&fixture, "remote-conflict");
    fs::write(fixture.repo.join("README.md"), "local conflict\n")
        .expect("write conflicting local change");
    git(&fixture.repo, &["add", "README.md"]);
    git(&fixture.repo, &["commit", "-m", "local conflict"]);
    let local_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);

    let error = bridge::synchronize_integration_base(&fixture.repo, "main")
        .expect_err("conflicting integration histories must fail closed");

    assert!(error.contains("diverged from origin"), "{error}");
    assert_eq!(git_stdout(&fixture.repo, &["rev-parse", "HEAD"]), local_oid);
    assert_eq!(
        git_stdout(
            &fixture.root,
            &["--git-dir", "remote.git", "rev-parse", "main"]
        ),
        remote_oid,
        "synchronization must not push or rewrite the remote"
    );
}

#[test]
fn autonomous_executor_bridge_integration_sync_rejects_ambiguous_default_ref() {
    let fixture = GitFixture::new("integration-sync-ambiguous");
    let local_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    git(&fixture.repo, &["tag", "main"]);
    git(&fixture.repo, &["push", "origin", "refs/tags/main"]);

    let error = bridge::synchronize_integration_base(&fixture.repo, "main")
        .expect_err("two matching default refs must fail closed");

    assert!(error.contains("ambiguously"), "{error}");
    assert_eq!(git_stdout(&fixture.repo, &["rev-parse", "HEAD"]), local_oid);
    assert_eq!(
        git_stdout(&fixture.repo, &["ls-remote", "--refs", "origin", "main"])
            .lines()
            .count(),
        2
    );
}

#[test]
fn autonomous_executor_bridge_integration_sync_retries_default_race() {
    let fixture = GitFixture::new("integration-sync-race");
    let (old_oid, new_oid) = stage_remote_advance(&fixture, "second");
    *bridge::INTEGRATION_SYNC_RACE.lock().unwrap() = Some((
        fixture.repo.clone(),
        fixture.root.join("remote.git"),
        old_oid,
        new_oid.clone(),
    ));

    let synchronized = bridge::synchronize_integration_base(&fixture.repo, "main")
        .expect("bounded retry must follow the remote advance");

    assert_eq!(synchronized, new_oid);
    assert_eq!(git_stdout(&fixture.repo, &["rev-parse", "HEAD"]), new_oid);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_explore_links_fail_closed_before_base_fallback() {
    use std::os::unix::fs::symlink;

    let fixture = GitFixture::new("explore-links");
    let environment_oid = fixture.branch("environment");
    fs::create_dir_all(fixture.repo.join(".autospec")).expect("create config directory");
    let explore_path = fixture.repo.join(".autospec/explore-mode.json");
    let env = BTreeMap::from([(
        "AUTOSPEC_BASE_BRANCH".to_string(),
        OsString::from("environment"),
    )]);

    symlink(fixture.root.join("missing-explore.json"), &explore_path)
        .expect("dangling explore symlink");
    let dangling = resolve_base(&fixture.repo, &env)
        .expect_err("dangling explore link must not fall through to the environment base");
    assert!(dangling.contains("symlink"), "{dangling}");
    assert!(fs::symlink_metadata(&explore_path)
        .expect("dangling link remains")
        .file_type()
        .is_symlink());

    fs::remove_file(&explore_path).expect("remove dangling explore link");
    let foreign = fixture.root.join("foreign-explore.json");
    fs::write(
        &foreign,
        format!("{{\"branch\":\"environment\",\"head_sha\":\"{environment_oid}\"}}\n"),
    )
    .expect("write foreign explore mode");
    symlink(&foreign, &explore_path).expect("live explore symlink");
    let live = resolve_base(&fixture.repo, &env)
        .expect_err("live explore link must not be followed or fall through");
    assert!(live.contains("symlink"), "{live}");
    assert!(fs::symlink_metadata(&explore_path)
        .expect("live link remains")
        .file_type()
        .is_symlink());
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_provisions_and_recovers_owned_worktree() {
    let fixture = GitFixture::new("worktree");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "owner_repo_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let sibling_scope = PathBuf::from("/tmp/autospec-executor").join(format!(
        "sibling_scope_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&sibling_scope).expect("create sibling executor scope");
    fs::write(sibling_scope.join("sentinel"), "preserve\n").expect("write sibling scope sentinel");
    let worktree =
        provision_issue_worktree(&fixture.repo, &scope, 42, &base).expect("provision worktree");
    assert_eq!(worktree.branch, "feat/autonomous-issue-42");
    assert!(worktree.path.starts_with("/tmp/autospec-executor"));
    assert_eq!(
        git_stdout(&worktree.path, &["rev-parse", "HEAD"]),
        base.base_oid
    );
    let adopted =
        provision_issue_worktree(&fixture.repo, &scope, 42, &base).expect("adopt worktree");
    assert_eq!(adopted, worktree);

    fs::write(worktree.path.join("dirty.txt"), "dirty").expect("dirty worktree");
    assert_eq!(
        provision_issue_worktree(&fixture.repo, &scope, 42, &base)
            .expect("exact owned retry WIP is recoverable"),
        worktree
    );
    let _ = fs::remove_file(worktree.path.join("dirty.txt"));
    git(
        &fixture.repo,
        &["worktree", "remove", worktree.path.to_str().unwrap()],
    );
    let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
    assert!(
        sibling_scope.join("sentinel").is_file(),
        "fixture teardown removed a sibling executor scope"
    );
    let _ = fs::remove_dir_all(sibling_scope);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_codex_sandbox_isolates_adopted_metadata_wip_before_launch() {
    bridge::METADATA_WIP_SYNC_EVENTS
        .lock()
        .expect("metadata WIP event lock")
        .clear();
    let fixture = GitFixture::new("metadata-wip-adoption");
    fs::write(fixture.repo.join(".gitignore"), "target/\n").expect("write tracked ignore");
    git(&fixture.repo, &["add", ".gitignore"]);
    git(
        &fixture.repo,
        &["commit", "-m", "test: add metadata baseline"],
    );
    git(&fixture.repo, &["push", "origin", "main"]);
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "metadata_wip_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let adopt = || {
        bridge::provision_issue_worktree_for_claim(
            &fixture.repo,
            &scope,
            42,
            &base,
            Some(("claim-metadata", "invocation-metadata")),
        )
    };
    let worktree = adopt().expect("provision issue worktree");
    fs::write(
        worktree.path.join(".gitignore"),
        "target/\n.omx/\noperator-only/\n",
    )
    .expect("write operator ignore WIP");
    fs::create_dir_all(worktree.path.join(".omx")).expect("create operator metadata");
    fs::write(
        worktree.path.join(".omx/session.json"),
        b"{\"operator\":\"preserve\"}\n",
    )
    .expect("write operator metadata");

    bridge::METADATA_WIP_FAILPOINT.store(1, Ordering::SeqCst);
    let interrupted = adopt().expect_err("interrupt after metadata is durably quarantined");
    assert!(interrupted.contains("injected metadata WIP crash"));
    assert_eq!(
        *bridge::METADATA_WIP_SYNC_EVENTS
            .lock()
            .expect("metadata WIP event lock"),
        [
            "rename-gitignore",
            "rename-omx",
            "sync-payload",
            "sync-destination",
            "sync-source",
            "sync-quarantine",
            "sync-metadata-parent",
            "sync-scope-root",
        ]
    );
    bridge::METADATA_WIP_SYNC_EVENTS
        .lock()
        .expect("metadata WIP event lock")
        .clear();

    let adopted = adopt().expect("resume metadata-only WIP isolation");
    assert_eq!(
        *bridge::METADATA_WIP_SYNC_EVENTS
            .lock()
            .expect("metadata WIP event lock"),
        [
            "sync-payload",
            "sync-destination",
            "sync-source",
            "sync-quarantine",
            "sync-metadata-parent",
            "sync-scope-root",
            "restore",
            "sync-restored-payload",
            "sync-restored-source",
            "sync-restored-index",
            "sync-restored-admin",
            "sync-restored-admin-parent",
            "clean",
            "complete",
        ]
    );

    assert_eq!(adopted, worktree);
    assert_eq!(
        git_stdout(
            &adopted.path,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        "",
        "issue-authorized worktree must start clean"
    );
    assert_eq!(
        fs::read_to_string(adopted.path.join(".gitignore")).expect("restored ignore"),
        "target/\n"
    );
    assert!(!adopted.path.join(".omx").exists());
    let quarantine = bridge::metadata_wip_quarantine_path(
        adopted.path.parent().expect("scope root"),
        42,
        "claim-metadata",
    );
    assert_eq!(
        fs::read_to_string(quarantine.join("worktree/.gitignore")).expect("quarantined ignore"),
        "target/\n.omx/\noperator-only/\n"
    );
    assert_eq!(
        fs::read_to_string(quarantine.join("worktree/.omx/session.json"))
            .expect("quarantined metadata"),
        "{\"operator\":\"preserve\"}\n"
    );
    assert!(quarantine.join("status.porcelain").is_file());
    assert!(quarantine.join("tracked.patch").is_file());
    assert!(quarantine.join("complete").is_file());

    let adopted_path = adopted.path.to_str().expect("worktree path");
    git(&fixture.repo, &["worktree", "remove", adopted_path]);
    let _ = fs::remove_dir_all(adopted.path.parent().expect("scope root"));
}
