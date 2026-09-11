//! Regression tests for the sibling-guard extraction invariants (issue
//! #4289).
//!
//! The incident, reconstructed here: one system, four implementations of
//! the same rule, each guard re-earned in a separate debugging session and
//! none extracted until the fourth copy. The tests instantiate the
//! configuration the incident had (four-copy ledger, the convpass/iwconv
//! sibling pair, the 16-runner fleet, the one-file fix) and the controls
//! that prove the heuristics do not false-positive on non-siblings and on
//! a guard that was extracted on schedule.

use autospec_core::guard_extraction::{
    audit, distinctive_shared, distinctive_tokens, extraction_due, fix_scope, guard_citations,
    guard_coverage, longest_common_substring_len, name_candidates, sibling_pair, CopyEvent,
    CopyLedger, CostModel, SECOND_COPY,
};

fn model() -> CostModel {
    // The incident's units: one extraction ~ 20 minutes (constant at any
    // copy count); one copy ~ one debugging session.
    CostModel::new(20, 1)
}

fn incident_ledger() -> CopyLedger {
    let mut ledger = CopyLedger::new();
    assert_eq!(ledger.add_copy("convpass"), CopyEvent::Original);
    assert_eq!(ledger.add_copy("iwconv"), CopyEvent::ExtractionDue);
    assert_eq!(
        ledger.add_copy("autospec-issue"),
        CopyEvent::Overdue { copies: 3 }
    );
    assert_eq!(
        ledger.add_copy("iw-issue"),
        CopyEvent::Overdue { copies: 4 }
    );
    ledger
}

#[test]
fn extraction_is_due_at_the_second_copy_not_the_fourth() {
    assert!(!extraction_due(0));
    assert!(!extraction_due(1));
    assert!(extraction_due(2));
    assert!(extraction_due(3));
    assert!(extraction_due(4));
    assert_eq!(SECOND_COPY, 2);

    let ledger = incident_ledger();
    assert_eq!(ledger.len(), 4);
    assert!(ledger.due());
    assert_eq!(
        ledger.components(),
        &["convpass", "iwconv", "autospec-issue", "iw-issue"]
    );
}

#[test]
fn deferring_extraction_multiplies_the_copy_cost_never_the_extraction_cost() {
    let m = model();
    // Extracting at the second copy: the one-time extraction plus one copy
    // cost per copy (the first is the original implementation, not a
    // rediscovery, but the model is linear: 20 + 2×1 = 22).
    assert_eq!(m.total_cost(2), 22);
    // The incident: extracting at the fourth copy. The extraction cost is
    // unchanged; two more copy costs were paid.
    assert_eq!(m.total_cost(4), 24);

    // The delta of deferring from copy 2 to copy 4 is exactly two copy
    // costs: the extraction cost is constant and cancels out.
    assert_eq!(m.total_cost(4) - m.total_cost(2), 2 * m.copy_cost);
    assert_eq!(m.deferral_cost(4), 2 * m.copy_cost);
    assert_eq!(m.deferral_cost(3), m.copy_cost);
    assert_eq!(m.deferral_cost(2), 0);
    assert_eq!(m.deferral_cost(1), 0);
    assert_eq!(m.deferral_cost(0), 0);

    // The constant claim holds at any copy count: the deferral cost never
    // depends on the extraction cost.
    for copies in 0..=10usize {
        let cheap = CostModel::new(5, 3);
        let dear = CostModel::new(400, 3);
        assert_eq!(cheap.deferral_cost(copies), dear.deferral_cost(copies));
        if copies >= 2 {
            assert_eq!(
                dear.total_cost(copies) - dear.total_cost(2),
                dear.deferral_cost(copies)
            );
        }
    }
}

#[test]
fn ledger_line_names_the_deferral_from_the_second_copy_on() {
    let m = model();
    let ok = CopyLedger::new();
    assert!(ok.is_empty());
    assert!(ok.line(&m).starts_with("OK:"));

    let mut one = CopyLedger::new();
    assert_eq!(one.add_copy("convpass"), CopyEvent::Original);
    assert!(!one.due());
    assert!(one.line(&m).starts_with("OK:"));

    let mut two = CopyLedger::new();
    two.add_copy("convpass");
    assert_eq!(two.add_copy("iwconv"), CopyEvent::ExtractionDue);
    assert!(two.due());
    assert_eq!(two.deferral_cost(&m), 0);
    let line = two.line(&m);
    assert!(line.starts_with("WARN:"), "{line}");
    assert!(line.contains("convpass"), "{line}");
    assert!(line.contains("iwconv"), "{line}");
    assert!(line.contains("extraction"), "{line}");
}

