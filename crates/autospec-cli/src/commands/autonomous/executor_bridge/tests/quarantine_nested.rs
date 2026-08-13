// executor_bridge tests: quarantine / nested — 14 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::support_base::{GitFixture, test_environment};
use super::support_invocation::NonDescendantDirectFixture;
use std::fs::{self, File};
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::time::Duration;

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_quarantine_crash_before_marker_leaves_both_anchors_live() {
    // Break caught: non-descendant classification signaling S before its durable quarantine
    // exists, allowing a restart to promote unrelated H into cleanup authority.
    let environment = test_environment();
    let fixture = NonDescendantDirectFixture::new("quarantine-before-marker");
    environment.launch(bridge::LaunchFailpoint::OwnershipBeforeMarker);
    let error = bridge::reconcile_direct_launch(&fixture.paths, Some(&fixture.intent))
        .expect_err("pre-marker failpoint");
    environment.launch(bridge::LaunchFailpoint::None);
    assert!(error.contains("ownership-before-marker"), "{error}");
    assert!(!fixture.marker_path().exists());
    fixture.assert_anchor_liveness(true, true);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_quarantine_crash_after_marker_retries_supervisor_only() {
    // Break caught: a crash after marker durability stranding S so a retry either kills H or
    // skips exact supervisor cleanup.
    let environment = test_environment();
    let fixture = NonDescendantDirectFixture::new("quarantine-after-marker");
    environment.launch(bridge::LaunchFailpoint::OwnershipAfterMarker);
    let error = bridge::reconcile_direct_launch(&fixture.paths, Some(&fixture.intent))
        .expect_err("post-marker failpoint");
    environment.launch(bridge::LaunchFailpoint::None);
    assert!(error.contains("ownership-after-marker"), "{error}");
    assert!(fixture.marker_path().is_file());
    fixture.assert_anchor_liveness(true, true);
    let retry = bridge::reconcile_direct_launch(&fixture.paths, Some(&fixture.intent))
        .expect_err("durable quarantine retry");
    assert!(retry.contains("permanently quarantined"), "{retry}");
    fixture.assert_anchor_liveness(false, true);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_quarantine_rejects_marker_corruption_before_signal() {
    // Break caught: a forged/rebased current-attempt marker falling through to signal S or H.
    let fixture = NonDescendantDirectFixture::new("quarantine-corruption");
    let exact = fixture.exact_marker();
    let exact_value: serde_json::Value = serde_json::from_str(&exact).expect("exact marker JSON");
    let mut corruptions = Vec::new();
    let mut wrong_attempt = exact_value.clone();
    wrong_attempt["attempt_id"] = serde_json::Value::String("0".repeat(64));
    corruptions.push(wrong_attempt.to_string());
    let mut missing_digest = exact_value.clone();
    missing_digest
        .as_object_mut()
        .expect("marker object")
        .remove("launch_digest");
    corruptions.push(missing_digest.to_string());
    corruptions.push("{malformed".to_string());
    let mut wrong_digest = exact_value.clone();
    wrong_digest["launch_digest"] = serde_json::Value::String("f".repeat(64));
    corruptions.push(wrong_digest.to_string());
    let mut wrong_anchor = exact_value;
    wrong_anchor["supervisor"]["start_identity"] = serde_json::Value::String("forged".to_string());
    corruptions.push(wrong_anchor.to_string());

    for corruption in corruptions {
        fixture.replace_marker(&corruption);
        let error = bridge::reconcile_direct_launch(&fixture.paths, Some(&fixture.intent))
            .expect_err("corrupt marker must fail closed");
        assert!(error.contains("direct ownership quarantine"), "{error}");
        fixture.assert_anchor_liveness(true, true);
        assert!(
            fixture.paths.launch.is_file(),
            "corrupt marker retired launch"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_quarantine_rejects_renamed_current_marker_before_signal() {
    // Break caught: renaming a valid current marker to another canonical attempt-ID path
    // hiding the quarantine from current-path lookup and re-enabling cleanup authority.
    let fixture = NonDescendantDirectFixture::new("quarantine-renamed-current");
    fixture.replace_marker(&fixture.exact_marker());
    let displaced_attempt =
        bridge::reserve_direct_attempt_id(&fixture.paths).expect("displaced attempt id");
    let displaced = bridge::direct_ownership_disproven_marker(&fixture.paths, &displaced_attempt)
        .expect("displaced marker path");
    fs::rename(fixture.marker_path(), &displaced).expect("rename current marker");
    File::open(displaced.parent().expect("marker parent"))
        .and_then(|directory| directory.sync_all())
        .expect("sync renamed marker");

    for _ in 0..2 {
        let error = bridge::reconcile_direct_launch(&fixture.paths, Some(&fixture.intent))
            .expect_err("renamed current marker must fail closed");
        assert!(
            error.contains("filename") || error.contains("marker"),
            "{error}"
        );
        fixture.assert_anchor_liveness(true, true);
        assert!(
            fixture.paths.launch.is_file(),
            "renamed marker retired launch"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_quarantine_is_addressed_to_one_exact_attempt() {
    // Break caught: a prior attempt's immutable diagnostic authorizing or quarantining a new
    // attempt that reuses the same command index.
    let fixture = NonDescendantDirectFixture::new("quarantine-distinct-attempt");
    let artifact_root = fixture.paths.record.parent().expect("artifact root");
    let paths = bridge::direct_attempt_paths(artifact_root, 1);
    let old_attempt = bridge::reserve_direct_attempt_id(&paths).expect("old attempt id");
    let current_attempt = bridge::reserve_direct_attempt_id(&paths).expect("current attempt id");
    let old_marker =
        bridge::direct_ownership_disproven_marker(&paths, &old_attempt).expect("old marker path");
    let old_document = bridge::direct_ownership_disproven_document(
        &old_attempt,
        &"a".repeat(64),
        &"b".repeat(64),
        &fixture.supervisor,
        &fixture.harness,
    );
    bridge::write_private_create_once(
        &old_marker,
        old_document.as_bytes(),
        "old ownership-disproven quarantine",
    )
    .expect("old marker");

    let commit = bridge::git_stdout(
        &fixture._fixture.repo,
        &["rev-parse", "--verify", "HEAD^{commit}"],
    )
    .expect("fixture commit");
    let executable = fs::canonicalize("/usr/bin/true").expect("canonical true fixture");
    let argv = vec![executable.display().to_string()];
    let current_intent =
        bridge::direct_intent_document(&current_attempt, &commit, None, &executable, &argv);
    bridge::write_private_create_once(
        &paths.intent,
        current_intent.as_bytes(),
        "current distinct intent",
    )
    .expect("current intent");

    assert!(
        !bridge::reconcile_direct_launch(&paths, Some(&current_intent))
            .expect("old marker must not authorize current attempt")
    );
    assert!(old_marker.is_file(), "old immutable diagnostic was removed");
    assert!(
        !bridge::direct_ownership_disproven_marker(&paths, &current_attempt)
            .expect("current marker path")
            .exists()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_quarantine_filename_grammar_is_canonical() {
    // Break caught: a truncated, uppercase, or suffix-extended attempt marker entering the
    // direct-attempt inventory as a valid authority artifact.
    let fixture = GitFixture::new("quarantine-filename-grammar");
    let artifact_root = fixture.root.join("evidence");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
        .expect("private artifact root");
    for suffix in [
        "ownership-disproven-deadbeef.json",
        &format!("ownership-disproven-{}.json", "A".repeat(64)),
        &format!("ownership-disproven-{}.json.extra", "a".repeat(64)),
    ] {
        let path = artifact_root.join(format!("command-000.{suffix}"));
        fs::write(&path, b"{}").expect("non-canonical marker fixture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("private marker fixture");
        let error = bridge::discover_direct_attempt_indices(&artifact_root)
            .expect_err("non-canonical marker must fail inventory");
        assert!(error.contains("non-canonical"), "{error}");
        fs::remove_file(path).expect("remove non-canonical fixture");
    }
    let canonical = artifact_root.join(format!(
        "command-000.ownership-disproven-{}.json",
        "a".repeat(64)
    ));
    fs::write(&canonical, b"{}").expect("canonical marker fixture");
    fs::set_permissions(&canonical, fs::Permissions::from_mode(0o600))
        .expect("private canonical marker fixture");
    assert_eq!(
        bridge::discover_direct_attempt_indices(&artifact_root).expect("canonical inventory"),
        std::collections::BTreeSet::from([0])
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_quarantine_inventory_precedes_no_intent_return() {
    // Break caught: a malformed retained marker bypassing the no-intent prelaunch
    // reconciliation and allowing a new harness to launch before quarantine validation.
    let fixture = GitFixture::new("quarantine-no-intent-inventory");
    let artifact_root = fixture.root.join("evidence");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
        .expect("private artifact root");
    let paths = bridge::direct_attempt_paths(&artifact_root, 0);
    let old_attempt = bridge::reserve_direct_attempt_id(&paths).expect("old attempt id");
    let old_marker =
        bridge::direct_ownership_disproven_marker(&paths, &old_attempt).expect("old marker path");
    bridge::write_private_create_once(
        &old_marker,
        b"{\"schema\":1}",
        "malformed retained ownership quarantine",
    )
    .expect("malformed old marker");

    let error = bridge::reconcile_direct_launch(&paths, None)
        .expect_err("marker inventory must precede no-intent return");
    assert!(error.contains("direct ownership quarantine"), "{error}");
    assert!(!paths.intent.exists(), "reconciliation created an intent");
    assert!(!paths.launch.exists(), "reconciliation launched a harness");
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_quarantine_preflight_validates_all_indices_before_action() {
    let _environment = test_environment();
    // Break caught: actionful reconciliation of command 000 cleaning its live supervisor
    // before a malformed retained marker at command 001 is discovered.
    let fixture = NonDescendantDirectFixture::new("quarantine-cross-index-preflight");
    fixture.replace_marker(&fixture.exact_marker());
    let artifact_root = fixture.paths.record.parent().expect("artifact root");
    let later_paths = bridge::direct_attempt_paths(artifact_root, 1);
    let later_attempt = bridge::reserve_direct_attempt_id(&later_paths).expect("later attempt id");
    let later_marker = bridge::direct_ownership_disproven_marker(&later_paths, &later_attempt)
        .expect("later marker path");
    bridge::write_private_create_once(
        &later_marker,
        b"{\"schema\":1}",
        "malformed later ownership quarantine",
    )
    .expect("malformed later marker");
    let first_launch = fs::read(&fixture.paths.launch).expect("first launch snapshot");
    let later_marker_body = fs::read(&later_marker).expect("later marker snapshot");
    let plan = bridge::DirectCommandPlan {
        commands: vec![bridge::DirectCommand::success(vec![
            "/usr/bin/true".to_string()
        ])],
    };

    let error = bridge::execute_direct_plan(
        &fixture._fixture.repo,
        &plan,
        artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("cross-index marker preflight must fail before action");
    assert!(error.contains("quarantine marker schema"), "{error}");
    fixture.assert_anchor_liveness(true, true);
    assert_eq!(
        fs::read(&fixture.paths.launch).expect("first launch after preflight"),
        first_launch,
        "preflight mutated the first launch"
    );
    assert_eq!(
        fs::read(&later_marker).expect("later marker after preflight"),
        later_marker_body,
        "preflight mutated the later marker"
    );
    assert!(
        !later_paths.launch.exists() && !later_paths.record.exists(),
        "preflight launched or finalized a later command"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_nested_legacy_evidence_directories_are_repaired() {
    // Break caught: evidence parents created by older releases used the process umask and
    // blocked recovery forever once nested ownership validation required mode 0700.
    let fixture = GitFixture::new("nested-legacy-evidence-directories");
    let root = fixture.root.join("evidence/premerge");
    let nested = root.join("attempts/legacy/qa");
    fs::create_dir_all(&nested).expect("legacy evidence hierarchy");
    let nested_directories = [
        root.clone(),
        root.join("attempts"),
        root.join("attempts/legacy"),
        nested.clone(),
    ];
    for directory in &nested_directories {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o775))
            .expect("make legacy evidence directory");
    }

    bridge::reconcile_nested_direct_ownership(&root)
        .expect("repair owned legacy evidence hierarchy");

    for directory in &nested_directories {
        assert_eq!(
            fs::metadata(directory)
                .expect("repaired directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "{}",
            directory.display()
        );
    }

    let evidence = fixture.repo.join(".autospec/evidence");
    let premerge = evidence.join("premerge");
    let lane = premerge.join("lane");
    fs::create_dir_all(&lane).expect("legacy lane hierarchy");
    for directory in [&evidence, &premerge] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o775))
            .expect("make legacy lane parent");
    }

    bridge::ensure_private_evidence_path(&fixture.repo, &lane)
        .expect("repair full evidence ancestor chain");

    for directory in [fixture.repo.join(".autospec"), evidence, premerge, lane] {
        assert_eq!(
            fs::metadata(&directory)
                .expect("private evidence ancestor metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "{}",
            directory.display()
        );
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_nested_root_symlinks_fail_closed_before_absence_check() {
    // Break caught: dangling or non-directory root symlinks looking absent to `is_dir` and
    // bypassing the nested evidence ownership boundary.
    let fixture = GitFixture::new("nested-root-symlink");
    let file_target = fixture.root.join("foreign-file");
    fs::write(&file_target, b"foreign\n").expect("file target");
    for (label, target) in [
        ("dangling", fixture.root.join("missing-target")),
        ("file", file_target.clone()),
    ] {
        let root = fixture.root.join(format!("{label}-root"));
        symlink(&target, &root).expect("nested root symlink");

        let error = bridge::reconcile_nested_direct_ownership(&root)
            .expect_err("root symlink must fail closed");

        assert!(error.contains("symlink"), "{label}: {error}");
        assert!(
            fs::symlink_metadata(&root)
                .expect("root symlink metadata")
                .file_type()
                .is_symlink(),
            "{label} root was mutated"
        );
    }
    assert_eq!(
        fs::read(&file_target).expect("file target after rejection"),
        b"foreign\n"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_nested_transaction_archives_are_not_live_attempts() {
    // Break caught: completed archive and retirement snapshots being rediscovered as live
    // command roots, then failing because their attempt reservation was intentionally moved.
    let fixture = GitFixture::new("nested-transaction-archives");
    for kind in ["archive", "retire"] {
        let root = fixture.root.join(kind);
        bridge::ensure_private_directory(&root).expect("private transaction parent");
        let transaction = root.join(format!("command-000.{kind}-completed"));
        bridge::ensure_private_directory(&transaction).expect("private transaction");
        let attempt_id = if kind == "archive" {
            "a".repeat(64)
        } else {
            "b".repeat(64)
        };
        let intent = bridge::direct_intent_document(
            &attempt_id,
            &"c".repeat(40),
            None,
            Path::new("/usr/bin/true"),
            &["/usr/bin/true".to_string()],
        );
        bridge::write_private_create_once(
            &transaction.join("command-000.intent.json"),
            intent.as_bytes(),
            "archived direct intent",
        )
        .expect("archived intent");
        for marker in ["manifest", "complete"] {
            bridge::write_private_create_once(
                &transaction.join(marker),
                b"complete\n",
                "completed direct transaction marker",
            )
            .expect("transaction marker");
        }

        bridge::reconcile_nested_direct_ownership(&root)
            .expect("completed transaction is diagnostic, not live");
    }

    let root = fixture.root.join("combined");
    bridge::ensure_private_directory(&root).expect("combined transaction parent");
    for name in [
        "command-000.archive-completed",
        "command-000.retire-completed",
        "command-000.attempt-ids",
    ] {
        let diagnostic = root.join(name);
        bridge::ensure_private_directory(&diagnostic).expect("combined diagnostic directory");
        bridge::write_private_create_once(
            &diagnostic.join("evidence"),
            b"complete\n",
            "combined diagnostic artifact",
        )
        .expect("combined diagnostic artifact");
    }

    bridge::reconcile_nested_direct_ownership(&root)
        .expect("transaction archives and attempt reservations are diagnostic, not live");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_nested_transaction_archives_reject_symlinks() {
    let fixture = GitFixture::new("nested-transaction-archive-symlink");
    let root = fixture.root.join("archive");
    let transaction = root.join("command-000.archive-completed");
    bridge::ensure_private_directory(&transaction).expect("private transaction");
    symlink(
        fixture.repo.join("README.md"),
        transaction.join("captured-link"),
    )
    .expect("archived symlink");

    let error = bridge::reconcile_nested_direct_ownership(&root)
        .expect_err("archived symlink remains fail-closed");
    assert!(error.contains("forbidden symlink"), "{error}");
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_nested_quarantine_preflight_validates_all_indices_before_action() {
    // Break caught: nested evidence reconciliation cleaning command 000 before discovering a
    // malformed ownership marker at command 001.
    let fixture = NonDescendantDirectFixture::new("nested-quarantine-cross-index");
    fixture.replace_marker(&fixture.exact_marker());
    let root = fixture.paths.record.parent().expect("nested root");
    let later_paths = bridge::direct_attempt_paths(root, 1);
    let later_attempt = bridge::reserve_direct_attempt_id(&later_paths).expect("later attempt id");
    let later_marker = bridge::direct_ownership_disproven_marker(&later_paths, &later_attempt)
        .expect("later marker path");
    bridge::write_private_create_once(
        &later_marker,
        b"{\"schema\":1}",
        "malformed nested ownership quarantine",
    )
    .expect("malformed nested marker");
    let first_launch = fs::read(&fixture.paths.launch).expect("first launch snapshot");
    let later_marker_body = fs::read(&later_marker).expect("later marker snapshot");

    let error = bridge::reconcile_nested_direct_ownership(root)
        .expect_err("nested marker preflight must fail before action");
    assert!(error.contains("quarantine marker schema"), "{error}");
    fixture.assert_anchor_liveness(true, true);
    assert_eq!(
        fs::read(&fixture.paths.launch).expect("first nested launch after preflight"),
        first_launch
    );
    assert_eq!(
        fs::read(&later_marker).expect("later nested marker after preflight"),
        later_marker_body
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_nested_marker_only_orphan_fails_closed() {
    // Break caught: a marker-only nested direct-artifact directory evading ownership
    // validation because the artifact detector only recognized intent/launch sidecars.
    let fixture = GitFixture::new("nested-marker-only-orphan");
    let root = fixture.root.join("nested");
    bridge::ensure_private_directory(&root).expect("nested root");
    let paths = bridge::direct_attempt_paths(&root, 0);
    let marker = bridge::direct_ownership_disproven_marker(&paths, &"a".repeat(64))
        .expect("orphan marker path");
    bridge::write_private_create_once(
        &marker,
        b"{\"schema\":1}",
        "malformed marker-only quarantine",
    )
    .expect("marker-only quarantine");
    let before = fs::read(&marker).expect("marker-only snapshot");

    let error = bridge::reconcile_nested_direct_ownership(&root)
        .expect_err("marker-only nested artifact must fail closed");
    assert!(error.contains("quarantine marker schema"), "{error}");
    assert_eq!(
        fs::read(&marker).expect("marker-only artifact after reconciliation"),
        before
    );
    assert!(!paths.intent.exists() && !paths.launch.exists() && !paths.record.exists());
}
