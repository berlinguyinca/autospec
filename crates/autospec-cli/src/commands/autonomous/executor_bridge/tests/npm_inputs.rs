// executor_bridge tests: npm / inputs — 9 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::support_base::{git, git_stdout, GitFixture};
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn autonomous_executor_bridge_every_required_scanner_fails_closed_when_missing() {
    for missing in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        let mut paths = BTreeMap::new();
        for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
            if scanner != missing {
                paths.insert(scanner.to_string(), PathBuf::from("/usr/bin/true"));
            }
        }
        let error = bridge::ScannerExecutables::from_paths(paths)
            .expect_err("missing required scanner must fail closed");
        assert!(error.contains(missing), "{missing}: {error}");
    }
}

#[test]
fn autonomous_executor_bridge_every_degraded_or_failing_scanner_blocks() {
    let fixture = GitFixture::new("scanner-status");
    for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        let degraded =
            bridge::validate_scanner_result(scanner, 0, b"", b"scanner degraded fallback")
                .expect_err("degraded scanner output must fail closed");
        assert!(degraded.contains(scanner), "{degraded}");

        let failed = bridge::validate_scanner_result(scanner, 1, b"{}", b"")
            .expect_err("failing scanner must fail closed");
        assert!(failed.contains(scanner), "{failed}");

        let generic = bridge::validate_scanner_result(scanner, 0, b"{}", b"")
            .expect_err("generic JSON must not impersonate scanner-native evidence");
        assert!(generic.contains(scanner), "{scanner}: {generic}");
    }
    drop(fixture);
}

