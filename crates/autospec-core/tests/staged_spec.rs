//! Staged-spec freshness and completeness (#3864).
//!
//! The merge host stages an issue as markdown because the dispatch worker has
//! no read access to the tracker. That makes the staged file the only thing
//! standing between the worker and the task, so these tests pin the two ways
//! it fails: the copy is incomplete (the clarification filed an hour after the
//! body was written is not in it), and the copy is un-ageable (nothing records
//! which revision of the issue it was taken from, so staleness is invisible
//! until a 900-line rework lands).
//!
//! Every verdict has both a positive and a negative case, because the
//! fail-closed half is the half that costs the dispatcher a run.

use autospec_core::grading::{staged_gates, GateSet, GATES_SECTION};
use autospec_core::staged_spec::{
    authorize, format_timestamp, is_mechanical_author, parse_timestamp, prompt_verdict,
    spec_is_empty, staged_at, staged_source_updated_at, DispatchVerdict, EnvironmentProbe,
    IssueComment, IssueSnapshot, ProbeState, PromptVerdict, RefuseReason, SpecReceipt, ABSENT,
    BODY_SECTION, BODY_TRUNCATION_MARKER, DISCUSSION_SECTION, ENVIRONMENT_SECTION, HEADER_COMMENTS,
    HEADER_ISSUE, HEADER_SOURCE_UPDATED_AT, HEADER_STAGED_AT, KEY_ABSENT, KEY_CONTAINER_RUNTIME,
    KEY_DATABASE, KEY_REGISTRY, MECHANICAL_AUTHORS, NOT_PROBED, NO_SPEC_STATUS, RUNNER_SPEC_BUDGET,
};

const STAGED_AT: u64 = 1_757_000_000; // 2025-09-04T15:33:20Z
const SOURCE_UPDATED_AT: u64 = 1_756_999_000; // before staging
const BODY_EDITED_AT: u64 = 1_756_998_000; // before the clarifications

fn comment(author: &str, created_at: u64, body: &str) -> IssueComment {
    IssueComment {
        author: author.to_string(),
        created_at,
        body: body.to_string(),
    }
}

/// The issue as it looked at staging time: body edited once, then two
/// clarifications filed after that edit.
fn snapshot() -> IssueSnapshot {
    IssueSnapshot {
        number: 50,
        title: "Run the batch on the cluster".to_string(),
        body: "## Goal\n\nDispatch the batch.\n\n## Acceptance criteria\n\n- [ ] `autospec dispatch --issue 50` exits 0\n".to_string(),
        source_updated_at: SOURCE_UPDATED_AT,
        body_updated_at: Some(BODY_EDITED_AT),
        comments: vec![
            comment("maintainer", BODY_EDITED_AT - 600, "Draft, ignore."),
            comment(
                "maintainer",
                SOURCE_UPDATED_AT,
                "Do not use `docker`: the cluster runs apptainer.",
            ),
            comment(
                "operator",
                SOURCE_UPDATED_AT + 60,
                "The merge host stages; the worker's gh is unauthenticated.",
            ),
        ],
        gates: Vec::new(),
    }
}

fn environment() -> EnvironmentProbe {
    EnvironmentProbe::new()
        .with(
            KEY_CONTAINER_RUNTIME,
            ProbeState::Present {
                value: "/usr/bin/apptainer".to_string(),
            },
            Some("$PATH lookup".to_string()),
        )
        .with(
            KEY_DATABASE,
            ProbeState::Absent,
            Some("no listener".to_string()),
        )
        .with(KEY_REGISTRY, ProbeState::NotProbed, None)
        .with(
            KEY_ABSENT,
            ProbeState::Present {
                value: "docker, podman, postgres".to_string(),
            },
            None,
        )
}

fn staged() -> String {
    snapshot().stage(&environment(), STAGED_AT)
}

/// The snapshot with no comments and the given body: the budget tests vary
/// the body and the discussion, not the rest of the issue.
fn snapshot_with_body(body: String) -> IssueSnapshot {
    IssueSnapshot {
        body,
        comments: Vec::new(),
        ..snapshot()
    }
}

// ---------------------------------------------------------------- staging --