#[test]
fn sibling_names_share_a_domain_stem() {
    // The incident's pairs: a shared word stem of NAME_STEM_MIN_LEN (4)
    // chars or more makes two names candidates.
    assert_eq!(longest_common_substring_len("convpass", "iwconv"), 4);
    assert_eq!(longest_common_substring_len("autospecissue", "iwissue"), 5);
    // The non-sibling control: only "con" (3) is shared.
    assert_eq!(
        longest_common_substring_len("conflictresolve", "convpass"),
        3
    );

    assert!(name_candidates("convpass.sh", "iwconv.sh"));
    assert!(name_candidates("autospec-issue.sh", "iw-issue.sh"));
    // Case-insensitive, extension-stripped.
    assert!(name_candidates("ConvPass", "IWCONV.sh"));
    // Not candidates:
    assert!(!name_candidates("conflict-resolve.sh", "convpass.sh"));
    assert!(!name_candidates("a.sh", "iwconv.sh"));
    assert!(!name_candidates(
        "gen-harness-runtime-aliases.sh",
        "lint-implementation.sh"
    ));
    assert_eq!(longest_common_substring_len("", "iwconv"), 0);
}

#[test]
fn sibling_content_shares_distinctive_strings() {
    // The incident's pair: two conversion paths re-implementing the same
    // system. `iwconv` is missing the empty-stage guard (#4170) and the
    // base-SHA resolution guard (#4192) that `convpass` carries.
    let (convpass, iwconv) = incident_conversion_pair();

    let shared = distinctive_shared(&convpass, &iwconv);
    for expected in [
        "--check",
        "rev-parse",
        "autospec-core",
        "OUT_ROOT=\"out/issue-",
    ] {
        assert!(
            shared.iter().any(|s| s == expected),
            "expected {expected} in shared: {shared:?}"
        );
    }

    // The pair is a confirmed sibling: name candidates AND more than a
    // small number of shared distinctive strings.
    let pair = sibling_pair("convpass.sh", "iwconv.sh", &convpass, &iwconv);
    let Some(pair) = pair else {
        panic!("expected a sibling finding; shared = {shared:?}");
    };
    assert_eq!(pair.a, "convpass.sh");
    assert_eq!(pair.b, "iwconv.sh");
    assert!(pair.shared.len() > 5, "{shared:?}");
    let line = pair.line();
    assert!(line.starts_with("WARN:"), "{line}");
    assert!(line.contains("convpass.sh"), "{line}");
    assert!(line.contains("iwconv.sh"), "{line}");

    // Control 1: same content, non-sibling names — a content match alone
    // is not a finding.
    assert!(sibling_pair("report.sh", "tagger.sh", &convpass, &iwconv).is_none());

    // Control 2: sibling names, unrelated content — a name match alone is
    // not a finding.
    let unrelated_a =
        "# alpha\nset -euo pipefail\nFOO_DIR=\"${FOO_DIR:?}\"\necho hello-world-marker\n";
    let unrelated_b =
        "# beta\nset -euo pipefail\nBAR_DIR=\"${BAR_DIR:?}\"\necho another-different-line\n";
    assert!(sibling_pair("convpass.sh", "iwconv.sh", unrelated_a, unrelated_b).is_none());

    // Shebangs are not distinctive: every shell script shares them and
    // they identify nothing.
    let toks = distinctive_tokens("#!/usr/bin/env bash\necho hi\n");
    assert!(toks.iter().all(|t| t != "#!/usr/bin/env"));
    // Boilerplate is not distinctive: short or purely alphabetic.
    for word in ["timeout", "set", "pipefail", "300", "-k"] {
        assert!(
            !distinctive_tokens(word).iter().any(|t| t == word),
            "{word}"
        );
    }
    assert!(distinctive_tokens("--check rev-parse out/issue-").len() == 3);
}

