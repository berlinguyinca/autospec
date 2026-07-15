use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::explore::specialists::{scan_specialists, ScanOptions};

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
        .any(|e| { e.file == "requirements.txt" && e.line == 2 && e.r#match.contains("ccxt") }));
    assert!(roster
        .suggested_specialists
        .iter()
        .any(|s| s.slug == "trading-specialist" && s.evidence.contains("requirements.txt:2")));
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
