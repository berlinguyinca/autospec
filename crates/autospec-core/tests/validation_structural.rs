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