#[test]
fn guard_cited_in_one_sibling_is_a_gap() {
    let (convpass, iwconv) = incident_conversion_pair();
    let files = vec![
        ("convpass.sh".into(), convpass),
        ("iwconv.sh".into(), iwconv.clone()),
    ];

    // convpass cites #3495, #4170 and #4192; iwconv cites only #3495.
    assert_eq!(guard_citations(&files[0].1), vec![3495, 4170, 4192]);
    assert_eq!(guard_citations(&files[1].1), vec![3495]);

    let gaps = guard_coverage(&files);
    // #3495 is cited in both files: no gap. #4170 and #4192 are cited in
    // exactly one: gaps, naming the missing sibling.
    let issues: Vec<u32> = gaps.iter().map(|g| g.issue).collect();
    assert_eq!(issues, vec![4170, 4192]);
    for gap in &gaps {
        assert_eq!(gap.present_in, vec!["convpass.sh".to_string()]);
        assert_eq!(gap.missing_from, vec!["iwconv.sh".to_string()]);
        let line = gap.line();
        assert!(line.starts_with("WARN:"), "{line}");
        assert!(line.contains('#'), "{line}");
        assert!(line.contains("iwconv.sh"), "{line}");
    }

    // The control: the same guard cited in every sibling is not a gap.
    let fixed_iwconv = iwconv + "# #4170: empty stage = no patch; refuse conversion.\n";
    let fixed = vec![
        ("convpass.sh".into(), files[0].1.clone()),
        ("iwconv.sh".into(), fixed_iwconv),
    ];
    let fixed_gaps = guard_coverage(&fixed);
    assert_eq!(fixed_gaps.len(), 1, "only #4192 should remain a gap");
    assert_eq!(fixed_gaps[0].issue, 4192);
}

#[test]
fn citation_parsing_rules() {
    // A comment-line `#` + 3..=5 digits is a citation.
    assert_eq!(
        guard_citations("# #3495: child holds the write descriptor"),
        vec![3495]
    );
    assert_eq!(guard_citations("# three digits #123 ok"), vec![123]);
    // Six or more digits is an id or a date, not an issue ref.
    assert_eq!(guard_citations("# #123456"), Vec::<u32>::new());
    // The leading `#` of a shell comment is not a citation (followed by a
    // space, not a digit).
    assert_eq!(
        guard_citations("# 4170 without hash is not a citation"),
        Vec::<u32>::new()
    );
    // A shebang is not a comment.
    assert_eq!(guard_citations("#!/usr/bin/env bash"), Vec::<u32>::new());
    // A non-comment line is not scanned.
    assert_eq!(guard_citations("echo #4289"), Vec::<u32>::new());
    // Indented comments and multiple citations per file.
    assert_eq!(
        guard_citations("  # indented #4170 and #4192"),
        vec![4170, 4192]
    );
}

#[test]
fn the_sixteen_runner_fleet_had_one_guard() {
    // The incident: `timeout -k` for wedged runners was written into one of
    // 16 runners, not the other 15.
    let mut files = Vec::new();
    for i in 1..=16 {
        let mut content = format!(
            "#!/usr/bin/env bash\n# runner-{i:02} — dispatch work items.\nset -euo pipefail\nWORKER_DIR=\"${{AUTOSPEC_WORKER_DIR:?}}\"\ndispatch_batch \"$WORKER_DIR\"\n"
        );
        if i == 1 {
            content.push_str(
                "# #4259: a wedged runner is killed; timeout -k prevents the second wedge.\n",
            );
            content.push_str("timeout -k 300 bash \"$WORKER_DIR/worker.sh\"\n");
        }
        files.push((format!("runner-{i:02}.sh"), content));
    }

    let gaps = guard_coverage(&files);
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert_eq!(gaps[0].issue, 4259);
    assert_eq!(gaps[0].present_in, vec!["runner-01.sh".to_string()]);
    assert_eq!(gaps[0].missing_from.len(), 15);
    let line = gaps[0].line();
    assert!(line.starts_with("WARN:"), "{line}");
    assert!(line.contains("runner-02.sh"), "{line}");
    assert!(line.contains("runner-16.sh"), "{line}");
}

#[test]
fn one_file_fix_with_a_sibling_in_the_tree_is_a_smell() {
    let tree = [
        "convpass.sh".to_string(),
        "iwconv.sh".to_string(),
        "conflict-resolve.sh".to_string(),
        "README.md".to_string(),
    ];

    // The incident's shape: a one-file fix in one sibling while the tree
    // names the other.
    let smell = fix_scope(&["iwconv.sh".to_string()], &tree);
    let line = smell.line();
    assert!(line.starts_with("WARN:"), "{line}");
    assert!(line.contains("iwconv.sh"), "{line}");
    assert!(line.contains("convpass.sh"), "{line}");
    assert!(!line.contains("conflict-resolve.sh"), "{line}");
    assert!(line.to_lowercase().contains("grep"), "{line}");

    // Controls: a one-file fix with no sibling in the tree is fine; a
    // multi-file fix is fine (the author searched the tree); an empty
    // change is fine.
    assert!(fix_scope(&["conflict-resolve.sh".to_string()], &tree)
        .line()
        .starts_with("OK:"));
    assert!(
        fix_scope(&["convpass.sh".to_string(), "iwconv.sh".to_string()], &tree)
            .line()
            .starts_with("OK:")
    );
    assert!(fix_scope(&[], &tree).line().starts_with("OK:"));
}