#[test]
fn autonomous_executor_bridge_scanner_results_require_native_clean_schemas() {
    let clean = [
        ("gitleaks", br#"[]"#.as_slice()),
        (
            "semgrep",
            br#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]},"version":"1.0"}"#
                .as_slice(),
        ),
        (
            "trivy",
            br#"{"SchemaVersion":2,"Results":[{"Target":".","Vulnerabilities":[],"Misconfigurations":[],"Secrets":[]}]}"#
                .as_slice(),
        ),
        (
            "license-checker",
            br#"{"fixture@1.0.0":{"licenses":"MIT","repository":"https://example.invalid"}}"#
                .as_slice(),
        ),
    ];
    for (scanner, output) in clean {
        bridge::validate_scanner_result(scanner, 0, output, b"")
            .unwrap_or_else(|error| panic!("{scanner} native clean output rejected: {error}"));
    }

    let findings = [
        ("gitleaks", br#"[{"RuleID":"secret"}]"#.as_slice()),
        (
            "semgrep",
            br#"{"results":[{"check_id":"rule"}],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#
                .as_slice(),
        ),
        (
            "trivy",
            br#"{"Results":[{"Target":".","Vulnerabilities":[{"VulnerabilityID":"CVE-1"}]}]}"#
                .as_slice(),
        ),
        (
            "license-checker",
            br#"{"fixture@1.0.0":{"licenses":"GPL-3.0"}}"#.as_slice(),
        ),
    ];
    for (scanner, output) in findings {
        let error = bridge::validate_scanner_result(scanner, 0, output, b"")
            .expect_err("native finding must block");
        assert!(error.contains(scanner), "{scanner}: {error}");
        assert!(error.contains("reported"), "{scanner}: {error}");
    }
    let semgrep_finding = br#"{"results":[{"check_id":"rule"}],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#;
    let error = bridge::validate_scanner_result("semgrep", 1, semgrep_finding, b"")
        .expect_err("Semgrep finding exit must reach native JSON validation");
    assert!(error.contains("reported findings"), "{error}");
    let error = bridge::validate_scanner_result(
        "semgrep",
        1,
        br#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#,
        b"",
    )
    .expect_err("empty Semgrep JSON must not legitimize a non-zero exit");
    assert!(error.contains("exit status 1"), "{error}");
    let error = bridge::validate_scanner_result(
        "semgrep",
        0,
        br#"{"results":[],"errors":[],"paths":{"scanned":[],"skipped":[{"path":"large.rs","reason":"exceeded_size_limit"}]}}"#,
        b"",
    )
    .expect_err("a successful Semgrep process must not hide skipped changed files");
    assert!(
        error.contains("scanned no files") || error.contains("skipped files"),
        "{error}"
    );

    let wrong_shape = [
        ("gitleaks", br#"{"results":[],"errors":[]}"#.as_slice()),
        ("semgrep", br#"[]"#.as_slice()),
        ("trivy", br#"{"results":[],"errors":[]}"#.as_slice()),
        (
            "license-checker",
            br#"{"results":[],"errors":[]}"#.as_slice(),
        ),
    ];
    for (scanner, output) in wrong_shape {
        let error = bridge::validate_scanner_result(scanner, 0, output, b"")
            .expect_err("another tool's JSON shape must not be accepted");
        assert!(error.contains(scanner), "{scanner}: {error}");
    }
}

#[test]
fn autonomous_executor_bridge_changed_paths_preserve_git_status_classes() {
    let changed = bridge::parse_changed_paths(
        b"A\0added\0D\0deleted\0M\0modified\0R100\0old\0new\0C100\0source\0copy\0T\0typed\0",
    )
    .expect("parse Git name-status output");

    for path in [
        "added", "deleted", "modified", "old", "new", "copy", "typed",
    ] {
        assert!(changed.all.contains(path), "missing changed path {path}");
    }
    for path in ["added", "new", "copy"] {
        assert!(changed.added.contains(path), "missing added path {path}");
    }
    for path in ["deleted", "old"] {
        assert!(
            changed.deleted.contains(path),
            "missing deleted path {path}"
        );
    }
    assert!(changed.type_changed.contains("typed"));
    assert!(!changed.all.contains("source"));
}

#[test]
fn autonomous_executor_bridge_changed_paths_reject_malformed_name_status() {
    for (output, expected) in [
        (b"X\0path\0".as_slice(), "status is unsupported"),
        (b"R100\0old\0".as_slice(), "output is truncated"),
        (b"\0path\0".as_slice(), "empty status"),
        (b"\xff\0path\0".as_slice(), "status is not valid UTF-8"),
        (b"M\0\xff\0".as_slice(), "path is not valid UTF-8"),
    ] {
        let error = bridge::parse_changed_paths(output)
            .expect_err("malformed Git name-status output must fail");
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_follow_manifest_policy() {
    // Break caught: treating scripts as graph input, or treating any other top-level
    // manifest field as irrelevant, weakens the conservative npm classification policy.
    let fixture = GitFixture::new("npm-dependency-inputs-manifest-policy");
    fs::write(
        fixture.repo.join("package.json"),
        r#"{"scripts":{"test":"old"}}"#,
    )
    .expect("baseline manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);

    for (field, current, expected) in [
        ("scripts", r#"{"scripts":{"test":"new"}}"#, false),
        (
            "dependencies",
            r#"{"scripts":{"test":"old"},"dependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "devDependencies",
            r#"{"scripts":{"test":"old"},"devDependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "optionalDependencies",
            r#"{"scripts":{"test":"old"},"optionalDependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "peerDependencies",
            r#"{"scripts":{"test":"old"},"peerDependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "peerDependenciesMeta",
            r#"{"scripts":{"test":"old"},"peerDependenciesMeta":{"fixture":{"optional":true}}}"#,
            true,
        ),
        (
            "overrides",
            r#"{"scripts":{"test":"old"},"overrides":{"fixture":"2"}}"#,
            true,
        ),
        (
            "version",
            r#"{"scripts":{"test":"old"},"version":"2.0.0"}"#,
            true,
        ),
        (
            "unknown",
            r#"{"scripts":{"test":"old"},"autospecUnknown":true}"#,
            true,
        ),
    ] {
        fs::write(fixture.repo.join("package.json"), current).expect("current manifest");
        git(&fixture.repo, &["add", "package.json"]);
        git(&fixture.repo, &["commit", "-m", field]);
        let changed = bridge::changed_paths_since_base(&fixture.repo, &base_oid)
            .expect("changed manifest paths");

        assert_eq!(
            bridge::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
                .expect("manifest classification"),
            expected,
            "{field}"
        );
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_preserve_git_status_classes() {
    // Break caught: dropping a lockfile identity on modify, delete, rename, copy, or
    // type change can misclassify a runtime dependency graph change as irrelevant.
    for lockfile in [
        "package-lock.json",
        "npm-shrinkwrap.json",
        "pnpm-lock.yaml",
        "yarn.lock",
    ] {
        let fixture = GitFixture::new(&format!("npm-dependency-inputs-{lockfile}"));
        fs::write(fixture.repo.join(lockfile), "baseline\n").expect("baseline lockfile");
        git(&fixture.repo, &["add", lockfile]);
        git(&fixture.repo, &["commit", "-m", "baseline lockfile"]);
        let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
        fs::write(fixture.repo.join(lockfile), "changed\n").expect("changed lockfile");
        git(&fixture.repo, &["add", lockfile]);
        git(&fixture.repo, &["commit", "-m", "change lockfile"]);
        let changed = bridge::changed_paths_since_base(&fixture.repo, &base_oid)
            .expect("changed lockfile paths");

        assert!(
            bridge::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
                .expect("lockfile classification"),
            "{lockfile}"
        );
    }

    let deleted = GitFixture::new("npm-dependency-inputs-deleted");
    fs::write(deleted.repo.join("package.json"), r#"{"name":"fixture"}"#)
        .expect("baseline manifest");
    git(&deleted.repo, &["add", "package.json"]);
    git(&deleted.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&deleted.repo, &["rev-parse", "HEAD"]);
    git(&deleted.repo, &["rm", "package.json"]);
    git(&deleted.repo, &["commit", "-m", "delete manifest"]);
    let changed =
        bridge::changed_paths_since_base(&deleted.repo, &base_oid).expect("deleted manifest paths");
    assert!(
        bridge::npm_dependency_inputs_changed(&deleted.repo, &base_oid, &changed)
            .expect("deleted manifest classification")
    );

    let renamed = GitFixture::new("npm-dependency-inputs-renamed");
    fs::write(renamed.repo.join("package-lock.json"), "{}\n").expect("baseline lockfile");
    git(&renamed.repo, &["add", "package-lock.json"]);
    git(&renamed.repo, &["commit", "-m", "baseline lockfile"]);
    let base_oid = git_stdout(&renamed.repo, &["rev-parse", "HEAD"]);
    git(
        &renamed.repo,
        &["mv", "package-lock.json", "archived-lock.json"],
    );
    git(&renamed.repo, &["commit", "-m", "rename lockfile"]);
    let changed =
        bridge::changed_paths_since_base(&renamed.repo, &base_oid).expect("renamed lockfile paths");
    assert!(
        bridge::npm_dependency_inputs_changed(&renamed.repo, &base_oid, &changed)
            .expect("renamed lockfile classification")
    );

    let copied = GitFixture::new("npm-dependency-inputs-copied");
    fs::write(copied.repo.join("source-lock.json"), "{}\n").expect("baseline source");
    git(&copied.repo, &["add", "source-lock.json"]);
    git(&copied.repo, &["commit", "-m", "baseline source"]);
    let base_oid = git_stdout(&copied.repo, &["rev-parse", "HEAD"]);
    fs::copy(
        copied.repo.join("source-lock.json"),
        copied.repo.join("package-lock.json"),
    )
    .expect("copy lockfile");
    git(&copied.repo, &["add", "package-lock.json"]);
    git(&copied.repo, &["commit", "-m", "copy lockfile"]);
    let changed =
        bridge::changed_paths_since_base(&copied.repo, &base_oid).expect("copied lockfile paths");
    assert!(
        bridge::npm_dependency_inputs_changed(&copied.repo, &base_oid, &changed)
            .expect("copied lockfile classification")
    );

    let typed = GitFixture::new("npm-dependency-inputs-type-changed");
    fs::write(typed.repo.join("package-lock.json"), "{}\n").expect("baseline lockfile");
    git(&typed.repo, &["add", "package-lock.json"]);
    git(&typed.repo, &["commit", "-m", "baseline lockfile"]);
    let base_oid = git_stdout(&typed.repo, &["rev-parse", "HEAD"]);
    fs::remove_file(typed.repo.join("package-lock.json")).expect("remove lockfile");
    symlink("README.md", typed.repo.join("package-lock.json")).expect("symlink lockfile");
    git(&typed.repo, &["add", "package-lock.json"]);
    git(&typed.repo, &["commit", "-m", "type change lockfile"]);
    let changed = bridge::changed_paths_since_base(&typed.repo, &base_oid)
        .expect("type-changed lockfile paths");
    assert!(changed.type_changed.contains("package-lock.json"));
    assert!(
        bridge::npm_dependency_inputs_changed(&typed.repo, &base_oid, &changed)
            .expect("type-changed lockfile classification")
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_fail_closed_on_bad_evidence() {
    // Break caught: malformed, unsafe, unreadable, or unattributable package.json
    // evidence silently becoming an absent manifest and weakening classification.
    let fixture = GitFixture::new("npm-dependency-inputs-bad-evidence");
    fs::write(fixture.repo.join("package.json"), r#"{"name":"fixture"}"#)
        .expect("baseline manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);

    fs::write(fixture.repo.join("package.json"), b"{").expect("malformed manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "malformed manifest"]);
    let changed = bridge::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("malformed manifest paths");
    let error = bridge::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
        .expect_err("malformed JSON must fail closed");
    assert!(error.contains("parse current package.json:"), "{error}");

    fs::write(fixture.repo.join("package.json"), "[]\n").expect("non-object manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "non-object manifest"]);
    let changed = bridge::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("non-object manifest paths");
    assert_eq!(
        bridge::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
            .expect_err("non-object JSON must fail closed"),
        "current package.json is not a JSON object"
    );

    fs::write(fixture.repo.join("package.json"), r#"{"name":"changed"}"#)
        .expect("changed manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "changed manifest"]);
    let changed =
        bridge::changed_paths_since_base(&fixture.repo, &base_oid).expect("changed manifest paths");
    let manifest = fixture.repo.join("package.json");
    let mut permissions = fs::metadata(&manifest)
        .expect("manifest metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&manifest, permissions).expect("make manifest unreadable");
    let result = bridge::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed);
    let mut permissions = fs::metadata(&manifest)
        .expect("manifest metadata")
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&manifest, permissions).expect("restore manifest permissions");
    let error = result.expect_err("unreadable current manifest must fail closed");
    assert!(error.contains("read current package.json:"), "{error}");

    let error = bridge::npm_dependency_inputs_changed(&fixture.repo, "missing-base", &changed)
        .expect_err("missing base evidence must fail closed");
    assert!(error.contains("read base package.json:"), "{error}");

    fs::remove_file(&manifest).expect("remove regular manifest");
    symlink("README.md", &manifest).expect("unsafe manifest symlink");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "symlink manifest"]);
    let changed =
        bridge::changed_paths_since_base(&fixture.repo, &base_oid).expect("symlink manifest paths");
    let error = bridge::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
        .expect_err("unsafe manifest symlink must fail closed");
    assert!(error.contains("current package.json is unsafe:"), "{error}");
    assert!(error.contains("path contains a symlink"), "{error}");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_reject_manifest_swap_before_open() {
    // Break caught: validating package.json by path and reopening it lets a symlink swap
    // redirect the classifier to attacker-controlled dependency data.
    let fixture = GitFixture::new("npm-dependency-inputs-open-swap");
    let manifest = fixture.repo.join("package.json");
    fs::write(
        &manifest,
        r#"{"dependencies":{"fixture":"1"},"scripts":{"test":"old"}}"#,
    )
    .expect("baseline manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    fs::write(
        &manifest,
        r#"{"dependencies":{"fixture":"1"},"scripts":{"test":"new"}}"#,
    )
    .expect("scripts-only manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "scripts-only change"]);
    let changed =
        bridge::changed_paths_since_base(&fixture.repo, &base_oid).expect("changed manifest paths");
    bridge::NPM_MANIFEST_OPEN_FAILPOINT.store(1, Ordering::SeqCst);
    let repo = fixture.repo.clone();
    let classifier =
        thread::spawn(move || bridge::npm_dependency_inputs_changed(&repo, &base_oid, &changed));
    let deadline = Instant::now() + Duration::from_secs(5);
    while bridge::NPM_MANIFEST_OPEN_FAILPOINT.load(Ordering::SeqCst) != 2 {
        assert!(
            Instant::now() < deadline,
            "classifier did not reach open boundary"
        );
        thread::yield_now();
    }
    fs::rename(&manifest, fixture.repo.join("package.original.json"))
        .expect("move validated manifest");
    let attacker = fixture.root.join("attacker-package.json");
    fs::write(&attacker, r#"{"dependencies":{"fixture":"2"}}"#).expect("attacker manifest");
    symlink(&attacker, &manifest).expect("replace manifest with symlink");
    bridge::NPM_MANIFEST_OPEN_FAILPOINT.store(3, Ordering::SeqCst);

    let error = classifier
        .join()
        .expect("classifier thread")
        .expect_err("manifest swap must fail closed");
    assert!(error.contains("current package.json"), "{error}");
    bridge::NPM_MANIFEST_OPEN_FAILPOINT.store(0, Ordering::SeqCst);
}
