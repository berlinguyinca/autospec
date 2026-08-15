// executor_bridge tests: license / checker — 7 cases.
//
// Split out of tests.rs; see the note in that file.

use super::support_base::{git, git_stdout, test_environment, write_executable, GitFixture};
use super::support_launch::completed_generation_bundle;
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

fn scanner_fixtures_with_license(root: &Path, license_report: &str) -> bridge::ScannerExecutables {
    let bin = root.join("license-scanner-bin");
    fs::create_dir_all(&bin).expect("scanner bin");
    let gitleaks = bin.join("gitleaks");
    write_executable(
        &gitleaks,
        "#!/bin/sh\nset -eu\nreport=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = --report-path ]; then report=\"$2\"; shift 2; else shift; fi\ndone\nprintf '%s' '[]' > \"$report\"\n",
    );
    let semgrep = bin.join("semgrep");
    write_executable(
        &semgrep,
        "#!/bin/sh\nset -eu\nprintf '%s\\n' '{\"results\":[],\"errors\":[],\"paths\":{\"scanned\":[\"package.json\"],\"skipped\":[]}}'\n",
    );
    let trivy = bin.join("trivy");
    write_executable(
        &trivy,
        "#!/bin/sh\nset -eu\nprintf '%s\\n' '{\"Results\":[{\"Target\":\".\"}]}'\n",
    );
    let license_checker = bin.join("license-checker");
    write_executable(
        &license_checker,
        &format!("#!/bin/sh\nset -eu\nprintf '%s\\n' '{license_report}'\n"),
    );
    bridge::ScannerExecutables::from_paths(BTreeMap::from([
        ("gitleaks".to_string(), gitleaks),
        ("semgrep".to_string(), semgrep),
        ("trivy".to_string(), trivy),
        ("license-checker".to_string(), license_checker),
    ]))
    .expect("scanner paths")
}

