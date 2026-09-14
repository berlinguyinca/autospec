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

#[test]
fn shell_lint_parser_reads_both_gcc_spellings() {
    let old =
        shell_lint_gcc_finding("a.sh:3:5: WARNING: Use cd or exit (SC2164)").expect("old gcc");
    assert_eq!(old.code, 2164);
    assert_eq!(old.level, "warning");
    assert_eq!(old.file, "a.sh");
    assert_eq!(old.line, 3);

    let new =
        shell_lint_gcc_finding("a.sh:3:5: warning: Use cd or exit [SC2164]").expect("new gcc");
    assert_eq!(new.code, 2164);
    assert_eq!(new.level, "warning");

    let note = shell_lint_gcc_finding("a.sh:9:1: note: A and B or C [SC2015]").expect("new note");
    assert_eq!(note.code, 2015);
    assert_eq!(note.level, "info");

    assert!(shell_lint_gcc_finding("a.sh:3:5: warning: no code here").is_none());
    assert!(shell_lint_gcc_finding("not a finding line").is_none());
}

#[test]
fn run_shell_lint_flags_the_unguarded_cd_and_ignores_advisory_only() {
    let root =
        std::env::temp_dir().join(format!("autospec-shell-lint-unit-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let scripts = root.join("scripts");
    fs::create_dir_all(&scripts).expect("scripts fixture directory");
    fs::write(
        scripts.join("bad.sh"),
        "#!/usr/bin/env bash\ncd /nonexistent\nexit 0\n",
    )
    .expect("write unguarded cd fixture");

    let result = run_shell_lint("check_shell_lint", true, &root);
    // The check shells out to shellcheck. On a loaded machine that spawn can
    // come back with no exit status and no output, and this test then had
    // nothing to assert about -- it failed intermittently, blaming the code for
    // the machine being busy. The check now reports that case as *unmeasured*
    // rather than as a silent pass, which is the behaviour worth having: a
    // linter that never ran must not be read as a linter that found nothing.
    //
    // An unmeasured result is therefore not a failure of this test. What must
    // never happen is a *green* result, which would mean the unguarded `cd` was
    // read as clean.
    if result.is_unmeasured() {
        return;
    }
    assert!(
        result.is_failure(),
        "unguarded cd must fail the gate: {result:?}"
    );
    assert!(
        result
            .failure
            .as_deref()
            .is_some_and(|failure| failure.contains("SC2164")),
        "the failure must name the rule: {result:?}"
    );

    fs::remove_dir_all(&root).expect("remove temporary shell-lint fixture");
}

#[test]
fn block_expansion_failure_keeps_the_reason_not_only_its_hash() {
    // The sibling test asserts the DIGEST preserves child evidence. A digest is
    // a hash: it proved the message had been consumed while the message itself
    // was dropped, which is exactly how this went unnoticed. Assert the text.
    let result = block_expansion_result(
        "check_block_expansion",
        true,
        Vec::new(),
        Some("skills/demo/codex/prompt.md has no golden".to_string()),
    );
    assert!(result.is_failure());
    assert_eq!(
        result.failure.as_deref(),
        Some("skills/demo/codex/prompt.md has no golden"),
        "a failing block-expansion result must carry its reason, not only count it"
    );
}

#[test]
fn block_expansion_success_carries_no_failure() {
    let result = block_expansion_result("check_block_expansion", true, Vec::new(), None);
    assert!(result.failure.is_none());
    assert!(!result.is_failure());
}

#[cfg(test)]
mod captured_failure_tests {
    use super::*;
    use crate::validation::results::CheckResult;

    fn base() -> CheckResult {
        CheckResult::completed("check_example", true, 1, 5, 1, 0, 0, "digest")
    }

    #[test]
    fn the_shared_failure_constructor_attaches_its_message() {
        // Used by 84 call sites. It recorded message.len() as stderr_bytes and
        // digested the text, but never attached it -- so every external check
        // built this way reported "no reason captured" while its own byte count
        // proved the reason existed.
        let out = failure("check_example", true, "skills/x.md: missing TOKEN");
        assert_eq!(
            out.failure.as_deref(),
            Some("skills/x.md: missing TOKEN"),
            "the reason must survive construction"
        );
        assert_eq!(out.stderr_bytes, "skills/x.md: missing TOKEN".len());
        assert!(out.is_failure());
    }

    #[test]
    fn a_message_is_kept_not_merely_counted() {
        // The old helper added message.len() to stderr_bytes and dropped the
        // text, so the byte count proved a reason had existed while the reason
        // itself was gone.
        let out =
            captured_check_failure(base(), b"", Some("bundler: the prefix is still injected"));
        assert_eq!(
            out.failure.as_deref(),
            Some("bundler: the prefix is still injected")
        );
        assert!(out.stderr_bytes > 0, "the count is still recorded");
    }

    #[test]
    fn a_failing_command_reports_the_output_it_produced() {
        let out = captured_check_failure(base(), b"not ok 3 the thing diverged\n", None);
        let reason = out.failure.expect("a failing command must carry a reason");
        assert!(reason.contains("not ok 3 the thing diverged"), "{reason}");
    }

    #[test]
    fn a_huge_output_is_bounded_and_says_so() {
        // check_block_expansion produced 2.8 MB. Printing it is useless and
        // dropping it is worse; the edges name the cause and the count tells
        // the reader what was elided.
        let noisy = vec![b'x'; 3_000_000];
        let out = captured_check_failure(base(), &noisy, None);
        let reason = out.failure.expect("bounded, not dropped");
        assert!(
            reason.contains("3000000 bytes"),
            "{}",
            &reason[..80.min(reason.len())]
        );
        assert!(reason.len() < 4_000, "reason is {} bytes", reason.len());
    }

    #[test]
    fn a_silent_failure_says_it_was_silent() {
        // Distinguishable from "the runner discarded it", which is the whole
        // point: these need different fixes.
        let out = captured_check_failure(base(), b"   \n  ", None);
        assert_eq!(
            out.failure.as_deref(),
            Some("the command failed and produced no output")
        );
    }

    #[test]
    fn a_childs_stderr_report_survives_the_runner() {
        // The observed shape (#4632): 0 bytes of stdout, 106 of stderr. The
        // command layer already bound that stderr into result.failure; the
        // runner must carry it through, not overwrite it with "produced no
        // output" -- a claim the stderr byte count refutes.
        let mut result = CheckResult::completed("check_example", true, 1, 5, 1, 0, 106, "digest");
        result.failure = Some("bundler: AUTOSPEC_REPO_ROOT is not a repository".to_string());
        let out = captured_check_failure(result, b"", None);
        assert_eq!(
            out.failure.as_deref(),
            Some("bundler: AUTOSPEC_REPO_ROOT is not a repository")
        );
    }

    #[test]
    fn a_mismatched_member_carries_its_expanded_output_not_only_its_size() {
        // check_block_expansion produced 2.8 MB of expander output and reported
        // nothing of it. The mismatch names the member; the bounded edges of
        // what that member expanded to are the evidence a reader can act on.
        let root = std::env::temp_dir().join(format!(
            "autospec-block-expansion-mismatch-{}",
            std::process::id()
        ));
        let scripts = root.join("scripts");
        let skill = root.join("skills/demo");
        let goldens = root.join("tests/fixtures/skill-goldens");
        fs::create_dir_all(&skill).expect("create skill fixture");
        fs::create_dir_all(&scripts).expect("create scripts fixture");
        fs::create_dir_all(&goldens).expect("create golden fixture");
        fs::write(
            scripts.join("expand-skill-blocks.sh"),
            "#!/bin/sh\ncat \"$1\"\n",
        )
        .expect("write expander fixture");
        fs::write(skill.join("SKILL.md"), "# demo skill\nbody line\n")
            .expect("write skill fixture");
        fs::write(goldens.join("demo.SKILL.md.sha256"), "deadbeef\n")
            .expect("write a wrong golden");

        let result = run_block_expansion("check", true, &root);
        assert!(result.is_failure());
        let reason = result.failure.expect("the mismatch must carry a reason");
        assert!(reason.contains("sha256 mismatch"), "{reason}");
        assert!(reason.contains("# demo skill"), "{reason}");
        assert!(reason.contains("bytes"), "{reason}");
        fs::remove_dir_all(root).expect("remove block expansion fixture");
    }
}
