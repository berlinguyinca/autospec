use std::path::PathBuf;

use autospec_core::validation::{StructuralCheck, StructuralValidator};

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
fn trio_lockstep_and_required_files_have_separate_owners() {
    StructuralValidator::validate_trio_lockstep(&fixture("missing-trio-file"))
        .expect("a missing installer does not make matching trio bodies drift");

    let failure = StructuralValidator::validate_required_trio_files(&fixture("missing-trio-file"))
        .expect_err("the required-files owner reports the missing installer");

    assert_eq!(failure, "skills/trio: missing required file uninstall.sh");
}

#[test]
fn trio_and_duo_lockstep_owners_validate_their_respective_harnesses() {
    StructuralValidator::validate_trio_lockstep(&fixture("valid-skill"))
        .expect("matching trio passes its owner");
    StructuralValidator::validate_duo_lockstep(&fixture("valid-skill"))
        .expect("matching duo passes its owner");
}

#[test]
fn structural_dispatch_runs_exactly_the_requested_owner() {
    StructuralValidator::run(StructuralCheck::TrioLockstep, &fixture("valid-skill"))
        .expect("trio owner passes through the dispatcher");

    let failure = StructuralValidator::run(StructuralCheck::CatalogSlot, &fixture("valid-skill"))
        .expect_err("unported catalog slots are not silently treated as valid");

    assert_eq!(failure, "validation structural owner is not implemented");
}

#[test]
fn startup_preflight_contract_accepts_markers_and_canonical_blocks() {
    StructuralValidator::validate_startup_preflight(&fixture("startup-preflight"))
        .expect("startup preflight fixture passes");
}

#[test]
fn startup_preflight_contract_reports_a_divergent_trio_block() {
    let failure =
        StructuralValidator::validate_startup_preflight(&fixture("startup-preflight-divergent"))
            .expect_err("divergent startup block fails");

    assert_eq!(
        failure,
        "skills/autospec/codex/prompt.md preflight body diverges from canonical"
    );
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

#[test]
fn codex_skills_install_contract_accepts_registry_and_legacy_destinations() {
    StructuralValidator::validate_codex_skills_install(&fixture("codex-skills-install"))
        .expect("installer fixture passes");
}

#[test]
fn codex_skills_install_contract_reports_a_missing_registry_destination() {
    let failure = StructuralValidator::validate_codex_skills_install(&fixture(
        "codex-skills-install-missing-registry",
    ))
    .expect_err("missing Codex registry destination fails");

    assert_eq!(
        failure,
        "skills/autospec-design/install.sh missing Codex skills-dir install (skills/$SKILL_NAME/SKILL.md)"
    );
}

#[test]
fn shared_script_install_contract_requires_only_shared_runtime_helpers() {
    StructuralValidator::validate_shared_script_install(&fixture("shared-script-install"))
        .expect("shared helper fixture passes");
}

#[test]
fn shared_script_install_contract_reports_the_autospec_design_anchor() {
    let failure = StructuralValidator::validate_shared_script_install(&fixture(
        "shared-script-install-autospec-design-missing-runtime",
    ))
    .expect_err("autospec-design must install a referenced shared helper");

    assert_eq!(
        failure,
        "check_shared_script_install: autospec-design references a shared helper but does not install into ~/.autospec/scripts"
    );
}

#[test]
fn flag_sentinel_docs_contract_requires_every_runtime_flag_to_be_documented() {
    StructuralValidator::validate_flag_sentinel_docs(&fixture("flag-sentinel-docs"))
        .expect("every runtime sentinel flag is documented");

    let failure =
        StructuralValidator::validate_flag_sentinel_docs(&fixture("flag-sentinel-docs-missing"))
            .expect_err("an undocumented runtime sentinel flag fails");

    assert_eq!(
        failure,
        "docs/FLAGS.md: missing sentinel flag(s): second.flag"
    );
}