#[test]
fn audit_names_every_violation_of_the_incident() {
    let (convpass, iwconv) = incident_conversion_pair();
    let files = vec![
        ("convpass.sh".into(), convpass),
        ("iwconv.sh".into(), iwconv),
    ];
    let ledger = incident_ledger();
    let lines = audit(&files, &ledger, &model());

    // Four findings: the four-copy ledger, the sibling pair, and the two
    // guard gaps (#4170, #4192).
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(
        lines[0].starts_with("WARN: guard has 4 copies"),
        "{lines:?}"
    );
    assert!(lines[1].contains("sibling"), "{lines:?}");
    assert!(lines[2].contains("4170"), "{lines:?}");
    assert!(lines[3].contains("4192"), "{lines:?}");
}

#[test]
fn audit_is_quiet_on_the_control_configuration() {
    // Two unrelated files, a ledger with a single implementation, no
    // citations: every invariant holds and `audit` says nothing is wrong.
    let report =
        "# report — summarize gate results.\nset -euo pipefail\nREPORT_DIR=\"${REPORT_DIR:?}\"\nprintf 'gates: %s\\n' \"$REPORT_DIR\"\n";
    let tagger =
        "# tagger — apply frontmatter tags.\nset -euo pipefail\nMANIFEST=\"${MANIFEST:?}\"\nsed -i '1i tags: []' \"$MANIFEST\"\n";
    let files = vec![
        ("report.sh".into(), report.to_string()),
        ("tagger.sh".into(), tagger.to_string()),
    ];
    let mut ledger = CopyLedger::new();
    ledger.add_copy("report");
    let lines = audit(&files, &ledger, &model());
    assert!(lines.iter().all(|l| l.starts_with("OK:")), "{lines:?}");
    assert!(lines.iter().all(|l| !l.starts_with("WARN:")), "{lines:?}");
}

/// The incident's sibling pair, reconstructed. `convpass` (the autospec
/// side) carries the empty-stage guard (#4170) and the base-SHA
/// resolution guard (#4192); `iwconv` (the InferWeave side) carries
/// neither — the configuration that produced the bogus conversion.
fn incident_conversion_pair() -> (String, String) {
    let convpass = r#"#!/usr/bin/env bash
# convpass — autospec patch-to-PR conversion pass.
# #3495: the child, not the parent, must hold the write descriptor on the
# published inode (ETXTBSY).
# #4170: empty stage = no patch; an empty stage is refused, never converted.
set -euo pipefail

PATCH_DIR="${AUTOSPEC_PATCH_DIR:?}"
STAGE_DIR="${AUTOSPEC_STAGE_DIR:?}"
OUT_ROOT="out/issue-"

require_patch() {
  git -C "$STAGE_DIR" apply --check "$PATCH_DIR/patch.diff"
  git -C "$STAGE_DIR" rev-parse --verify HEAD
}

empty_stage_check() {
  if [ -z "$(git -C "$STAGE_DIR" diff --name-only)" ]; then
    echo "WARN: empty stage = no patch; refusing conversion" >&2
    return 1
  fi
}

stage_spec() {
  cp "$PATCH_DIR/spec.md" "$STAGE_DIR/spec.md"
  git -C "$STAGE_DIR" add spec.md
}

# #4192: resolve the base ref to a full SHA before using it.
base_sha="$(git -C "$STAGE_DIR" rev-parse HEAD^{commit})"

run_gate() {
  timeout -k 300 bash -c "cargo test -p autospec-core --no-fail-fast"
}
"#
    .to_string();
    let iwconv = r#"#!/usr/bin/env bash
# iwconv — InferWeave patch-to-PR conversion pass.
# #3495: the child, not the parent, must hold the write descriptor on the
# published inode (ETXTBSY).
set -euo pipefail

PATCH_DIR="${IW_PATCH_DIR:?}"
STAGE_DIR="${IW_STAGE_DIR:?}"
OUT_ROOT="out/issue-"

require_patch() {
  git -C "$STAGE_DIR" apply --check "$PATCH_DIR/patch.diff"
  git -C "$STAGE_DIR" rev-parse --verify HEAD
}

base_sha="$(git -C "$STAGE_DIR" merge-base HEAD origin/main)"

run_gate() {
  timeout -k 300 bash -c "cargo test -p autospec-core --no-fail-fast"
}
"#
    .to_string();
    (convpass, iwconv)
}