#[test]
fn a_comment_after_the_last_body_edit_is_part_of_the_staged_spec() {
    let text = staged();

    assert!(text.contains("Do not use `docker`"), "{text}");
    assert!(text.contains("unauthenticated"), "{text}");
    assert!(text.contains("### Comment 1 — maintainer at "), "{text}");
    assert!(text.contains("### Comment 2 — operator at "), "{text}");
}

#[test]
fn a_comment_before_the_last_body_edit_is_left_out_of_the_discussion() {
    let text = staged();

    assert!(!text.contains("Draft, ignore."), "{text}");
    // The count says two of three, so the exclusion is visible, not silent.
    assert!(
        text.contains(&format!("{HEADER_COMMENTS} 2 of 3")),
        "{text}"
    );
}

#[test]
fn an_unknown_body_edit_time_keeps_every_comment() {
    let mut source = snapshot();
    source.body_updated_at = None;

    let text = source.stage(&environment(), STAGED_AT);

    assert!(text.contains("Draft, ignore."), "{text}");
    assert!(
        text.contains(&format!("{HEADER_COMMENTS} 3 of 3")),
        "{text}"
    );
}

#[test]
fn an_issue_with_no_comments_stages_an_explicit_empty_discussion() {
    let mut source = snapshot();
    source.comments.clear();

    let text = source.stage(&environment(), STAGED_AT);

    assert!(
        text.contains("No comments since the last body edit"),
        "{text}"
    );
}

#[test]
fn the_staged_spec_records_the_source_revision_and_the_staging_time() {
    let text = staged();

    assert!(text.contains(&format!("{HEADER_ISSUE} 50")), "{text}");
    assert!(
        text.contains(&format!(
            "{HEADER_SOURCE_UPDATED_AT} {SOURCE_UPDATED_AT} ({})",
            format_timestamp(SOURCE_UPDATED_AT)
        )),
        "{text}"
    );
    assert_eq!(staged_source_updated_at(&text), Some(SOURCE_UPDATED_AT));
    assert_eq!(staged_at(&text), Some(STAGED_AT));
}

#[test]
fn the_body_is_staged_verbatim_so_the_acceptance_criteria_need_no_tracker() {
    let text = staged();

    assert!(text.contains(BODY_SECTION), "{text}");
    assert!(
        text.contains("- [ ] `autospec dispatch --issue 50` exits 0"),
        "{text}"
    );
    // The discussion precedes the body: the reader sees the clarification
    // before the text it clarifies.
    assert!(
        text.find(DISCUSSION_SECTION).expect("discussion section")
            < text.find(BODY_SECTION).expect("body section"),
        "{text}"
    );
}

#[test]
fn the_staged_spec_declares_the_container_runtime_and_what_is_absent() {
    let text = staged();

    assert!(text.contains(ENVIRONMENT_SECTION), "{text}");
    assert!(
        text.contains("- container-runtime: /usr/bin/apptainer [$PATH lookup]"),
        "{text}"
    );
    assert!(text.contains("- database: absent [no listener]"), "{text}");
    assert!(
        text.contains(&format!("- registry: {NOT_PROBED}")),
        "{text}"
    );
    assert!(
        text.contains("- absent: docker, podman, postgres"),
        "{text}"
    );
    assert!(text.contains(ABSENT), "{text}");
}

#[test]
fn a_quoted_header_in_the_body_cannot_forge_the_staged_revision() {
    let mut source = snapshot();
    source.body =
        format!("Notes:\n\n{HEADER_SOURCE_UPDATED_AT} 9999999999\n{HEADER_STAGED_AT} 1\n");

    let text = source.stage(&environment(), STAGED_AT);

    // The header region is the leading metadata run only; the quoted lines sit
    // under `## Issue body` and are ignored by the reader.
    assert_eq!(staged_source_updated_at(&text), Some(SOURCE_UPDATED_AT));
    assert_eq!(staged_at(&text), Some(STAGED_AT));
}

// ---------------------------------------------------------- gate set --

#[test]
fn the_staged_spec_carries_the_gate_set_the_patch_is_graded_against() {
    let set = GateSet::rust_workspace();
    let mut source = snapshot();
    source.gates = set.gates().to_vec();

    let text = source.stage(&environment(), STAGED_AT);

    assert!(text.contains(GATES_SECTION), "{text}");
    // The gate set is the last section: the acceptance criteria the worker
    // satisfies come before the gates the result is graded against.
    assert!(
        text.rfind(GATES_SECTION).expect("gate section")
            > text.find(BODY_SECTION).expect("body section"),
        "{text}"
    );
    for gate in set.gates() {
        assert!(text.contains(&format!("- [ ] {}", gate.command)), "{text}");
    }
    // And it reads back as the same commands, in the same order: the spec
    // names the gate set the grade enforces, not a weaker one.
    assert_eq!(staged_gates(&text), Some(set.commands()));
}

