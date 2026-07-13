use std::path::PathBuf;

use autospec_core::validation::StructuralValidator;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../autospec-cli/tests/fixtures/validation-cutover")
        .join(name)
}

#[test]
fn matching_trio_and_duo_skill_bodies_pass_without_external_processes() {
    StructuralValidator::validate_root(&fixture("valid-skill")).expect("valid fixtures pass");
}

#[test]
fn drifted_duo_reports_the_divergent_file() {
    let failure = StructuralValidator::validate_root(&fixture("drifted-skill"))
        .expect_err("drifted duo fails");

    assert!(failure.contains("skills/duo/codex/prompt.md"));
}

#[test]
fn missing_trio_required_file_reports_the_skill_and_file() {
    let failure = StructuralValidator::validate_root(&fixture("missing-trio-file"))
        .expect_err("missing trio file fails");

    assert_eq!(failure, "skills/trio: missing required file uninstall.sh");
}

#[test]
fn named_skill_policy_sections_pass_when_every_member_has_the_required_heading() {
    StructuralValidator::validate_policy_sections(&fixture("policy-sections"))
        .expect("policy section fixture passes");
}

#[test]
fn named_skill_policy_sections_report_the_missing_member_heading() {
    let failure = StructuralValidator::validate_policy_sections(&fixture("missing-policy-section"))
        .expect_err("missing policy section fails");

    assert_eq!(
        failure,
        "skills/autospec-run/codex/prompt.md: missing '## Stop mode' section"
    );
}

#[test]
fn self_update_sections_accept_trio_and_duo_forms() {
    StructuralValidator::validate_self_update_sections(&fixture("self-update"))
        .expect("trio and duo self-update fixtures pass");
    StructuralValidator::validate_trio_self_update_sections(&fixture("self-update"))
        .expect("trio self-update fixture passes independently");
    StructuralValidator::validate_duo_self_update_sections(&fixture("self-update"))
        .expect("duo self-update fixture passes independently");
}

#[test]
fn self_update_sections_report_a_missing_update_flag() {
    let failure =
        StructuralValidator::validate_self_update_sections(&fixture("self-update-missing-flag"))
            .expect_err("trio install flag is required");

    assert_eq!(
        failure,
        "autospec: install.sh missing --update flag handling"
    );
}

#[test]
fn keyword_routing_contract_passes_when_trio_and_classifier_literals_match() {
    StructuralValidator::validate_keyword_routing_section(&fixture("keyword-routing"))
        .expect("keyword-routing fixture passes");
}

#[test]
fn keyword_routing_contract_reports_the_missing_trio_literal() {
    let failure = StructuralValidator::validate_keyword_routing_section(&fixture(
        "keyword-routing-missing-literal",
    ))
    .expect_err("missing trio literal fails");

    assert_eq!(
        failure,
        "autospec-listen: codex/prompt.md missing completed Plan-mode handoff route"
    );
}
