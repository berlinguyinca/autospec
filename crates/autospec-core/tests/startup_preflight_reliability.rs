use autospec_core::validation::StructuralValidator;

#[test]
fn startup_preflight_contract_requires_persistent_failure_evidence() {
    let root = std::env::temp_dir().join(format!(
        "autospec-startup-failure-contract-{}",
        std::process::id()
    ));
    let template_dir = root.join("templates/skill-blocks");
    let skill_dir = root.join("skills/autospec-design");
    std::fs::create_dir_all(skill_dir.join("codex")).expect("codex fixture dir");
    std::fs::create_dir_all(skill_dir.join("opencode")).expect("opencode fixture dir");
    std::fs::create_dir_all(&template_dir).expect("template fixture dir");
    std::fs::write(
        template_dir.join("startup-self-update.md"),
        "## Startup self-update\n\n```bash\ncurl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh\nbootstrap --skill all --harness all --update\n```\n",
    )
    .expect("legacy template");
    for path in [
        skill_dir.join("SKILL.md"),
        skill_dir.join("codex/prompt.md"),
        skill_dir.join("opencode/agent.md"),
    ] {
        std::fs::write(
            path,
            "<!-- autospec-block:startup-self-update SKILL_NAME=autospec-design -->\n",
        )
        .expect("skill marker");
    }

    let failure = StructuralValidator::validate_startup_preflight(&root)
        .expect_err("legacy silent-failure preflight must fail validation");
    assert!(failure.contains("last-update-failure.json"));
    let _ = std::fs::remove_dir_all(root);
}
