//! Unit tests for the external validation checks.
//!
//! Extracted from external.rs, which is 6.5k lines and over the file-size
//! ratchet's 600-line limit: an oversized file may shrink or hold but never
//! grow, so the tests had to move before a new assertion could be added.

use super::*;
#[test]
fn block_expansion_rejects_markered_member_without_golden() {
    let root = std::env::temp_dir().join(format!(
        "autospec-block-expansion-missing-golden-{}",
        std::process::id()
    ));
    let scripts = root.join("scripts");
    let skill = root.join("skills/demo");
    let goldens = root.join("tests/fixtures/skill-goldens");
    fs::create_dir_all(skill.join("codex")).expect("create skill fixture");
    fs::create_dir_all(&scripts).expect("create scripts fixture");
    fs::create_dir_all(&goldens).expect("create golden fixture");
    fs::write(
        scripts.join("expand-skill-blocks.sh"),
        "#!/bin/sh\ncat \"$1\"\n",
    )
    .expect("write expander fixture");
    fs::write(skill.join("SKILL.md"), "# demo\n").expect("write skill fixture");
    fs::write(
        skill.join("codex/prompt.md"),
        "<!-- autospec-block:startup-self-update SKILL_NAME=demo -->\n",
    )
    .expect("write markered member fixture");
    fs::write(
        goldens.join("demo.SKILL.md.sha256"),
        "bc70e26f40b8816eb177813dda1f5f529a27a4641d45aa19cae2348a8c6a5fe9\n",
    )
    .expect("write required skill golden");

    let result = run_block_expansion("check", true, &root);
    let expected = "check_block_expansion: markered member skills/demo/codex/prompt.md has no golden (tests/fixtures/skill-goldens/demo.codex.prompt.md.sha256 missing — fail closed)";

    assert!(result.is_failure());
    assert_eq!(
        result.spawn_count, 2,
        "only SKILL expansion and hashing run"
    );
    assert_eq!(result.stderr_bytes, expected.len());
    assert_ne!(
        result.output_digest,
        output_digest(&[], expected.as_bytes())
    );
    fs::remove_dir_all(root).expect("remove block expansion fixture");
}

#[test]
fn block_expansion_failure_digest_preserves_child_evidence() {
    let child = CheckResult::completed("child", true, 0, 0, 1, 5, 0, "child-output-digest");
    let mut evidence = b"child-output-digest\n".to_vec();
    evidence.extend_from_slice(b"missing golden");

    let result = block_expansion_result(
        "check_block_expansion",
        true,
        vec![child],
        Some("missing golden".to_string()),
    );

    assert_eq!(result.output_digest, output_digest(&evidence, &[]));
}

#[test]
fn aggregate_preserves_a_missing_child_tool_as_unmeasured() {
    let result = aggregate(
        "check",
        true,
        vec![CheckResult::unmeasured(
            "child",
            true,
            "bats is not on PATH, so nothing was measured",
        )],
    );

    assert_eq!(result.exit_code, None);
    assert!(result.is_unmeasured(), "{result:?}");
    assert!(
        !result.is_success(),
        "an aggregate over an absent tool must not read as a pass"
    );
}

#[test]
fn aggregate_over_no_sub_checks_is_unmeasured_rather_than_a_pass() {
    let result = aggregate("check", true, Vec::new());

    assert!(result.is_unmeasured(), "{result:?}");
    assert!(!result.is_success());
}

#[test]
fn aggregate_reports_a_measured_failure_ahead_of_an_unmeasured_sibling() {
    let result = aggregate(
        "check",
        true,
        vec![
            CheckResult::completed("broken", true, 1, 0, 1, 0, 0, "broken"),
            CheckResult::unmeasured("absent", true, "bats is not on PATH"),
        ],
    );

    assert_eq!(result.exit_code, Some(1));
    assert!(result.is_failure());
    assert!(!result.is_unmeasured());
}

#[test]
fn stale_researcher_count_matches_the_legacy_word_boundary_contract() {
    let path = std::env::temp_dir().join(format!(
        "autospec-stale-researcher-count-{}",
        std::process::id()
    ));

    fs::write(&path, "6 researchers").expect("temporary fixture writes");
    assert!(contains_stale_researcher_count(&path));
    fs::write(&path, "16 researchers").expect("temporary fixture rewrites");
    assert!(!contains_stale_researcher_count(&path));
    fs::write(&path, "each of the 6 sources").expect("temporary fixture rewrites");
    assert!(contains_stale_researcher_count(&path));

    fs::remove_file(path).expect("temporary fixture removes");
}

#[test]
fn retired_safety_writer_guard_rejects_the_file_and_all_live_writeback_surfaces() {
    let root = std::env::temp_dir().join(format!(
        "autospec-retired-safety-writer-{}",
        std::process::id()
    ));
    let scripts = root.join("scripts");
    fs::create_dir_all(&scripts).expect("create temporary scripts directory");
    let retired = scripts.join("apply-safety-review.sh");
    fs::write(&retired, "#!/usr/bin/env bash\n").expect("write retired script fixture");

    assert!(retired_safety_writer_guard("check", true, &root).is_failure());

    fs::remove_file(&retired).expect("remove retired script fixture");
    fs::write(
        scripts.join("autonomous-promote-open-issues.sh"),
        "bash apply-safety-review.sh\n",
    )
    .expect("write live caller fixture");

    assert!(retired_safety_writer_guard("check", true, &root).is_failure());

    fs::remove_file(scripts.join("autonomous-promote-open-issues.sh"))
        .expect("remove live caller fixture");
    let prompt = root.join("skills/autospec-classify/SKILL.md");
    fs::create_dir_all(prompt.parent().expect("skill fixture has parent"))
        .expect("create temporary skill directory");
    fs::write(&prompt, "gh issue edit <N> --add-label safety:reviewed\n")
        .expect("write direct safety writer fixture");

    assert!(retired_safety_writer_guard("check", true, &root).is_failure());

    fs::remove_file(&prompt).expect("remove direct safety writer fixture");
    let explorer = scripts.join("autospec-explore.sh");
    fs::write(&explorer, "autospec-safety-decision:begin\n")
        .expect("write explorer safety writer fixture");

    assert!(retired_safety_writer_guard("check", true, &root).is_failure());

    fs::remove_file(&explorer).expect("remove explorer safety writer fixture");
    let run_prompt = root.join("skills/autospec-run/codex/prompt.md");
    fs::create_dir_all(run_prompt.parent().expect("run prompt fixture has parent"))
        .expect("create run prompt fixture directory");
    fs::write(&run_prompt, "autospec:needs-human\n")
        .expect("write run prompt safety writer fixture");

    assert!(retired_safety_writer_guard("check", true, &root).is_failure());

    fs::remove_dir_all(root).expect("remove temporary guard fixture");
}
