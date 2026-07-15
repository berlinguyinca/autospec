use autospec_core::explore::{
    discover_specialists_json, scan_specialist_roster, SpecialistScanOptions,
};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn ranks_domains_deterministically_with_file_line_evidence() {
    let repo = temp_repo("explore-specialists-ranking");
    write(
        &repo.join("README.md"),
        "LC-MS metabolomics with mzML\nretention time alignment\nfeature table peak picking\nStripe billing ledger\n",
    );
    write(&repo.join("requirements.txt"), "ccxt\nbacktrader\n");

    let roster = scan_specialist_roster(&SpecialistScanOptions::new(&repo, 6)).unwrap();

    let domain_names = roster
        .domains
        .iter()
        .map(|domain| domain.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(domain_names[0], "ms-data");
    assert!(domain_names.contains(&"lc-binbase"));
    assert!(domain_names.contains(&"trading"));
    assert!(roster
        .domains
        .iter()
        .all(|domain| !domain.evidence.is_empty()));
    assert!(roster
        .domains
        .iter()
        .all(|domain| domain
            .evidence
            .iter()
            .all(|evidence| !evidence.file.is_empty()
                && evidence.line >= 1
                && !evidence.match_text.is_empty())));
}

#[test]
fn returns_empty_roster_for_generic_repo() {
    let repo = temp_repo("generic-repo");
    write(&repo.join("README.md"), "A generic task tracker.\n");

    let roster = scan_specialist_roster(&SpecialistScanOptions::new(&repo, 3)).unwrap();

    assert!(roster.domains.is_empty());
    assert!(roster.suggested_specialists.is_empty());
}

#[test]
fn caps_suggested_specialists_at_requested_count_and_six() {
    let repo = temp_repo("metabolomics-trading-healthcare-payments-infra-ml-security");
    write(
        &repo.join("README.md"),
        "mzML InChI retention index MoNA SLURM ccxt FHIR Stripe PyTorch OAuth Kubernetes\n",
    );

    let two = scan_specialist_roster(&SpecialistScanOptions::new(&repo, 2)).unwrap();
    let above_cap = scan_specialist_roster(&SpecialistScanOptions::new(&repo, 99)).unwrap();

    assert_eq!(two.suggested_specialists.len(), 2);
    assert_eq!(above_cap.suggested_specialists.len(), 6);
}

#[test]
fn reuses_valid_cache_without_rescanning_by_default() {
    let repo = temp_repo("explore-specialists-cache");
    write(&repo.join("README.md"), "ccxt trading\n");
    let cache = repo.join(".autospec/explore-specialists.json");
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    let cached = "{\"schema_version\":1,\"domains\":[],\"suggested_specialists\":[{\"slug\":\"cached-specialist\",\"persona\":\"Cached\",\"lens\":\"L\",\"why\":\"W\",\"evidence\":\"E\"}]}\n";
    write(&cache, cached);

    let output = discover_specialists_json(&SpecialistScanOptions::new(&repo, 3)).unwrap();

    assert_eq!(output, cached);
}

#[test]
fn force_refresh_replaces_valid_cache() {
    let repo = temp_repo("explore-specialists-force");
    write(&repo.join("README.md"), "ccxt trading\n");
    let cache = repo.join(".autospec/explore-specialists.json");
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    write(
        &cache,
        "{\"schema_version\":1,\"domains\":[],\"suggested_specialists\":[]}",
    );

    let mut options = SpecialistScanOptions::new(&repo, 3);
    options.force = true;
    let output = discover_specialists_json(&options).unwrap();

    assert!(output.contains("trading-specialist"));
    assert_eq!(std::fs::read_to_string(cache).unwrap(), output);
}

fn temp_repo(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    path.push(format!("{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&path).expect("temp repo");
    path
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}
