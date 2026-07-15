use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_repo(path: &str) -> String {
    std::fs::read_to_string(workspace_root().join(path)).expect("repo file is readable")
}

fn body_after_frontmatter(document: &str) -> String {
    let mut fence_count = 0;
    let mut body = String::new();

    for line in document.lines() {
        if line == "---" {
            fence_count += 1;
            continue;
        }

        if fence_count >= 2 {
            body.push_str(line);
            body.push('\n');
        }
    }

    body
}

#[test]
fn autospec_explore_documents_metabolomics_lab_ops_specialists_in_lockstep() {
    let skill = read_repo("skills/autospec-explore/SKILL.md");
    let codex = read_repo("skills/autospec-explore/codex/prompt.md");
    let opencode = read_repo("skills/autospec-explore/opencode/agent.md");
    let skill_body = body_after_frontmatter(&skill);
    let opencode_body = body_after_frontmatter(&opencode);

    assert_eq!(skill_body, codex, "codex prompt mirrors SKILL.md body");
    assert_eq!(
        skill_body, opencode_body,
        "opencode body mirrors SKILL.md body"
    );

    for required in [
        "repo names",
        "dependency manifests",
        "docs",
        "code paths",
        "ms-data-curator",
        "chemical-identity-reviewer",
        "lc-binbase-workflow-analyst",
        "mona-sirius-integration-reviewer",
        "hpc-lab-ops-reliability",
        "`evidence`",
        "`severity`",
        "`consumer`",
        "`gap_check`",
        "gap-confirm",
        "verify",
        "ROI",
        "pattern-synthesis",
        "severity-first rank",
    ] {
        assert!(
            skill.contains(required),
            "autospec-explore must document {required}"
        );
    }
}