#[test]
fn a_snapshot_with_no_gates_stages_no_gate_section() {
    let text = staged();

    assert!(!text.contains(GATES_SECTION), "{text}");
    assert_eq!(staged_gates(&text), None);
}

#[test]
fn a_snapshot_document_without_a_gates_field_reads_back_with_none() {
    // A pre-#3925 snapshot document: no `gates` key at all. The field defaults
    // to empty rather than failing, so older serialized snapshots stay
    // readable.
    let json = r#"{
        "number": 50,
        "title": "Run the batch on the cluster",
        "body": "Dispatch the batch.",
        "source_updated_at": 1756999000,
        "body_updated_at": 1756998000,
        "comments": []
    }"#;
    let source: IssueSnapshot = serde_json::from_str(json).expect("deserializes");

    assert!(source.gates.is_empty());
    assert_eq!(staged_gates(&source.stage(&environment(), STAGED_AT)), None);
}

// ---------------------------------------- mechanical filter (#4020) --

#[test]
fn a_correcting_comment_after_the_body_edit_is_staged_in_order() {
    let text = staged();

    // Both clarifications are staged, oldest first: the correction that moved
    // the task is present, and the later comment does not overtake it.
    let first = text
        .find("### Comment 1 — maintainer at ")
        .expect("first comment");
    let second = text
        .find("### Comment 2 — operator at ")
        .expect("second comment");
    assert!(first < second, "{text}");
    assert!(text.contains("Do not use `docker`"), "{text}");
    // The correction stays a correction: it is not merged into the body, the
    // body stays what the tracker says it is.
    let body_start = text.find(BODY_SECTION).expect("body section");
    assert!(
        !text[body_start..].contains("Do not use `docker`"),
        "{text}"
    );
}

#[test]
fn comments_by_declared_mechanical_authors_are_never_staged() {
    let mut source = snapshot();
    source.comments.push(comment(
        "dependabot[bot]",
        SOURCE_UPDATED_AT + 120,
        "Bumped a dependency.",
    ));

    let text = source.stage(&environment(), STAGED_AT);

    assert!(!text.contains("Bumped a dependency."), "{text}");
    // 2 of 3: the bot post is not even counted — the denominator is the
    // comments that can appear at all, not every comment on the issue.
    assert!(
        text.contains(&format!("{HEADER_COMMENTS} 2 of 3")),
        "{text}"
    );
    // The human clarifications are unaffected by the filter.
    assert!(text.contains("Do not use `docker`"), "{text}");
    assert!(text.contains("unauthenticated"), "{text}");
}

#[test]
fn a_bot_only_discussion_stages_byte_identically_to_no_discussion() {
    let mut plain = snapshot();
    plain.comments.clear();

    let mut bot_only = snapshot();
    bot_only.comments = vec![
        comment("dependabot[bot]", SOURCE_UPDATED_AT, "Bumped a dependency."),
        comment(
            "github-actions[bot]",
            SOURCE_UPDATED_AT + 30,
            "Workflow ran.",
        ),
    ];

    let plain_text = plain.stage(&environment(), STAGED_AT);
    let bot_text = bot_only.stage(&environment(), STAGED_AT);

    assert_eq!(plain_text, bot_text);
    assert!(
        plain_text.contains(&format!("{HEADER_COMMENTS} 0 of 0")),
        "{plain_text}"
    );
    assert!(
        plain_text.contains("No comments since the last body edit"),
        "{plain_text}"
    );
    assert!(!plain_text.contains("Bumped a dependency."), "{plain_text}");
}

#[test]
fn mechanical_matching_is_exact_and_case_insensitive_not_substring() {
    // The list is the declaration the filter is allowed to use: no
    // heuristics beyond it.
    assert_eq!(
        MECHANICAL_AUTHORS,
        &["dependabot[bot]", "github-actions[bot]"]
    );
    assert!(is_mechanical_author("dependabot[bot]"));
    assert!(is_mechanical_author("DEPENDABOT[BOT]"));
    assert!(is_mechanical_author("github-actions[bot]"));
    // A human login that merely contains a bot name is not a bot.
    assert!(!is_mechanical_author("my-dependabot[bot]"));
    assert!(!is_mechanical_author("dependabot"));
    assert!(!is_mechanical_author(""));
}

