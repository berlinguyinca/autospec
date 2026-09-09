//! Firing and silent fixtures for the AS-DAG-001 through AS-DAG-010 catalogue
//! (docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md §22).

use autospec_core::lint::dag::{
    artificial_serialization, cycle, excessive_critical_path, excessive_fan_in, low_initial_width,
    metadata_mismatch, order_only_dependency, shared_write_hotspot, unjustified_dependency,
    DagLintRule, DagLintSeverity, DEFAULT_FAN_IN_THRESHOLD, DEFAULT_SHARED_WRITE_THRESHOLD,
};

fn finding_id(finding: Option<autospec_core::lint::dag::DagLintFinding>) -> Option<&'static str> {
    finding.map(|f| f.rule.id())
}

// ── Stable ids and severity ──────────────────────────────────────────────────

#[test]
fn rule_ids_are_stable_as_dag_001_through_010() {
    let ids = [
        (DagLintRule::UnjustifiedDependency, "AS-DAG-001"),
        (DagLintRule::OrderOnlyDependency, "AS-DAG-002"),
        (DagLintRule::ArtificialSerialization, "AS-DAG-003"),
        (DagLintRule::ExcessiveFanIn, "AS-DAG-004"),
        (DagLintRule::ExcessiveCriticalPath, "AS-DAG-005"),
        (DagLintRule::LowInitialWidth, "AS-DAG-006"),
        (DagLintRule::SharedWriteHotspot, "AS-DAG-007"),
        (DagLintRule::SplitCreatedOrdering, "AS-DAG-008"),
        (DagLintRule::MetadataDependencyMismatch, "AS-DAG-009"),
        (DagLintRule::Cycle, "AS-DAG-010"),
    ];
    for (rule, expected) in ids {
        assert_eq!(rule.id(), expected, "stable rule id for {rule:?}");
    }
}

#[test]
fn cycle_is_the_only_fatal_rule() {
    for rule in [
        DagLintRule::UnjustifiedDependency,
        DagLintRule::OrderOnlyDependency,
        DagLintRule::ArtificialSerialization,
        DagLintRule::ExcessiveFanIn,
        DagLintRule::ExcessiveCriticalPath,
        DagLintRule::LowInitialWidth,
        DagLintRule::SharedWriteHotspot,
        DagLintRule::SplitCreatedOrdering,
    ] {
        assert_ne!(
            rule.severity(),
            DagLintSeverity::Fatal,
            "{rule:?} must not be fatal"
        );
    }
    assert_eq!(DagLintRule::Cycle.severity(), DagLintSeverity::Fatal);
}

// ── AS-DAG-001 UNJUSTIFIED_DEPENDENCY ────────────────────────────────────────

#[test]
fn unjustified_dependency_fires_without_reason_code_or_artifact() {
    let finding = unjustified_dependency(None, None);
    assert_eq!(finding_id(finding), Some("AS-DAG-001"));
}

#[test]
fn unjustified_dependency_fires_on_unrecognized_reason_code() {
    let finding = unjustified_dependency(Some("do it first"), None);
    assert_eq!(finding_id(finding), Some("AS-DAG-001"));
}

#[test]
fn unjustified_dependency_fires_on_empty_artifact() {
    let finding = unjustified_dependency(None, Some("   "));
    assert_eq!(finding_id(finding), Some("AS-DAG-001"));
}

#[test]
fn unjustified_dependency_silent_with_recognized_reason_code() {
    assert!(unjustified_dependency(Some("consumes-generated-artifact"), None).is_none());
    assert!(unjustified_dependency(Some("external-prerequisite"), None).is_none());
}

#[test]
fn unjustified_dependency_silent_with_artifact() {
    assert!(unjustified_dependency(None, Some("RouterBackend")).is_none());
}

// ── AS-DAG-002 ORDER_ONLY_DEPENDENCY ─────────────────────────────────────────

#[test]
fn order_only_dependency_flags_implement_first() {
    let finding = order_only_dependency("implement first, then wire it up");
    assert_eq!(finding_id(finding), Some("AS-DAG-002"));
}

#[test]
fn order_only_dependency_flags_foundation() {
    let finding = order_only_dependency("this issue is the foundation for the rest");
    assert_eq!(finding_id(finding), Some("AS-DAG-002"));
}

#[test]
fn order_only_dependency_flags_do_before_ui_case_insensitively() {
    let finding = order_only_dependency("Do before UI so the screens have data");
    assert_eq!(finding_id(finding), Some("AS-DAG-002"));
}

#[test]
fn order_only_dependency_flags_easier_if() {
    let finding = order_only_dependency("easier if this lands before the dashboard");
    assert_eq!(finding_id(finding), Some("AS-DAG-002"));
}

#[test]
fn order_only_dependency_silent_on_technical_prerequisite() {
    let rationale = "consumes the RouterBackend trait introduced on issue #12";
    assert!(order_only_dependency(rationale).is_none());
}

#[test]
fn order_only_dependency_requires_word_start_boundary() {
    // "prefoundation" embeds the phrase mid-word; it is not ordering wording.
    assert!(order_only_dependency("the prefoundation layout is unrelated").is_none());
}

// ── AS-DAG-003 ARTIFICIAL_SERIALIZATION ──────────────────────────────────────

