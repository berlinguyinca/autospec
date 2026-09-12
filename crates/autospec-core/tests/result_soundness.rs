//! Result soundness: the producer must be able to produce a different
//! answer (issue #4210).
//!
//! The regression tests reconstruct the three findings of the investigation:
//! a probe whose tool is absent reporting "nothing found" forever, a TSV row
//! corrupted by two tools in one pass, and a fix verified through a
//! convenience argument that disables the branch production uses.

use autospec_core::result_soundness::{
    check_record, verification_verdict, EditFinding, EditPass, EditTool, Override, ProbeSoundness,
    RecordCheck, RecordSchema,
};

// ---------------------------------------------------------------------------
// Invariant 1: a probe whose tool is absent cannot report a positive
// ---------------------------------------------------------------------------

/// The incident: the architecture probe (`strings <binary> | grep …` inside
/// apptainer images) returned empty for all three images, and the unanimity
/// was believed. `command -v strings` was missing inside every image. A probe
/// without its tool cannot report anything, so its negative is a property of
/// the probe, not of the target.
#[test]
fn probe_without_its_tool_cannot_report_positive() {
    let mut probe = autospec_core::result_soundness::ProbeCheck::new("arch-probe", "strings");
    // `command -v strings` inside every .sif -> MISSING.
    probe.tool_present(false);

    assert_eq!(
        probe.soundness(),
        ProbeSoundness::ToolAbsent {
            tool: "strings".to_string()
        }
    );
    assert!(!probe.negative_is_evidence());
    let line = probe.line();
    assert!(
        line.contains("`strings`"),
        "line names the absent tool: {line}"
    );
    assert!(
        line.contains("cannot report anything"),
        "line says the probe cannot report: {line}"
    );
}

/// A fresh probe is not yet evidence either: the tool has not been verified
/// present and no control has run. Failing closed, never failing open.
#[test]
fn probe_is_unvalidated_before_any_check() {
    let probe = autospec_core::result_soundness::ProbeCheck::new("arch-probe", "strings");
    assert_eq!(probe.soundness(), ProbeSoundness::Unvalidated);
    assert!(!probe.negative_is_evidence());
}

/// The invariant's remedy: `qwen` on an image serving Qwen right now is the
/// free positive control. A positive control validates the probe (and proves
/// the tool works in the environment), and its negatives become evidence.
#[test]
fn positive_control_validates_probe_and_proves_tool_present() {
    let mut probe = autospec_core::result_soundness::ProbeCheck::new("arch-probe", "strings");
    // No tool presence check yet — the control itself is the check.
    probe.record_control("qwen (image serving Qwen)", true);

    assert_eq!(probe.soundness(), ProbeSoundness::Sound);
    assert!(probe.negative_is_evidence());
    assert!(probe.line().contains("known-positive"));
}

/// The second independent cause of the incident: the target was a
/// 17,888-byte wrapper, not a binary. With the tool present but the target
/// not what was assumed, the probe returns negative on the known-positive
/// input — the control catches it and the probe is broken, not the fleet.
#[test]
fn negative_on_known_positive_input_breaks_the_probe() {
    let mut probe = autospec_core::result_soundness::ProbeCheck::new("arch-probe", "strings");
    probe.tool_present(true);
    probe.record_control("qwen (image serving Qwen)", false);

    assert_eq!(
        probe.soundness(),
        ProbeSoundness::Broken {
            control: "qwen (image serving Qwen)".to_string()
        }
    );
    assert!(!probe.negative_is_evidence());
    assert!(probe.line().contains("broken"));
}

/// The incident's order of operations, inverted: the probe returned empty for
/// `glm*`/`deepseek*` in all three images *before* any control was run. With
/// no control, the negative is not evidence — the check costs seconds and was
/// never taken.
#[test]
fn negative_before_control_is_not_evidence() {
    let mut probe = autospec_core::result_soundness::ProbeCheck::new("arch-probe", "strings");
    probe.tool_present(true);
    // Empty for all three images, no control run.
    assert_eq!(probe.soundness(), ProbeSoundness::Unvalidated);
    assert!(!probe.negative_is_evidence());
}

// ---------------------------------------------------------------------------
// Invariant 2: structured data edited by one tool, validated by schema
// ---------------------------------------------------------------------------

/// Every other row of the fleet's model table has seven fields; the schema is
/// the one-line assertion against a known-good row.
#[test]
fn schema_is_derived_from_a_known_good_row() {
    let good =
        "qwen3.8-27b\tmodels--unsloth--Qwen3.8-27B-GGUF\tUD-Q6_K_XL\t6\t299520\t1048576\t131072";
    let schema = RecordSchema::from_row(good).expect("a known-good row names a schema");
    assert_eq!(schema.field_count, 7);
    assert_eq!(check_record(good, schema), RecordCheck::Ok);
    assert_eq!(RecordSchema::from_row("   "), None);
}