#[test]
fn a_human_comment_about_a_bot_is_staged() {
    let mut source = snapshot();
    source.comments.push(comment(
        "operator",
        SOURCE_UPDATED_AT + 90,
        "Ignore the dependabot[bot] posts: they are tracker noise.",
    ));

    let text = source.stage(&environment(), STAGED_AT);

    // The filter is on the author, never on the content.
    assert!(text.contains("Ignore the dependabot[bot] posts"), "{text}");
    assert!(
        text.contains(&format!("{HEADER_COMMENTS} 3 of 4")),
        "{text}"
    );
}

// --------------------------------------------------- spec budget (#4020) --

#[test]
fn a_spec_at_or_under_the_budget_is_staged_verbatim() {
    let source = snapshot_with_body(format!("## Goal\n\n{}\n", "x".repeat(20_000)));

    let text = source.stage(&environment(), STAGED_AT);

    assert!(!text.contains(BODY_TRUNCATION_MARKER), "{text}");
    assert!(text.contains(&"x".repeat(20_000)), "the body is verbatim");
    assert!(text.len() < RUNNER_SPEC_BUDGET, "{}", text.len());
}

#[test]
fn an_over_budget_body_is_truncated_with_a_marker_not_the_comments() {
    let mut source = snapshot_with_body("x".repeat(40_000));
    source.comments = vec![comment(
        "maintainer",
        SOURCE_UPDATED_AT,
        "Do not use `docker`: the cluster runs apptainer.",
    )];

    let text = source.stage(&environment(), STAGED_AT);

    assert!(text.contains(BODY_TRUNCATION_MARKER), "{text}");
    // The whole discussion survives: truncation is at the body, not the
    // comments.
    assert!(
        text.contains("Do not use `docker`: the cluster runs apptainer."),
        "{text}"
    );
    assert!(!text.contains(&"x".repeat(40_000)), "the body was cut");
    // The cut lands exactly on the budget: an ASCII body needs no
    // boundary back-off, so nothing overflows past it.
    assert_eq!(text.len(), RUNNER_SPEC_BUDGET, "{}", text.len());
}

#[test]
fn a_truncated_body_ends_on_a_character_boundary() {
    // Every body character is two bytes: a cut on a non-boundary byte
    // offset would panic the slice rather than produce a spec.
    let source = snapshot_with_body("é".repeat(20_000));

    let text = source.stage(&environment(), STAGED_AT);

    assert!(text.contains(BODY_TRUNCATION_MARKER), "{text}");
    let start = text.find(BODY_SECTION).expect("body section") + BODY_SECTION.len();
    let end = text.find(BODY_TRUNCATION_MARKER).expect("marker");
    let body_part = &text[start..end];
    // body_part is "\n" plus the kept body; a boundary-safe cut keeps whole
    // two-byte characters.
    assert_eq!(body_part[1..].len() % 2, 0, "{}", body_part[1..].len());
}

#[test]
fn comments_that_exceed_the_budget_shrink_the_body_to_the_marker() {
    let mut source = snapshot_with_body(String::from("Dispatch the batch.\n"));
    source.comments.push(comment(
        "maintainer",
        SOURCE_UPDATED_AT,
        &"x".repeat(40_000),
    ));

    let text = source.stage(&environment(), STAGED_AT);

    // The discussion is never truncated: the whole comment is there even
    // though it alone exceeds the budget.
    assert!(text.contains(&"x".repeat(40_000)), "the comment is intact");
    // The body gives way entirely: the marker is all that is left of it, and
    // the spec may exceed the budget — the body is what may not.
    assert!(text.contains(BODY_TRUNCATION_MARKER), "{text}");
    assert!(!text.contains("Dispatch the batch."), "{text}");
    assert!(text.len() > RUNNER_SPEC_BUDGET, "{}", text.len());
}

