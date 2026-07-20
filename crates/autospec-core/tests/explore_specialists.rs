use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::explore::specialists::{
    collect_strict_domains, scan_specialists, ScanOptions, StrictCollectorErrorCode,
    StrictCollectorOptions,
};

#[test]
fn trading_manifest_records_file_line_evidence_and_ranked_specialist() {
    let repo = temp_repo("trading-manifest");
    fs::write(
        repo.join("requirements.txt"),
        "flask==2.0\nccxt>=4.0\nbacktrader==1.9.78\n",
    )
    .unwrap();

    let roster = scan_specialists(&ScanOptions::new(&repo).with_num_specialists(6)).unwrap();

    let trading = roster
        .domains
        .iter()
        .find(|domain| domain.name == "trading")
        .expect("trading domain");
    assert_eq!(trading.score, trading.evidence.len());
    assert!(trading
        .evidence
        .iter()
        .any(|e| { e.file == "requirements.txt" && e.line == 2 && e.r#match == "ccxt>=4.0" }));
    assert!(roster
        .suggested_specialists
        .iter()
        .any(|s| s.slug == "trading-specialist" && s.evidence == "requirements.txt:2 (ccxt>=4.0)"));
}

#[test]
fn generic_repo_has_empty_domains_and_specialists() {
    let repo = temp_repo("generic-widget");
    fs::write(repo.join("requirements.txt"), "flask==2.0\nrequests>=2.0\n").unwrap();
    fs::write(
        repo.join("README.md"),
        "# Generic widget app\nPlain web app.\n",
    )
    .unwrap();

    let roster = scan_specialists(&ScanOptions::new(&repo)).unwrap();

    assert!(roster.domains.is_empty(), "{roster:?}");
    assert!(roster.suggested_specialists.is_empty(), "{roster:?}");
}

#[test]
fn specialist_cap_is_clamped_to_six_and_deterministic() {
    let repo = temp_repo("multi-domain");
    fs::write(
        repo.join("README.md"),
        "ccxt stripe fhir pytorch oauth kubernetes metabolomics inchi binbase mona slurm\n",
    )
    .unwrap();

    let first = scan_specialists(&ScanOptions::new(&repo).with_num_specialists(99)).unwrap();
    let second = scan_specialists(&ScanOptions::new(&repo).with_num_specialists(99)).unwrap();

    assert!(first.suggested_specialists.len() <= 6);
    assert_eq!(first, second);
}

#[test]
fn cache_reuse_and_force_refresh_are_owned_by_core() {
    let repo = temp_repo("cache-repo");
    fs::write(repo.join("requirements.txt"), "ccxt>=4.0\n").unwrap();

    let first = scan_specialists(&ScanOptions::new(&repo)).unwrap();
    assert_eq!(first.schema_version, 1);

    fs::write(repo.join("requirements.txt"), "flask==2.0\n").unwrap();
    let reused = scan_specialists(&ScanOptions::new(&repo)).unwrap();
    assert_eq!(first, reused, "valid cache should be reused without force");

    let refreshed = scan_specialists(&ScanOptions::new(&repo).force(true)).unwrap();
    assert!(
        refreshed.domains.is_empty(),
        "force should replace cache: {refreshed:?}"
    );
    assert!(repo.join(".autospec/explore-specialists.json").is_file());
}

#[test]
fn malformed_cache_is_regenerated() {
    let repo = temp_repo("malformed-cache");
    fs::create_dir_all(repo.join(".autospec")).unwrap();
    fs::write(
        repo.join(".autospec/explore-specialists.json"),
        r#"{"schema_version":1,"domains":"wrong","suggested_specialists":[]}"#,
    )
    .unwrap();
    fs::write(repo.join("requirements.txt"), "ccxt>=4.0\n").unwrap();

    let roster = scan_specialists(&ScanOptions::new(&repo)).unwrap();

    assert!(roster.domains.iter().any(|domain| domain.name == "trading"));
    let persisted = fs::read_to_string(repo.join(".autospec/explore-specialists.json")).unwrap();
    assert!(persisted.contains("\"domains\": ["), "{persisted}");
}

#[test]
fn cache_with_optional_generated_at_is_reused() {
    let repo = temp_repo("generated-at-cache");
    fs::create_dir_all(repo.join(".autospec")).unwrap();
    fs::write(
        repo.join(".autospec/explore-specialists.json"),
        r#"{"schema_version":1,"generated_at":"2026-07-15T00:00:00Z","domains":[],"suggested_specialists":[{"slug":"cached","persona":"Cached","lens":"reuse","why":"cache","evidence":"cache:1"}]}"#,
    )
    .unwrap();
    fs::write(repo.join("requirements.txt"), "ccxt>=4.0\n").unwrap();

    let roster = scan_specialists(&ScanOptions::new(&repo)).unwrap();

    assert_eq!(roster.suggested_specialists[0].slug, "cached");
}

#[test]
fn strict_collector_is_deterministic_and_ranks_domains_and_evidence() {
    let repo = temp_repo("trading-strict-order");
    fs::write(
        repo.join("README.md"),
        "ccxt stripe\nccxt oauth\nbacktrader kubernetes\n",
    )
    .unwrap();
    fs::write(repo.join("requirements.txt"), "ccxt>=4\nstripe>=1\n").unwrap();

    let first = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap();
    let second = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.schema_version, 1);
    assert_eq!(first.collector_version, "strict-local-v1");
    assert_eq!(
        first.canonical_repo_scope,
        fs::canonicalize(&repo)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    );
    assert_eq!(
        first
            .domains
            .iter()
            .map(|domain| domain.name.as_str())
            .collect::<Vec<_>>(),
        vec!["trading", "payments", "infra", "security"]
    );
    assert_eq!(
        first.domains[0]
            .evidence
            .iter()
            .map(|evidence| (evidence.file.as_str(), evidence.line))
            .collect::<Vec<_>>(),
        vec![
            (".", 1),
            ("README.md", 1),
            ("README.md", 2),
            ("README.md", 3),
            ("requirements.txt", 1),
        ]
    );
}

#[test]
fn strict_collector_excludes_generated_and_vendor_trees() {
    let repo = temp_repo("strict-exclusions");
    fs::write(repo.join("README.md"), "plain application\n").unwrap();
    for directory in ["node_modules", ".next", "coverage", "out"] {
        fs::create_dir_all(repo.join(directory)).unwrap();
        fs::write(
            repo.join(directory).join("README.md"),
            "ccxt stripe oauth kubernetes\n",
        )
        .unwrap();
    }

    let evidence = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap();
    let files = evidence
        .domains
        .iter()
        .flat_map(|domain| domain.evidence.iter().map(|item| item.file.as_str()))
        .collect::<Vec<_>>();
    assert!(files.iter().all(|file| {
        !["node_modules/", ".next/", "coverage/", "out/"]
            .iter()
            .any(|prefix| file.starts_with(prefix))
    }));
}

#[test]
fn strict_collector_accepts_a_valid_zero_domain_snapshot() {
    let repo = temp_repo("strict-empty");
    fs::write(
        repo.join("README.md"),
        "# Generic widget\nPlain application.\n",
    )
    .unwrap();

    let evidence = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap();

    assert!(evidence.domains.is_empty(), "{evidence:?}");
}

#[test]
fn strict_collector_rejects_invalid_utf8_in_a_selected_manifest() {
    let repo = temp_repo("strict-invalid-utf8");
    fs::write(repo.join("requirements.txt"), [0xff, b'\n']).unwrap();

    let error = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap_err();

    assert_eq!(error.code, StrictCollectorErrorCode::InvalidUtf8);
    assert!(error.detail.contains("requirements.txt"));
}

#[test]
fn strict_collector_rejects_a_selected_input_that_is_not_a_regular_file() {
    let repo = temp_repo("strict-unreadable");
    fs::create_dir(repo.join("Cargo.toml")).unwrap();

    let error = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap_err();

    assert_eq!(error.code, StrictCollectorErrorCode::ReadFile);
    assert!(error.detail.contains("Cargo.toml"));
}

#[test]
fn strict_collector_rejects_a_nondefault_depth_policy() {
    let repo = temp_repo("strict-depth");
    let mut options = StrictCollectorOptions::new(&repo);
    options.max_depth = 2;

    let error = collect_strict_domains(&options).unwrap_err();

    assert_eq!(error.code, StrictCollectorErrorCode::InvalidCollectorSchema);
    assert!(error.detail.contains("max_depth"));
}

#[cfg(unix)]
#[test]
fn strict_collector_rejects_a_root_escaping_symlink() {
    use std::os::unix::fs::symlink;

    let repo = temp_repo("strict-symlink");
    let outside = temp_repo("strict-outside");
    fs::write(outside.join("requirements.txt"), "ccxt>=4\n").unwrap();
    symlink(
        outside.join("requirements.txt"),
        repo.join("requirements.txt"),
    )
    .unwrap();

    let error = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap_err();

    assert_eq!(error.code, StrictCollectorErrorCode::PathEscapesRoot);
    assert!(error.detail.contains("requirements.txt"));
}

#[cfg(unix)]
#[test]
fn strict_collector_rejects_a_non_utf8_canonical_root_scope() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let parent = temp_repo("strict-non-utf8");
    let repo = parent
        .join(OsString::from_vec(b"scope-\xff".to_vec()))
        .join("repo");
    if fs::create_dir_all(&repo).is_err() {
        return;
    }

    let error = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap_err();

    assert_eq!(error.code, StrictCollectorErrorCode::InvalidRoot);
    assert!(error.detail.contains("UTF-8"));
}

#[cfg(unix)]
#[test]
fn strict_collector_rejects_a_fifo_selected_input_without_reading_it() {
    let repo = temp_repo("strict-fifo");
    let fifo = repo.join("requirements.txt");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("create fifo");
    assert!(status.success(), "mkfifo status: {status}");

    let error = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap_err();

    assert_eq!(error.code, StrictCollectorErrorCode::ReadFile);
    assert!(error.detail.contains("requirements.txt"));
}

#[test]
fn strict_collector_caps_evidence_at_eight_rows_per_domain() {
    let repo = temp_repo("strict-evidence-cap");
    let document = (1..=12)
        .map(|line| format!("ccxt signal {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(repo.join("requirements.txt"), document).unwrap();

    let evidence = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap();
    let trading = evidence
        .domains
        .iter()
        .find(|domain| domain.name == "trading")
        .expect("trading domain");

    assert_eq!(trading.score, 8);
    assert_eq!(trading.evidence.len(), 8);
    assert_eq!(trading.evidence[7].line, 8);
    assert!(trading
        .evidence
        .iter()
        .all(|item| item.r#match.chars().count() <= 120));
}

#[test]
fn strict_collector_ignores_legacy_cache_without_writing_or_environment_authority() {
    let repo = temp_repo("strict-no-authority");
    fs::create_dir_all(repo.join(".autospec")).unwrap();
    let cache = repo.join(".autospec/explore-specialists.json");
    let cached = r#"{"schema_version":1,"domains":[{"name":"trading","score":1,"evidence":[{"file":"cache","line":1,"match":"ccxt"}]}],"suggested_specialists":[]}"#;
    fs::write(&cache, cached).unwrap();
    fs::write(repo.join("README.md"), "Plain generic application.\n").unwrap();

    let evidence = collect_strict_domains(&StrictCollectorOptions::new(&repo)).unwrap();
    let strict_source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/explore/specialists/strict.rs"),
    )
    .unwrap();
    let strict_production = strict_source
        .split("#[cfg(test)]")
        .next()
        .expect("production strict collector source");

    assert!(evidence.domains.is_empty(), "{evidence:?}");
    assert_eq!(fs::read_to_string(cache).unwrap(), cached);
    assert!(!strict_production.contains("std::env"));
    assert!(!strict_production.contains("AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT"));
    assert!(!strict_production.contains("fs::write"));
}

fn temp_repo(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    path.push(format!(
        "autospec-core-{name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
