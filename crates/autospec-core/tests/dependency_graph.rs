use autospec_core::graph::{execution_order, GraphErrorKind};
use autospec_core::spec::{SpecId, SpecMetadata, SpecStatus, SpecVersion};

fn spec(id: &str, version: &str, dependencies: &[&str]) -> SpecMetadata {
    SpecMetadata {
        id: SpecId::new(id).expect("valid id"),
        title: id.to_string(),
        version: SpecVersion::new(version).expect("valid version"),
        status: SpecStatus::Ready,
        objective: "test graph ordering".to_string(),
        dependencies: dependencies
            .iter()
            .map(|dependency| dependency.to_string())
            .collect(),
        acceptance_criteria: Vec::new(),
        validation_command: "true".to_string(),
    }
}

#[test]
fn dependency_graph_orders_linear_specs() {
    let specs = vec![
        spec(
            "v64-dependency-graph-ordering",
            "V64",
            &["v63-spec-metadata-parser"],
        ),
        spec("v62-rust-core-workspace", "V62", &[]),
        spec(
            "v63-spec-metadata-parser",
            "V63",
            &["v62-rust-core-workspace"],
        ),
    ];

    let order = execution_order(&specs).expect("linear graph is acyclic");

    assert_eq!(
        order,
        vec![
            "v62-rust-core-workspace",
            "v63-spec-metadata-parser",
            "v64-dependency-graph-ordering"
        ]
    );
}

#[test]
fn dependency_graph_orders_diamond_specs_stably() {
    let specs = vec![
        spec(
            "v65-spec-state-validation",
            "V65",
            &["v63-spec-metadata-parser"],
        ),
        spec(
            "v64-dependency-graph-ordering",
            "V64",
            &["v63-spec-metadata-parser"],
        ),
        spec(
            "v66-autonomous-execution-queue",
            "V66",
            &["v64-dependency-graph-ordering", "v65-spec-state-validation"],
        ),
        spec("v63-spec-metadata-parser", "V63", &[]),
    ];

    let order = execution_order(&specs).expect("diamond graph is acyclic");

    assert_eq!(
        order,
        vec![
            "v63-spec-metadata-parser",
            "v64-dependency-graph-ordering",
            "v65-spec-state-validation",
            "v66-autonomous-execution-queue"
        ]
    );
}

#[test]
fn dependency_graph_reports_missing_dependency() {
    let specs = vec![spec(
        "v64-dependency-graph-ordering",
        "V64",
        &["v63-spec-metadata-parser"],
    )];

    let error = execution_order(&specs).expect_err("missing dependency should fail");

    assert_eq!(error.kind, GraphErrorKind::MissingDependency);
    assert_eq!(
        error.spec_id.as_deref(),
        Some("v64-dependency-graph-ordering")
    );
    assert_eq!(
        error.dependency.as_deref(),
        Some("v63-spec-metadata-parser")
    );
}

#[test]
fn dependency_graph_reports_cycle_path() {
    let specs = vec![
        spec(
            "v62-rust-core-workspace",
            "V62",
            &["v64-dependency-graph-ordering"],
        ),
        spec(
            "v63-spec-metadata-parser",
            "V63",
            &["v62-rust-core-workspace"],
        ),
        spec(
            "v64-dependency-graph-ordering",
            "V64",
            &["v63-spec-metadata-parser"],
        ),
    ];

    let error = execution_order(&specs).expect_err("cycle should fail");

    assert_eq!(error.kind, GraphErrorKind::Cycle);
    assert!(error.cycle.contains(&"v62-rust-core-workspace".to_string()));
    assert!(error
        .cycle
        .contains(&"v64-dependency-graph-ordering".to_string()));
}