#[test]
fn a_no_comment_spec_is_byte_identical_to_the_pre_4020_layout() {
    // The budget must not move a byte of a spec that already fits: this is
    // the exact layout staging produced before #4020, pinned byte for byte.
    let mut source = snapshot();
    source.comments.clear();

    let text = source.stage(&environment(), STAGED_AT);

    let expected = format!(
        "{HEADER_ISSUE} 50\n\
         {HEADER_STAGED_AT} {STAGED_AT} ({})\n\
         {HEADER_SOURCE_UPDATED_AT} {SOURCE_UPDATED_AT} ({})\n\
         {HEADER_COMMENTS} 0 of 0\n\
         \n\
         # Issue #50: Run the batch on the cluster\n\
         \n\
         {ENVIRONMENT_SECTION}\n\
         - container-runtime: /usr/bin/apptainer [$PATH lookup]\n\
         - database: absent [no listener]\n\
         - registry: {NOT_PROBED}\n\
         - absent: docker, podman, postgres\n\
         \n\
         {DISCUSSION_SECTION}\n\
         _No comments since the last body edit at {}._\n\
         \n\
         {BODY_SECTION}\n\
         ## Goal\n\
         \n\
         Dispatch the batch.\n\
         \n\
         ## Acceptance criteria\n\
         \n\
         - [ ] `autospec dispatch --issue 50` exits 0\n",
        format_timestamp(STAGED_AT),
        format_timestamp(SOURCE_UPDATED_AT),
        format_timestamp(BODY_EDITED_AT)
    );

    assert_eq!(text, expected);
}

// ------------------------------------------------------------- freshness --

#[test]
fn a_staged_spec_matching_the_live_issue_proceeds() {
    let verdict = authorize(Some(&staged()), Some(SOURCE_UPDATED_AT));

    assert_eq!(
        verdict,
        DispatchVerdict::Proceed {
            source_updated_at: SOURCE_UPDATED_AT,
            staged_at: Some(STAGED_AT),
        }
    );
    assert!(!verdict.held());
    assert!(!verdict.needs_restage());
}

#[test]
fn a_moved_issue_holds_the_dispatch_for_re_staging() {
    let live = SOURCE_UPDATED_AT + 300;
    let verdict = authorize(Some(&staged()), Some(live));

    assert!(verdict.held());
    assert!(verdict.needs_restage());
    assert_eq!(
        verdict,
        DispatchVerdict::Restage {
            staged_source_updated_at: SOURCE_UPDATED_AT,
            live_updated_at: live,
            staged_at: Some(STAGED_AT),
        }
    );
    let line = verdict.line(50, "issues/50.md");
    assert!(line.contains("STALE"), "{line}");
    assert!(line.contains("issue 50"), "{line}");
    assert!(line.contains("re-stage"), "{line}");
    // Both revisions are named so the operator can see how far behind it is.
    assert!(
        line.contains(&format_timestamp(SOURCE_UPDATED_AT))
            && line.contains(&format_timestamp(live)),
        "{line}"
    );
}

#[test]
fn no_staged_spec_at_all_is_a_refusal() {
    let verdict = authorize(None, Some(SOURCE_UPDATED_AT));

    assert_eq!(
        verdict,
        DispatchVerdict::Refuse {
            reason: RefuseReason::StagedSpecAbsent,
            staged_at: None,
        }
    );
    assert!(verdict.held());
    assert!(!verdict.needs_restage());
}

#[test]
fn a_missing_staged_spec_reads_as_no_spec_not_a_baseline_problem() {
    // The #3620 run's status read like a baseline problem even though the
    // real fact was simpler: the agent was never told what to do. The
    // refusal carries its own status token for exactly this.
    let verdict = authorize(None, Some(SOURCE_UPDATED_AT));

    let line = verdict.line(15, "iw/issues/15.md");
    assert!(line.contains(NO_SPEC_STATUS), "{line}");
    assert!(line.contains("issue 15"), "{line}");
}

#[test]
fn an_empty_staged_spec_is_no_spec_and_is_refused() {
    let verdict = authorize(Some(""), Some(SOURCE_UPDATED_AT));

    assert_eq!(
        verdict,
        DispatchVerdict::Refuse {
            reason: RefuseReason::NoSpec { bytes: 0 },
            staged_at: None,
        }
    );
    assert!(verdict.held());
    // An empty spec is refused before the revision is even parsed: the
    // verdict does not pretend the file was aged and found un-ageable.
    assert!(!matches!(
        verdict,
        DispatchVerdict::Refuse {
            reason: RefuseReason::NoStagedRevision,
            ..
        }
    ));
    let line = verdict.line(15, "iw/issues/15.md");
    assert!(line.contains(NO_SPEC_STATUS), "{line}");
    assert!(line.contains("0 bytes"), "{line}");
    assert!(line.contains("REFUSED"), "{line}");
}

