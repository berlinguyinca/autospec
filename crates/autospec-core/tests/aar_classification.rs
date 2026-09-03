//! AAR spec section 3: task classification.

use autospec_core::aar::classify::{
    classify, Capability, ClassificationInput, Complexity, Risk, TaskClass,
    DEFAULT_TIE_BREAKER_THRESHOLD,
};

#[test]
fn classifies_a_rust_bugfix_from_title_and_paths() {
    let input = ClassificationInput::new(
        "Fix panic when the queue parser sees an empty spec",
        "The parser panics on an empty document. Reproduce with the failing fixture.",
    )
    .with_paths(["crates/autospec-core/src/execution/queue_parser.rs"]);

    let classification = classify(&input);

    assert_eq!(classification.task_class, TaskClass::Bugfix);
    assert_eq!(classification.language, "rust");
    assert_eq!(classification.estimated_files, 1);
    assert!(classification.capabilities.contains(&Capability::Debugging));
    assert!(classification.confidence > DEFAULT_TIE_BREAKER_THRESHOLD);
}

#[test]
fn label_outweighs_incidental_keywords() {
    let input = ClassificationInput::new(
        "Add tests covering the broken retry path",
        "The retry path is broken; add coverage before fixing it.",
    )
    .with_labels(["type:test"]);

    let classification = classify(&input);

    assert_eq!(classification.task_class, TaskClass::Test);
    assert!(
        classification
            .evidence
            .iter()
            .any(|line| line.contains("label")),
        "evidence must record that a label decided the class: {:?}",
        classification.evidence
    );
}

#[test]
fn unmatched_text_defaults_with_low_confidence_and_asks_for_a_tie_breaker() {
    let input = ClassificationInput::new("Handle the thing", "Make it work.");

    let classification = classify(&input);

    assert_eq!(classification.task_class, TaskClass::Feature);
    assert!(classification.confidence <= DEFAULT_TIE_BREAKER_THRESHOLD);
    assert!(classification.needs_tie_breaker(DEFAULT_TIE_BREAKER_THRESHOLD));
}

#[test]
fn deep_design_verbs_raise_complexity_above_the_file_count() {
    let input = ClassificationInput::new(
        "Redesign the claim lease protocol",
        "We must reconcile two competing lease designs and decide on one.",
    )
    .with_paths(["crates/autospec-core/src/claim/lease.rs"]);

    let classification = classify(&input);

    assert!(classification.complexity >= Complexity::High);
    assert!(classification.requires_long_context);
}

#[test]
fn a_single_line_typo_is_trivial() {
    let input = ClassificationInput::new(
        "Fix typo in the install guide",
        "One line: copy the corrected sentence.",
    )
    .with_paths(["docs/install.md"]);

    let classification = classify(&input);

    assert_eq!(classification.complexity, Complexity::Trivial);
    assert_eq!(classification.risk, Risk::Low);
}

/// Risk follows the blast radius of the paths, not the verb in the title:
/// "small change to the credential handler" is still critical.
#[test]
fn security_paths_force_critical_risk_regardless_of_wording() {
    let input = ClassificationInput::new(
        "Small tidy-up in the credential helper",
        "Rename one local variable.",
    )
    .with_paths(["crates/autospec-cli/src/commands/security/credential.rs"]);

    let classification = classify(&input);

    assert_eq!(classification.risk, Risk::Critical);
}

#[test]
fn public_api_paths_raise_risk_to_high() {
    let input = ClassificationInput::new("Extend the core prelude", "Add one re-export.")
        .with_paths(["crates/autospec-core/src/lib.rs"]);

    let classification = classify(&input);

    assert!(classification.risk >= Risk::High);
}

#[test]
fn ui_work_requires_vision_capability() {
    let input = ClassificationInput::new(
        "Fix the dashboard component layout",
        "The css grid collapses at narrow widths.",
    )
    .with_paths(["src/components/RunPanel.tsx"]);

    let classification = classify(&input);

    assert_eq!(classification.task_class, TaskClass::Ui);
    assert!(classification.requires_vision);
    assert!(classification.capabilities.contains(&Capability::Vision));
}

#[test]
fn a_screenshot_in_the_body_requires_vision() {
    let input = ClassificationInput::new(
        "Compare the rendered output",
        "The attached screenshot shows the misaligned header.",
    )
    .with_paths(["src/report.rs"]);

    let classification = classify(&input);

    assert!(classification.requires_vision);
    assert!(classification.capabilities.contains(&Capability::Vision));
}

#[test]
fn many_files_require_long_context_and_context_handling() {
    let paths: Vec<String> = (0..12)
        .map(|index| format!("crates/autospec-core/src/module_{index}.rs"))
        .collect();
    let input = ClassificationInput::new("Implement the new report surface", "Wire it up.")
        .with_paths(paths);

    let classification = classify(&input);

    assert!(classification.requires_long_context);
    assert!(classification
        .capabilities
        .contains(&Capability::ContextHandling));
    assert!(classification.complexity >= Complexity::High);
}

#[test]
fn explicit_estimated_files_overrides_the_referenced_path_count() {
    let input = ClassificationInput::new("Implement the exporter", "See the plan.")
        .with_paths(["src/a.rs"])
        .with_estimated_files(9);

    let classification = classify(&input);

    assert_eq!(classification.estimated_files, 9);
    assert!(classification.complexity >= Complexity::High);
}

#[test]
fn evidence_is_recorded_for_every_classification() {
    let input = ClassificationInput::new("Fix the crash in the runner", "It panics.")
        .with_paths(["src/runner.rs"]);

    let classification = classify(&input);

    assert!(classification
        .evidence
        .iter()
        .any(|line| line.starts_with("task_class=")));
    assert!(classification
        .evidence
        .iter()
        .any(|line| line.starts_with("complexity=")));
    assert!(classification
        .evidence
        .iter()
        .any(|line| line.starts_with("risk=")));
}

#[test]
fn every_enum_round_trips_through_its_string_form() {
    for class in TaskClass::all() {
        assert_eq!(TaskClass::parse(class.as_str()), Some(class));
    }
    for complexity in [
        Complexity::Trivial,
        Complexity::Low,
        Complexity::Medium,
        Complexity::High,
        Complexity::Exceptional,
    ] {
        assert_eq!(Complexity::parse(complexity.as_str()), Some(complexity));
    }
    for risk in [Risk::Low, Risk::Medium, Risk::High, Risk::Critical] {
        assert_eq!(Risk::parse(risk.as_str()), Some(risk));
    }
    for capability in [
        Capability::Coding,
        Capability::Debugging,
        Capability::Planning,
        Capability::Review,
        Capability::RepositoryReasoning,
        Capability::ToolUse,
        Capability::TextualAnalysis,
        Capability::Documentation,
        Capability::Vision,
        Capability::ContextHandling,
        Capability::Concurrency,
    ] {
        assert_eq!(Capability::parse(capability.as_str()), Some(capability));
    }
}