/// The incident: a `sed` substitution with a malformed backreference and an
/// `awk` field rewrite in the same pass produced nine fields where every
/// other row has seven — and it still *looks* like a row. The schema check
/// is the one-line assertion that catches what visual inspection cannot.
#[test]
fn corrupted_row_fails_the_schema_check() {
    let schema = RecordSchema { field_count: 7 };
    let corrupted = "glm-5.3-flash\tmodels--unsloth--GLM-5.3-Flash-GGUF\tUD-Q2_K_XL\t2\tUD-Q6_K_XL\t4032\t1048576\t131072\t2";
    assert_eq!(
        check_record(corrupted, schema),
        RecordCheck::FieldCount {
            expected: 7,
            found: 9
        }
    );
    let line = RecordCheck::FieldCount {
        expected: 7,
        found: 9,
    }
    .line();
    assert!(
        line.contains("9 fields"),
        "line reports the found count: {line}"
    );
    assert!(
        line.contains("7"),
        "line reports the expected count: {line}"
    );
}

/// An empty line is not a schema violation: there is no record to check.
#[test]
fn empty_record_is_empty_not_a_violation() {
    assert_eq!(
        check_record("", RecordSchema { field_count: 7 }),
        RecordCheck::Empty
    );
}

/// Two tools in one pass is a finding in itself: the second tool ran over the
/// first's output, inheriting its mistakes while making the result look
/// plausible. With the incident's corrupted row, both findings fire.
#[test]
fn two_tools_on_one_record_is_a_finding() {
    let pass = EditPass::new(vec![EditTool::Text, EditTool::Structured]);
    let schema = RecordSchema { field_count: 7 };
    let corrupted = "glm-5.3-flash\tmodels--unsloth--GLM-5.3-Flash-GGUF\tUD-Q2_K_XL\t2\tUD-Q6_K_XL\t4032\t1048576\t131072\t2";

    let findings = pass.findings(corrupted, schema);
    assert!(
        findings.contains(&EditFinding::MultiTool { count: 2 }),
        "the multi-tool pass is flagged: {findings:?}"
    );
    assert!(
        findings.contains(&EditFinding::SchemaViolation {
            expected: 7,
            found: 9
        }),
        "the corrupted record is flagged: {findings:?}"
    );
    assert!(findings[0].line().contains("one tool"));
}

/// The clean pass: one structure-aware tool, a record that passes the schema.
/// No findings — the invariant is a check, not a prohibition on editing.
#[test]
fn single_structured_tool_with_valid_record_is_clean() {
    let pass = EditPass::new(vec![EditTool::Structured]);
    let schema = RecordSchema { field_count: 7 };
    let good = "glm-5.3-flash\tmodels--unsloth--GLM-5.3-Flash-GGUF\tUD-Q2_K_XL\t2\tUD-Q6_K_XL\t4032\t1048576";
    assert!(pass.findings(good, schema).is_empty());
}

/// A single text-level tool that corrupts the record is still caught by the
/// schema check: the multi-tool finding is about the pass, the schema check
/// is about the output, and the output is what ships.
#[test]
fn single_text_tool_with_corrupted_record_is_caught_by_schema() {
    let pass = EditPass::new(vec![EditTool::Text]);
    let schema = RecordSchema { field_count: 7 };
    let corrupted = "glm-5.3-flash\tgarbage\tUD-Q2_K_XL\t2\tUD-Q6_K_XL\t4032\t1048576\t131072";
    let findings = pass.findings(corrupted, schema);
    assert_eq!(
        findings,
        vec![EditFinding::SchemaViolation {
            expected: 7,
            found: 8
        }]
    );
}

// ---------------------------------------------------------------------------
// Invariant 3: verify through the production call path
// ---------------------------------------------------------------------------

/// Production calls `pick-config.py` without `--vram-mib`, taking the branch
/// that derives VRAM and GPU count from `detect_gpu()`. A verification that
/// reproduces the production arguments exactly exercises that branch.
#[test]
fn verification_matching_production_is_sound() {
    let production = vec!["--model".to_string(), "glm-5.3-flash".to_string()];
    let verification = vec!["--model".to_string(), "glm-5.3-flash".to_string()];
    let overrides = [Override::new("--vram-mib", "detect_gpu()")];

    let verdict = verification_verdict(&production, &verification, &overrides);
    assert!(verdict.is_sound());
    assert!(verdict.line().contains("production call path"));
}