#[test]
fn a_whitespace_only_staged_spec_is_no_spec_and_is_refused() {
    // A bare existence check (`[ -f "$f" ]`) passes on this file; only the
    // checked read (`[ -s "$f" ]`, here `spec_is_empty`) sees through it.
    let whitespace = "\n  \n\t\n";
    assert!(spec_is_empty(whitespace), "sanity: whitespace is empty");

    let verdict = authorize(Some(whitespace), Some(SOURCE_UPDATED_AT));

    assert_eq!(
        verdict,
        DispatchVerdict::Refuse {
            reason: RefuseReason::NoSpec {
                bytes: whitespace.len(),
            },
            staged_at: None,
        }
    );
    assert!(verdict.line(15, "iw/issues/15.md").contains(NO_SPEC_STATUS));
}

#[test]
fn spec_is_empty_catches_the_swallowed_cat() {
    assert!(spec_is_empty(""));
    assert!(spec_is_empty("   \n\t  "));
    assert!(!spec_is_empty("anything"));
}

#[test]
fn a_spec_recording_no_revision_cannot_be_aged_and_is_refused() {
    // The pre-#3864 staged spec: faithful, complete-looking, and un-ageable.
    let legacy = "# Issue #50: Run the batch on the cluster\n\nDispatch the batch.\n";
    let verdict = authorize(Some(legacy), Some(SOURCE_UPDATED_AT));

    assert_eq!(
        verdict,
        DispatchVerdict::Refuse {
            reason: RefuseReason::NoStagedRevision,
            staged_at: None,
        }
    );
    assert!(verdict.held());
    // A refusal that cannot even say when it was staged says so out loud.
    assert!(
        verdict.line(50, "issues/50.md").contains("cannot be aged"),
        "{}",
        verdict.line(50, "issues/50.md")
    );
}

#[test]
fn a_dispatcher_that_cannot_read_the_live_issue_refuses_instead_of_assuming() {
    let verdict = authorize(Some(&staged()), None);

    assert!(verdict.held());
    assert!(!verdict.needs_restage());
    assert_eq!(
        verdict,
        DispatchVerdict::Refuse {
            reason: RefuseReason::LiveUnknown {
                detail: "the source issue's updatedAt could not be read".to_string(),
            },
            staged_at: Some(STAGED_AT),
        }
    );
    // Acceptance criterion 4: the refusal names the issue and the last known
    // staging time.
    let line = verdict.line(50, "issues/50.md");
    assert!(line.contains("REFUSED"), "{line}");
    assert!(line.contains("issue 50"), "{line}");
    assert!(line.contains(&STAGED_AT.to_string()), "{line}");
    assert!(line.contains("last known staging time"), "{line}");
}

#[test]
fn a_missing_staged_spec_is_refused_before_the_live_read_matters() {
    // The absence is decisive on its own: an unreadable tracker must not turn
    // "nothing staged" into a different, softer verdict.
    let verdict = authorize(None, None);

    assert_eq!(
        verdict,
        DispatchVerdict::Refuse {
            reason: RefuseReason::StagedSpecAbsent,
            staged_at: None,
        }
    );
}

#[test]
fn the_json_verdict_carries_the_exit_code_a_wrapper_branches_on() {
    let json = authorize(Some(&staged()), Some(SOURCE_UPDATED_AT + 1)).to_json(50, "issues/50.md");

    assert!(json.contains("\"held\": true"), "{json}");
    assert!(json.contains("\"needs_restage\": true"), "{json}");
    assert!(json.contains("\"restage\""), "{json}");
    assert!(json.contains("\"issue\": 50"), "{json}");
}

// ---------------------------------------------------------- no-spec gate --

