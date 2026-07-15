use autospec_core::exploration::{
    route_repositories, ExplorationInput, Finding, RepositoryEvidence,
};

macro_rules! repository {
    ($name:expr, $family:expr, $archived:expr, $revival_requested:expr, $pushed_at:expr, $readme:expr, $module_paths:expr, $packages:expr, $dependency_references:expr $(,)?) => {
        repository_from_parts(
            $name,
            $family,
            $archived,
            $revival_requested,
            $pushed_at,
            contents($readme, $module_paths, $packages, $dependency_references),
        )
    };
}

struct RepositoryContents<'a> {
    readme: &'a str,
    module_paths: &'a [&'a str],
    packages: &'a [&'a str],
    dependency_references: &'a [&'a str],
}

fn contents<'a>(
    readme: &'a str,
    module_paths: &'a [&'a str],
    packages: &'a [&'a str],
    dependency_references: &'a [&'a str],
) -> RepositoryContents<'a> {
    RepositoryContents {
        readme,
        module_paths,
        packages,
        dependency_references,
    }
}

fn repository_from_parts(
    name: &str,
    family: &str,
    archived: bool,
    revival_requested: bool,
    pushed_at: &str,
    contents: RepositoryContents<'_>,
) -> RepositoryEvidence {
    RepositoryEvidence {
        name: name.to_string(),
        family: family.to_string(),
        archived,
        revival_requested,
        pushed_at: pushed_at.to_string(),
        readme: contents.readme.to_string(),
        module_paths: contents
            .module_paths
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        packages: contents
            .packages
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        dependency_references: contents
            .dependency_references
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
    }
}

fn finding(repository: &str, fingerprint: &str, title: &str) -> Finding {
    Finding {
        repository: repository.to_string(),
        fingerprint: fingerprint.to_string(),
        title: title.to_string(),
    }
}

#[test]
fn routes_an_archived_split_repository_to_the_active_family_target() {
    let report = route_repositories(&ExplorationInput {
        repositories: vec![
            repository!(
                "metabolomics-us/go-modules",
                "go",
                false,
                false,
                "2026-07-14T00:00:00Z",
                "active umbrella",
                &["example.com/go-modules"],
                &["admin"],
                &[],
            ),
            repository!(
                "metabolomics-us/go-admin",
                "go",
                true,
                false,
                "2025-01-01T00:00:00Z",
                "",
                &[],
                &[],
                &[],
            ),
        ],
        findings: vec![finding("metabolomics-us/go-admin", "lint-x", "Fix lint")],
    })
    .expect("repository evidence routes");

    assert_eq!(report.canonical_targets.len(), 1);
    assert_eq!(report.canonical_targets[0].family, "go");
    assert_eq!(
        report.canonical_targets[0].repository,
        "metabolomics-us/go-modules"
    );
    assert_eq!(
        report.do_not_file_by_default,
        vec!["metabolomics-us/go-admin"]
    );
    assert_eq!(report.routed_findings.len(), 1);
    assert_eq!(
        report.routed_findings[0].canonical_target,
        "metabolomics-us/go-modules"
    );
    assert!(!report.routed_findings[0].duplicate);
    assert!(report.deferred_findings.is_empty());
}

#[test]
fn marks_later_fingerprint_copies_for_the_same_canonical_target_as_duplicates() {
    let report = route_repositories(&ExplorationInput {
        repositories: vec![
            repository!(
                "metabolomics-us/go-modules",
                "go",
                false,
                false,
                "2026-07-14T00:00:00Z",
                "active umbrella",
                &["example.com/go-modules"],
                &[],
                &[],
            ),
            repository!(
                "metabolomics-us/go-admin",
                "go",
                false,
                false,
                "2026-07-13T00:00:00Z",
                "",
                &[],
                &[],
                &[],
            ),
        ],
        findings: vec![
            finding("metabolomics-us/go-admin", "lint-x", "Fix lint"),
            finding("metabolomics-us/go-modules", "lint-x", "Fix lint again"),
        ],
    })
    .expect("repository evidence routes");

    assert_eq!(report.routed_findings.len(), 2);
    assert!(!report.routed_findings[0].duplicate);
    assert!(report.routed_findings[1].duplicate);
    assert_eq!(
        report.routed_findings[1].canonical_target,
        "metabolomics-us/go-modules"
    );
}