/// The incident: the multi-GPU budget fix was verified with
/// `pick-config.py --vram-mib <sum computed by hand>`. Production calls it
/// *without* `--vram-mib` — the argument passed is the one argument that
/// disables the code under test. The verification cannot fail on the branch
/// it was meant to test.
#[test]
fn verification_with_override_disables_the_branch_under_test() {
    let production = vec!["--model".to_string(), "glm-5.3-flash".to_string()];
    let verification = vec![
        "--model".to_string(),
        "glm-5.3-flash".to_string(),
        "--vram-mib".to_string(),
        "49152".to_string(),
    ];
    let overrides = [Override::new("--vram-mib", "detect_gpu()")];

    let verdict = verification_verdict(&production, &verification, &overrides);
    assert!(!verdict.is_sound());
    assert_eq!(
        verdict,
        autospec_core::result_soundness::PathVerdict::DisablesBranch {
            flag: "--vram-mib".to_string(),
            branch: "detect_gpu()".to_string(),
        }
    );
    let line = verdict.line();
    assert!(line.contains("`--vram-mib`"), "line names the flag: {line}");
    assert!(
        line.contains("`detect_gpu()`"),
        "line names the disabled branch: {line}"
    );
}

/// A verification that differs from production in a way that is not a known
/// override exercises a different invocation: it establishes nothing about
/// the production path, and the differences are named.
#[test]
fn divergent_verification_names_its_differences() {
    let production = vec!["--model".to_string(), "glm-5.3-flash".to_string()];
    let verification = vec![
        "--model".to_string(),
        "deepseek-v4.1".to_string(),
        "--dry-run".to_string(),
    ];
    let overrides = [Override::new("--vram-mib", "detect_gpu()")];

    let verdict = verification_verdict(&production, &verification, &overrides);
    assert!(!verdict.is_sound());
    assert_eq!(
        verdict,
        autospec_core::result_soundness::PathVerdict::Divergent {
            extra: vec!["deepseek-v4.1".to_string(), "--dry-run".to_string()],
            missing: vec!["glm-5.3-flash".to_string()],
        }
    );
    let line = verdict.line();
    assert!(
        line.contains("`--dry-run`"),
        "line names the extra argument: {line}"
    );
    assert!(
        line.contains("`glm-5.3-flash`"),
        "line names the missing argument: {line}"
    );
}

/// An argument both invocations pass is not a divergence: when production
/// and the verification agree on the override flag, the branch is the same
/// and the comparison falls back to the rest of the arguments.
#[test]
fn shared_override_is_not_a_divergence() {
    let production = vec![
        "--model".to_string(),
        "glm-5.3-flash".to_string(),
        "--vram-mib".to_string(),
    ];
    let verification = vec![
        "--model".to_string(),
        "glm-5.3-flash".to_string(),
        "--vram-mib".to_string(),
    ];
    let overrides = [Override::new("--vram-mib", "detect_gpu()")];
    assert!(verification_verdict(&production, &verification, &overrides).is_sound());
}

// ---------------------------------------------------------------------------
// The common thread, end to end
// ---------------------------------------------------------------------------

/// The regression: three findings from one investigation, each a different
/// way of trusting a result that was never produced. In every case the check
/// that would have established that the producer could produce a different
/// answer costs seconds — and in every case it is the one that is run.
#[test]
fn the_three_findings_are_each_caught_by_their_check() {
    // 1. The probe: `strings` missing inside every .sif.
    let mut probe = autospec_core::result_soundness::ProbeCheck::new("arch-probe", "strings");
    probe.tool_present(false);
    assert!(!probe.negative_is_evidence());
    assert!(probe.line().contains("`strings`"));

    // 2. The table: sed + awk in one pass, nine fields where rows have seven.
    let pass = EditPass::new(vec![EditTool::Text, EditTool::Structured]);
    let schema = RecordSchema { field_count: 7 };
    let corrupted = "glm-5.3-flash\tmodels--unsloth--GLM-5.3-Flash-GGUF\tUD-Q2_K_XL\t2\tUD-Q6_K_XL\t4032\t1048576\t131072\t2";
    let findings = pass.findings(corrupted, schema);
    assert!(findings.iter().any(|f| matches!(
        f,
        EditFinding::SchemaViolation {
            expected: 7,
            found: 9
        }
    )));

    // 3. The fix: verified with the one argument that disables the branch
    //    production uses.
    let production = vec!["--model".to_string(), "glm-5.3-flash".to_string()];
    let verification = vec![
        "--model".to_string(),
        "glm-5.3-flash".to_string(),
        "--vram-mib".to_string(),
        "49152".to_string(),
    ];
    let overrides = [Override::new("--vram-mib", "detect_gpu()")];
    let verdict = verification_verdict(&production, &verification, &overrides);
    assert!(!verdict.is_sound());
    assert!(verdict.line().contains("detect_gpu"));
}