#[test]
fn the_spec_receipt_records_the_bytes_and_checksum_a_run_saw() {
    let text = staged();
    let receipt = SpecReceipt::of(&text);

    assert_eq!(receipt.bytes, text.len());
    // A known hash pins the algorithm and the casing.
    assert_eq!(
        SpecReceipt::of("hello").sha256,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    let line = receipt.line();
    assert!(line.starts_with("spec-bytes="), "{line}");
    assert!(line.contains("spec-sha256="), "{line}");
    assert_eq!(
        line.len(),
        "spec-bytes= spec-sha256=".len() + text.len().to_string().len() + 64
    );
}

#[test]
fn the_prompt_assertion_catches_an_empty_issue_section() {
    let spec = staged();

    // The happy path: the prompt embeds the staged spec between its markers.
    let prompt = format!("===== ISSUE #50 =====\n{spec}===== END ISSUE =====");
    assert_eq!(prompt_verdict(&prompt, &spec), PromptVerdict::CarriesSpec);

    // The #3620 shape: the cat failed into an unchecked command substitution,
    // $BODY became empty, and the prompt arrived with nothing between.
    let hollow = "===== ISSUE #15 =====\n\n===== END ISSUE =====\n";
    assert_eq!(
        prompt_verdict(hollow, &spec),
        PromptVerdict::SpecMissingFromPrompt
    );

    // An empty prompt assembled from nothing at all.
    assert_eq!(prompt_verdict("   ", &spec), PromptVerdict::EmptyPrompt);

    // A spec nobody has — even with a fat prompt — is still no task.
    assert_eq!(
        prompt_verdict(hollow, ""),
        PromptVerdict::SpecMissingFromPrompt
    );

    let refusal = PromptVerdict::SpecMissingFromPrompt.line(15);
    assert!(refusal.contains(NO_SPEC_STATUS), "{refusal}");
    assert!(refusal.contains("issue 15"), "{refusal}");
}

// ------------------------------------------------------------ timestamps --

#[test]
fn a_github_instant_parses_to_epoch_and_formats_back() {
    assert_eq!(
        parse_timestamp(&format_timestamp(SOURCE_UPDATED_AT)),
        Some(SOURCE_UPDATED_AT)
    );
    assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(
        parse_timestamp("2026-09-08T07:03:00Z"),
        Some(1_788_850_980) // leap-year aware, and not a round day
    );
    assert_eq!(parse_timestamp("1757000000"), Some(STAGED_AT));
}

#[test]
fn an_unparseable_instant_is_none_rather_than_zero() {
    for candidate in [
        "",
        "not a timestamp",
        "2026-13-08T07:03:00Z",
        "2026-09-32T07:03:00Z",
        "2026-09-08 07:03:00Z",
        "2026-09-08T07:03:00",
        "2026-09-08T07:03:00+0100",
        "-5",
    ] {
        assert_eq!(parse_timestamp(candidate), None, "parsed {candidate}");
    }
}

#[test]
fn every_day_of_a_leap_cycle_round_trips() {
    // 400 years is the full Gregorian leap cycle, century exceptions included:
    // a mis-signed term in the civil-date conversion shifts the day, which is
    // exactly the error that would make a correctly staged spec read as stale.
    let day = 86_400u64;
    let start = 0u64; // 1970-01-01
    for offset in 0..146_097 {
        let secs = start + offset * day;
        assert_eq!(
            parse_timestamp(&format_timestamp(secs)),
            Some(secs),
            "day {offset} ({})",
            format_timestamp(secs)
        );
    }
}

#[test]
fn the_century_non_leap_year_is_not_a_leap_year() {
    let february_28_2100 = parse_timestamp("2100-02-28T00:00:00Z").expect("parses");
    assert_eq!(
        format_timestamp(february_28_2100 + 86_400),
        "2100-03-01T00:00:00Z"
    );
    // And 2000 is one.
    let february_28_2000 = parse_timestamp("2000-02-28T00:00:00Z").expect("parses");
    assert_eq!(
        format_timestamp(february_28_2000 + 86_400),
        "2000-02-29T00:00:00Z"
    );
}

#[test]
fn fractional_and_offset_instants_parse() {
    assert_eq!(
        parse_timestamp("2026-09-08T07:03:00.250Z"),
        Some(1_788_850_980)
    );
    assert_eq!(
        parse_timestamp("2026-09-08T09:03:00+02:00"),
        Some(1_788_850_980)
    );
    assert_eq!(
        parse_timestamp("2026-09-08T02:03:00-05:00"),
        Some(1_788_850_980)
    );
}