#[test]
fn autonomous_executor_bridge_trivy_findings_are_scoped_to_changed_targets() {
    assert!(bridge::scanner_terminal_is_accepted(
        "trivy",
        &bridge::AttemptTerminal::Exited(1)
    ));
    assert!(!bridge::scanner_terminal_is_accepted(
        "semgrep",
        &bridge::AttemptTerminal::Exited(1)
    ));
    assert!(!bridge::scanner_terminal_is_accepted(
        "trivy",
        &bridge::AttemptTerminal::Signaled(9)
    ));
    let fixture = GitFixture::new("trivy-changed-targets");
    let command = bridge::scanner_command(
        "trivy",
        Path::new("/scanner/trivy"),
        &fixture.repo,
        "base-oid",
        Path::new("/safe/gitleaks-policy.toml"),
        Path::new("/safe/gitleaks-result.json"),
    )
    .expect("Trivy command");
    assert_eq!(command.accepted_exit_codes, vec![0, 1]);
    fs::write(fixture.repo.join("package-lock.json"), "{}\n").expect("baseline lockfile");
    git(&fixture.repo, &["add", "package-lock.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline lockfile"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    fs::write(fixture.repo.join("feature.js"), "feature();\n").expect("feature source");
    git(&fixture.repo, &["add", "feature.js"]);
    git(&fixture.repo, &["commit", "-m", "feature source"]);
    let report = serde_json::json!({
        "SchemaVersion": 2,
        "Results": [{
            "Target": "package-lock.json",
            "Vulnerabilities": [{"VulnerabilityID": "CVE-1"}]
        }]
    });
    let changed =
        bridge::changed_paths_since_base(&fixture.repo, &base_oid).expect("changed paths");
    let filtered = bridge::filter_trivy_result_for_changes(&report, &fixture.repo, &changed)
        .expect("filter unchanged lockfile finding");
    bridge::validate_trivy_transport(1, &serde_json::to_vec(&report).expect("raw report"), b"")
        .expect("raw Trivy findings justify exit 1");
    bridge::validate_scanner_result(
        "trivy",
        0,
        &serde_json::to_vec(&filtered).expect("filtered report"),
        b"",
    )
    .expect("unchanged lockfile finding must not block feature work");

    fs::write(
        fixture.repo.join("package-lock.json"),
        "{\"changed\":true}\n",
    )
    .expect("changed lockfile");
    git(&fixture.repo, &["add", "package-lock.json"]);
    git(&fixture.repo, &["commit", "-m", "change lockfile"]);
    let changed =
        bridge::changed_paths_since_base(&fixture.repo, &base_oid).expect("changed paths");
    let filtered = bridge::filter_trivy_result_for_changes(&report, &fixture.repo, &changed)
        .expect("filter changed lockfile finding");
    let error = bridge::validate_scanner_result(
        "trivy",
        0,
        &serde_json::to_vec(&filtered).expect("filtered report"),
        b"",
    )
    .expect_err("changed lockfile finding must block");
    assert!(error.contains("reported findings"), "{error}");

    let unsafe_report = serde_json::json!({
        "SchemaVersion": 2,
        "Results": [{
            "Target": "../outside/package-lock.json",
            "Vulnerabilities": [{"VulnerabilityID": "CVE-1"}]
        }]
    });
    let error = bridge::filter_trivy_result_for_changes(&unsafe_report, &fixture.repo, &changed)
        .expect_err("unsafe Trivy target must fail closed");
    assert!(error.contains("unsafe target"), "{error}");

    for target in ["missing/package-lock.json", "src"] {
        if target == "src" {
            fs::create_dir_all(fixture.repo.join(target)).expect("directory target");
        }
        let unknown_report = serde_json::json!({
            "SchemaVersion": 2,
            "Results": [{
                "Target": target,
                "Vulnerabilities": [{"VulnerabilityID": "CVE-1"}]
            }]
        });
        let error =
            bridge::filter_trivy_result_for_changes(&unknown_report, &fixture.repo, &changed)
                .expect_err("unattributable Trivy target must fail closed");
        assert!(error.contains("unattributable target"), "{error}");
    }

    let whitespace_path = " changed-lock.json ";
    fs::write(fixture.repo.join(whitespace_path), "{}\n").expect("whitespace target");
    git(&fixture.repo, &["add", whitespace_path]);
    git(&fixture.repo, &["commit", "-m", "whitespace target"]);
    let changed =
        bridge::changed_paths_since_base(&fixture.repo, &base_oid).expect("changed paths");
    let whitespace_report = serde_json::json!({
        "SchemaVersion": 2,
        "Results": [{
            "Target": whitespace_path,
            "Vulnerabilities": [{"VulnerabilityID": "CVE-1"}]
        }]
    });
    let filtered =
        bridge::filter_trivy_result_for_changes(&whitespace_report, &fixture.repo, &changed)
            .expect("whitespace target remains attributable");
    assert!(
        bridge::validate_scanner_result(
            "trivy",
            0,
            &serde_json::to_vec(&filtered).expect("filtered report"),
            b"",
        )
        .is_err(),
        "changed whitespace target finding must remain"
    );

    let metacharacter_path = "*";
    fs::write(fixture.repo.join(metacharacter_path), "{}\n")
        .expect("untracked pathspec metacharacter target");
    let metacharacter_report = serde_json::json!({
        "SchemaVersion": 2,
        "Results": [{
            "Target": metacharacter_path,
            "Vulnerabilities": [{"VulnerabilityID": "CVE-1"}]
        }]
    });
    let error =
        bridge::filter_trivy_result_for_changes(&metacharacter_report, &fixture.repo, &changed)
            .expect_err("an untracked pathspec metacharacter must not match tracked files");
    assert!(error.contains("unattributable target"), "{error}");

    let error = bridge::validate_trivy_transport(1, br#"{"SchemaVersion":2,"Results":[]}"#, b"")
        .expect_err("Trivy exit 1 requires native findings");
    assert!(error.contains("without native findings"), "{error}");
}

#[test]
fn autonomous_executor_bridge_license_checker_admits_only_unchanged_graph_findings() {
    let fixture = GitFixture::new("license-unchanged-graph");
    fs::write(
        fixture.repo.join("package.json"),
        r#"{"dependencies":{"fixture":"1.0.0"},"scripts":{"test":"old"}}"#,
    )
    .expect("baseline manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    fs::write(
        fixture.repo.join("package.json"),
        r#"{"dependencies":{"fixture":"1.0.0"},"scripts":{"test":"new"}}"#,
    )
    .expect("script-only manifest change");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "change script"]);
    let scanners = scanner_fixtures_with_license(
        &fixture.root,
        r#"{"fixture@1.0.0":{"licenses":"LGPL-3.0"}}"#,
    );

    let error = bridge::run_required_scanners(
        &fixture.repo,
        "HEAD",
        &fixture.root.join("symbolic-security"),
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect_err("symbolic scanner base must fail closed");
    assert!(error.contains("canonical"), "{error}");
    let observed = bridge::run_required_scanners(
        &fixture.repo,
        &base_oid,
        &fixture.root.join("security"),
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect("pre-existing forbidden license with unchanged graph");
    let license = observed
        .iter()
        .find(|scanner| scanner.name == "license-checker")
        .expect("license-checker observation");

    assert_eq!(license.result_path, license.command.stdout_path);
    assert_eq!(
        license
            .result_path
            .strip_prefix(&fixture.root)
            .expect("fixture-relative result"),
        Path::new("security/license-checker/process/command-000.stdout")
    );
    let result: serde_json::Value =
        serde_json::from_slice(&fs::read(&license.result_path).expect("private license result"))
            .expect("native license-checker JSON");
    assert_eq!(result["fixture@1.0.0"]["licenses"], "LGPL-3.0");
    let mut wrong_base = observed.clone();
    let head_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    wrong_base
        .iter_mut()
        .for_each(|scanner| scanner.base_oid = head_oid.clone());
    let error = bridge::validate_observed_scanners(&fixture.repo, &base_oid, &wrong_base)
        .expect_err("uniform wrong scanner base must fail closed");
    assert!(error.contains("expected base"), "{error}");
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&license.result_path)
            .expect("license result metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn autonomous_executor_bridge_license_checker_rejects_changed_graph_finding() {
    let fixture = GitFixture::new("license-changed-graph");
    fs::write(fixture.repo.join("package-lock.json"), "{}\n").expect("baseline lockfile");
    git(&fixture.repo, &["add", "package-lock.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline lockfile"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    fs::write(
        fixture.repo.join("package-lock.json"),
        "{\"changed\":true}\n",
    )
    .expect("changed lockfile");
    git(&fixture.repo, &["add", "package-lock.json"]);
    git(&fixture.repo, &["commit", "-m", "change lockfile"]);
    let scanners = scanner_fixtures_with_license(
        &fixture.root,
        r#"{"fixture@1.0.0":{"licenses":"LGPL-3.0"}}"#,
    );

    let error = bridge::run_required_scanners(
        &fixture.repo,
        &base_oid,
        &fixture.root.join("security"),
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect_err("changed dependency graph must enforce forbidden-license policy");

    assert!(error.contains("forbidden license"), "{error}");
}

#[test]
fn autonomous_executor_bridge_license_checker_rejects_malformed_preexisting_record() {
    let fixture = GitFixture::new("license-malformed-record");
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let scanners =
        scanner_fixtures_with_license(&fixture.root, r#"{"fixture@1.0.0":{"licenses":null}}"#);

    let error = bridge::run_required_scanners(
        &fixture.repo,
        &base_oid,
        &fixture.root.join("security"),
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect_err("malformed record must fail closed on an unchanged graph");

    assert!(error.contains("missing licenses"), "{error}");
}

#[test]
fn autonomous_executor_bridge_license_checker_persisted_base_identity_fails_closed() {
    let _environment = test_environment();
    let fixture = GitFixture::new("license-persisted-base");
    git(
        &fixture.repo,
        &["commit", "--allow-empty", "-m", "expected base"],
    );
    let wrong_base = git_stdout(&fixture.repo, &["rev-parse", "HEAD^"]);
    let commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let lane =
        bridge::PremergeLaneIdentity::new("test/repo", 42, "worker", "claim", "main", commit)
            .expect("lane");
    let lane_root = fixture.root.join("lane");
    bridge::ensure_private_directory(&lane_root).expect("lane root");
    let bundle =
        completed_generation_bundle(&fixture, &lane, &lane_root, 1, &fixture.root.join("count"));
    let manifest: serde_json::Value =
        serde_json::from_str(bundle.manifest_body()).expect("observed manifest");

    for mutation in ["manifest", "missing", "mixed", "uniform"] {
        let mut malformed = manifest.clone();
        if mutation == "manifest" {
            malformed
                .as_object_mut()
                .expect("manifest")
                .remove("base_oid");
        } else {
            let scanners = malformed["scanners"]
                .as_array_mut()
                .expect("mutable scanners");
            if mutation == "missing" {
                scanners[0]
                    .as_object_mut()
                    .expect("scanner object")
                    .remove("base_oid");
            } else if mutation == "mixed" {
                scanners[0]["base_oid"] = serde_json::Value::String("foreign-base".to_string());
            } else {
                for scanner in scanners {
                    scanner["base_oid"] = serde_json::Value::String(wrong_base.clone());
                }
            }
        }
        let error = bridge::validate_persisted_observed_manifest(
            &fixture.repo,
            &bundle.artifact_root,
            &bundle.attempt_root,
            &malformed,
            &bundle.qa,
            &bundle.security,
            &bundle.intent_digest,
        )
        .expect_err("missing or mixed persisted base identity must fail closed");
        assert!(error.contains("base"), "{mutation}: {error}");
    }
}

#[test]
fn autonomous_executor_bridge_gitleaks_ignores_only_next_generated_output() {
    // Break caught: generated Next.js bundles replaying source-like test secrets into the
    // required scan while an equivalent finding in a source fixture must still block.
    let fixture = GitFixture::new("gitleaks-next-policy");
    fs::write(
        fixture.repo.join(".gitleaks.toml"),
        "title = \"Autospec test policy\"\n\
         [[rules]]\n\
         id = \"autospec-test-secret\"\n\
         description = \"harmless test marker\"\n\
         regex = '''AUTOSPEC_TEST_SECRET_[A-Z]+'''\n",
    )
    .expect("test Gitleaks config");
    let generated = fixture.repo.join(".next/cache");
    let source = fixture.repo.join("fixtures/cache");
    fs::create_dir_all(&generated).expect("generated Next.js cache");
    fs::create_dir_all(&source).expect("source fixture cache");
    let token = "AUTOSPEC_TEST_SECRET_ALPHA";
    fs::write(generated.join("bundle.js"), format!("{token}\n")).expect("generated secret fixture");
    fs::write(source.join("source.js"), format!("{token}\n")).expect("source secret fixture");
    let gitleaks = bridge::resolve_direct_executable(&fixture.repo, "gitleaks")
        .expect("real gitleaks")
        .program;
    let scanners = bridge::ScannerExecutables::from_paths(
        ["gitleaks", "semgrep", "trivy", "license-checker"]
            .into_iter()
            .map(|scanner| (scanner.to_string(), gitleaks.clone()))
            .collect(),
    )
    .expect("scanner paths");
    let artifact_root = fixture.root.join("scanner-evidence");

    let error = bridge::run_required_scanners(
        &fixture.repo,
        &git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
        &artifact_root,
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect_err("source finding must block the required scan");
    assert!(error.contains("gitleaks reported findings"), "{error}");
    let report = artifact_root.join("gitleaks/result.json");
    let findings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&report).expect("durable Gitleaks finding report"),
    )
    .expect("Gitleaks JSON report");
    let paths = findings
        .as_array()
        .expect("Gitleaks findings array")
        .iter()
        .map(|finding| {
            finding
                .get("File")
                .and_then(serde_json::Value::as_str)
                .expect("finding file")
        })
        .collect::<Vec<_>>();

    assert!(
        paths
            .iter()
            .any(|path| path.ends_with("fixtures/cache/source.js")),
        "{paths:?}"
    );
    assert!(
        paths.iter().all(|path| !path.contains("/.next/")),
        "{paths:?}"
    );
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(report)
            .expect("durable report metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn autonomous_executor_bridge_gitleaks_preserves_repository_rules() {
    // Break caught: the generated exclusion policy replacing a repository's custom rules
    // instead of extending them.
    let fixture = GitFixture::new("gitleaks-repository-policy");
    fs::write(
        fixture.repo.join(".gitleaks.toml"),
        "title = \"Repository policy\"\n\
         [[rules]]\n\
         id = \"autospec-repository-secret\"\n\
         description = \"repository secret fixture\"\n\
         regex = '''AUTOSPEC_CUSTOM_SECRET_[A-Z]+'''\n",
    )
    .expect("repository Gitleaks config");
    fs::create_dir_all(fixture.repo.join(".next/cache")).expect("generated Next.js cache");
    fs::create_dir_all(fixture.repo.join("fixtures/cache")).expect("source fixture cache");
    fs::write(
        fixture.repo.join(".next/cache/bundle.js"),
        "AUTOSPEC_CUSTOM_SECRET_GENERATED\n",
    )
    .expect("generated custom-rule fixture");
    fs::write(
        fixture.repo.join("fixtures/cache/source.js"),
        "AUTOSPEC_CUSTOM_SECRET_SOURCE\n",
    )
    .expect("source custom-rule fixture");
    let gitleaks = bridge::resolve_direct_executable(&fixture.repo, "gitleaks")
        .expect("real gitleaks")
        .program;
    let scanners = bridge::ScannerExecutables::from_paths(
        ["gitleaks", "semgrep", "trivy", "license-checker"]
            .into_iter()
            .map(|scanner| (scanner.to_string(), gitleaks.clone()))
            .collect(),
    )
    .expect("scanner paths");
    let artifact_root = fixture.root.join("scanner-evidence");

    bridge::run_required_scanners(
        &fixture.repo,
        &git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
        &artifact_root,
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect_err("repository source finding must block");
    let findings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join("gitleaks/result.json"))
            .expect("repository-rule report"),
    )
    .expect("repository-rule JSON");
    let findings = findings.as_array().expect("Gitleaks findings array");

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(
        findings[0]
            .get("RuleID")
            .and_then(serde_json::Value::as_str),
        Some("autospec-repository-secret")
    );
    assert!(
        findings[0]
            .get("File")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|path| path.ends_with("fixtures/cache/source.js")),
        "{findings:?}"
    );
}