#[test]
fn permits_a_revival_requested_archived_repository_when_its_family_has_no_active_target() {
    let report = route_repositories(&ExplorationInput {
        repositories: vec![repository!(
            "metabolomics-us/go-admin",
            "go",
            true,
            true,
            "2026-07-14T00:00:00Z",
            "revival candidate",
            &[],
            &[],
            &[],
        )],
        findings: vec![finding("metabolomics-us/go-admin", "lint-x", "Fix lint")],
    })
    .expect("repository evidence routes");

    assert_eq!(
        report.canonical_targets[0].repository,
        "metabolomics-us/go-admin"
    );
    assert!(report.do_not_file_by_default.is_empty());
    assert_eq!(report.routed_findings.len(), 1);
    assert!(report.deferred_findings.is_empty());
}

#[test]
fn selects_the_repository_name_first_when_family_scores_tie() {
    let report = route_repositories(&ExplorationInput {
        repositories: vec![
            repository!(
                "metabolomics-us/go-modules",
                "go",
                false,
                false,
                "2026-07-14T00:00:00Z",
                "",
                &[],
                &[],
                &[],
            ),
            repository!(
                "metabolomics-us/go-admin",
                "go",
                false,
                false,
                "2026-07-14T00:00:00Z",
                "",
                &[],
                &[],
                &[],
            ),
        ],
        findings: Vec::new(),
    })
    .expect("repository evidence routes");

    assert_eq!(
        report.canonical_targets[0].repository,
        "metabolomics-us/go-admin"
    );
}

#[test]
fn counts_inbound_dependency_references_when_scoring_a_family() {
    let report = route_repositories(&ExplorationInput {
        repositories: vec![
            repository!(
                "metabolomics-us/go-modules",
                "go",
                false,
                false,
                "2026-07-14T00:00:00Z",
                "",
                &[],
                &[],
                &[],
            ),
            repository!(
                "metabolomics-us/go-admin",
                "go",
                false,
                false,
                "2026-07-14T00:00:00Z",
                "",
                &[],
                &[],
                &["metabolomics-us/go-modules"],
            ),
        ],
        findings: Vec::new(),
    })
    .expect("repository evidence routes");

    assert_eq!(
        report.canonical_targets[0].repository,
        "metabolomics-us/go-modules"
    );
    assert_eq!(report.canonical_targets[0].score, 20);
}

#[test]
fn defers_findings_when_a_family_has_no_eligible_target() {
    let report = route_repositories(&ExplorationInput {
        repositories: vec![repository!(
            "metabolomics-us/go-admin",
            "go",
            true,
            false,
            "2026-07-14T00:00:00Z",
            "",
            &[],
            &[],
            &[],
        )],
        findings: vec![finding("metabolomics-us/go-admin", "lint-x", "Fix lint")],
    })
    .expect("repository evidence routes");

    assert!(report.canonical_targets.is_empty());
    assert!(report.routed_findings.is_empty());
    assert_eq!(report.deferred_findings.len(), 1);
    assert_eq!(
        report.deferred_findings[0].reason,
        "no_eligible_canonical_target"
    );
}

#[test]
fn rejects_ambiguous_or_invalid_repository_evidence() {
    for document in [
        r#"{"repositories":[{"name":"metabolomics-us/go-modules","family":"go","archived":false,"revival_requested":false,"pushed_at":"2026-02-30T00:00:00Z","readme":"","module_paths":[],"packages":[],"dependency_references":[]}],"findings":[]}"#,
        r#"{"repositories":[{"name":"metabolomics-us/go-modules","family":"go","archived":false,"revival_requested":false,"pushed_at":"2026-07-14T00:00:00Z","readme":"","module_paths":[],"packages":[],"dependency_references":[],"extra":true}],"findings":[]}"#,
        r#"{"repositories":[{"name":"metabolomics-us/go-modules","family":"go","archived":false,"revival_requested":false,"pushed_at":"2026-07-14T00:00:00Z","readme":"","module_paths":[],"packages":[],"dependency_references":[]},{"name":"metabolomics-us/go-modules","family":"go","archived":false,"revival_requested":false,"pushed_at":"2026-07-14T00:00:00Z","readme":"","module_paths":[],"packages":[],"dependency_references":[]}],"findings":[]}"#,
        r#"{"repositories":[],"findings":[{"repository":"metabolomics-us/go-admin","fingerprint":"lint-x","title":"Fix lint"}]}"#,
    ] {
        assert!(ExplorationInput::from_json(document).is_err(), "{document}");
    }
}