#[test]
fn artificial_serialization_fires_when_child_never_references_artifact() {
    let child_text = "Render the dashboard from existing APIs.";
    let finding = artificial_serialization(child_text, "RouterBackend");
    assert_eq!(finding_id(finding), Some("AS-DAG-003"));
}

#[test]
fn artificial_serialization_silent_when_child_references_artifact() {
    let child_text = "Wire the dashboard to `RouterBackend` for routing.";
    assert!(artificial_serialization(child_text, "RouterBackend").is_none());
}

#[test]
fn artificial_serialization_silent_when_artifact_is_blank() {
    // With no named artifact there is nothing the child could reference.
    assert!(artificial_serialization("Render the dashboard.", "  ").is_none());
}

// ── AS-DAG-004 EXCESSIVE_FAN_IN ──────────────────────────────────────────────

#[test]
fn excessive_fan_in_fires_at_six_with_default_threshold() {
    assert_eq!(
        finding_id(excessive_fan_in(6, DEFAULT_FAN_IN_THRESHOLD)),
        Some("AS-DAG-004")
    );
    assert_eq!(DEFAULT_FAN_IN_THRESHOLD, 5);
}

#[test]
fn excessive_fan_in_silent_at_five_with_default_threshold() {
    assert!(excessive_fan_in(5, DEFAULT_FAN_IN_THRESHOLD).is_none());
    assert!(excessive_fan_in(0, DEFAULT_FAN_IN_THRESHOLD).is_none());
}

#[test]
fn excessive_fan_in_respects_custom_threshold() {
    assert_eq!(finding_id(excessive_fan_in(4, 3)), Some("AS-DAG-004"));
    assert!(excessive_fan_in(3, 3).is_none());
}

// ── AS-DAG-005 EXCESSIVE_CRITICAL_PATH ───────────────────────────────────────

#[test]
fn excessive_critical_path_fires_over_limit() {
    // 20 issues: limit = max(5, ceil(20 * 0.30)) = 6; path 7 exceeds it.
    assert_eq!(
        finding_id(excessive_critical_path(20, 7)),
        Some("AS-DAG-005")
    );
}

#[test]
fn excessive_critical_path_silent_at_limit() {
    assert!(excessive_critical_path(20, 6).is_none());
}

#[test]
fn excessive_critical_path_silent_below_issue_count_floor() {
    // Rule only applies for issue count >= 10.
    assert!(excessive_critical_path(9, 40).is_none());
}

// ── AS-DAG-006 LOW_INITIAL_WIDTH ─────────────────────────────────────────────

#[test]
fn low_initial_width_fires_for_20_issues_capacity_32_width_4() {
    // floor = min(32, max(4, ceil(20 * 0.25))) = 5; width 4 is below it.
    assert_eq!(finding_id(low_initial_width(20, 32, 4)), Some("AS-DAG-006"));
}

#[test]
fn low_initial_width_silent_when_width_meets_floor() {
    assert!(low_initial_width(20, 32, 5).is_none());
    assert!(low_initial_width(20, 32, 17).is_none());
}

#[test]
fn low_initial_width_silent_below_issue_count_floor() {
    assert!(low_initial_width(9, 32, 0).is_none());
}

#[test]
fn low_initial_width_silent_below_capacity_floor() {
    // Rule only applies when capacity >= 10.
    assert!(low_initial_width(20, 8, 0).is_none());
}

// ── AS-DAG-007 SHARED_WRITE_HOTSPOT ──────────────────────────────────────────

#[test]
fn shared_write_hotspot_fires_at_three() {
    assert_eq!(
        finding_id(shared_write_hotspot(DEFAULT_SHARED_WRITE_THRESHOLD)),
        Some("AS-DAG-007")
    );
    assert_eq!(DEFAULT_SHARED_WRITE_THRESHOLD, 3);
}

#[test]
fn shared_write_hotspot_silent_at_two() {
    assert!(shared_write_hotspot(2).is_none());
}

// ── AS-DAG-008 SPLIT_CREATED_ORDERING ────────────────────────────────────────
// No pure predicate: detection needs sibling-size comparison the caller owns.
// The catalogue entry is covered by the id/severity tests above.

// ── AS-DAG-009 METADATA_DEPENDENCY_MISMATCH ──────────────────────────────────

#[test]
fn metadata_mismatch_fires_on_differing_sets() {
    assert_eq!(
        finding_id(metadata_mismatch(&[1, 2], &[1, 3])),
        Some("AS-DAG-009")
    );
}

#[test]
fn metadata_mismatch_silent_when_sets_match_ignoring_order_and_duplicates() {
    assert!(metadata_mismatch(&[3, 1, 2, 1], &[2, 3, 1]).is_none());
    assert!(metadata_mismatch(&[], &[]).is_none());
}

// ── AS-DAG-010 CYCLE ─────────────────────────────────────────────────────────

#[test]
fn cycle_returns_fatal_finding() {
    let finding = cycle(&["#1".to_string(), "#2".to_string(), "#1".to_string()]);
    assert_eq!(finding.rule.id(), "AS-DAG-010");
    assert_eq!(finding.severity(), DagLintSeverity::Fatal);
}

#[test]
fn cycle_finding_carries_the_cycle_path() {
    let finding = cycle(&["#1".to_string(), "#2".to_string(), "#1".to_string()]);
    assert_eq!(finding.subject, "#1 -> #2 -> #1");
    assert!(!finding.message.is_empty());
}
