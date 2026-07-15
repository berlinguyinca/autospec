use autospec_core::coordination::{
    plan_repository_routing, RepositoryEvidence, RepositoryFinding, RepositoryRoutingInput,
};

fn repo(name: &str) -> RepositoryEvidence {
    RepositoryEvidence {
        name: name.to_string(),
        archived: false,
        pushed_at: None,
        readme: String::new(),
        module_paths: Vec::new(),
        packages: Vec::new(),
        dependency_references: Vec::new(),
        revival_requested: false,
    }
}

#[test]
fn active_recent_package_owner_beats_archived_split_successor() {
    let mut canonical = repo("acme/platform");
    canonical.pushed_at = Some("2026-07-01T00:00:00Z".to_string());
    canonical.readme = "Canonical home for acme-core packages.".to_string();
    canonical.module_paths = vec!["github.com/acme/platform".to_string()];
    canonical.packages = vec!["acme-core".to_string()];

    let mut split = repo("acme/platform-api");
    split.archived = true;
    split.pushed_at = Some("2023-01-01T00:00:00Z".to_string());
    split.readme = "Archived split successor. Use acme/platform instead.".to_string();
    split.module_paths = vec!["github.com/acme/platform/api".to_string()];
    split.packages = vec!["acme-core-api".to_string()];
    split.dependency_references = vec!["acme/platform".to_string()];

    let report = plan_repository_routing(&RepositoryRoutingInput {
        repositories: vec![split, canonical],
        findings: Vec::new(),
    });

    assert_eq!(report.canonical_targets[0].repository, "acme/platform");
    assert!(report.canonical_targets[0]
        .reasons
        .iter()
        .any(|reason| reason == "active_repository"));
    assert!(report.canonical_targets[0]
        .reasons
        .iter()
        .any(|reason| reason.starts_with("package:")));
    assert_eq!(
        report.do_not_file_by_default[0].repository,
        "acme/platform-api"
    );
    assert_eq!(
        report.do_not_file_by_default[0].reason,
        "archived_split_repository"
    );
}

#[test]
fn revival_requested_archived_repository_is_not_deferred_by_default() {
    let mut archived = repo("acme/legacy");
    archived.archived = true;
    archived.readme = "Archived repository, revival requested for maintenance.".to_string();
    archived.revival_requested = true;

    let report = plan_repository_routing(&RepositoryRoutingInput {
        repositories: vec![archived],
        findings: Vec::new(),
    });

    assert!(report.do_not_file_by_default.is_empty());
    assert_eq!(report.canonical_targets[0].repository, "acme/legacy");
}

#[test]
fn equal_fingerprint_findings_route_once_to_selected_canonical_repository() {
    let mut canonical = repo("acme/platform");
    canonical.pushed_at = Some("2026-07-01T00:00:00Z".to_string());
    canonical.packages = vec!["acme-core".to_string()];

    let mut archived = repo("acme/platform-api");
    archived.archived = true;
    archived.dependency_references = vec!["acme/platform".to_string()];

    let report = plan_repository_routing(&RepositoryRoutingInput {
        repositories: vec![archived, canonical],
        findings: vec![
            RepositoryFinding {
                repository: "acme/platform-api".to_string(),
                fingerprint: "fp-same".to_string(),
                title: "stale API docs".to_string(),
                evidence: "first archived evidence".to_string(),
            },
            RepositoryFinding {
                repository: "acme/platform".to_string(),
                fingerprint: "fp-same".to_string(),
                title: "stale API docs duplicate".to_string(),
                evidence: "second canonical evidence".to_string(),
            },
        ],
    });

    assert_eq!(report.routed_findings.len(), 1);
    assert_eq!(report.routed_findings[0].target_repository, "acme/platform");
    assert_eq!(report.routed_findings[0].fingerprint, "fp-same");
    assert_eq!(
        report.routed_findings[0].source_repository,
        "acme/platform-api"
    );
    assert_eq!(
        report.routed_findings[0].evidence,
        "first archived evidence"
    );
}
