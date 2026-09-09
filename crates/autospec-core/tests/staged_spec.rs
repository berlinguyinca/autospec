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

use autospec_core::staged_spec::{
    authorize, format_timestamp, parse_timestamp, staged_at, staged_source_updated_at,
    DispatchVerdict, EnvironmentProbe, IssueComment, IssueSnapshot, ProbeState, RefuseReason,
    ABSENT, BODY_SECTION, DISCUSSION_SECTION, ENVIRONMENT_SECTION, HEADER_COMMENTS, HEADER_ISSUE,
    HEADER_SOURCE_UPDATED_AT, HEADER_STAGED_AT, KEY_ABSENT, KEY_CONTAINER_RUNTIME, KEY_DATABASE,
    KEY_REGISTRY, NOT_PROBED,
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
