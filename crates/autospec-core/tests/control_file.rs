//! A write to a control file is not a control action until verified
//! against the reader's predicate (issue #4453).
//!
//! The regression this suite pins: eight issues were appended to the
//! dispatcher's hold list with a reason and a timestamp —
//! `3550 repeatedly-dispatched-no-patch 2026-09-11T23:03:45-07:00` — and
//! the dispatcher matches with `grep -qx "$n"`, a *whole-line* match. The
//! holds would never have taken effect: the file would have listed them,
//! the log would have said they were held, and the dispatcher would have
//! kept sending them to GPUs. The format was visible in the file (two
//! bare numbers) and was still not followed.

use std::fs;
use std::path::PathBuf;

use autospec_core::control_file::{
    hold, release, ControlFile, ControlFileError, ControlRecord, HoldRecord, HoldSidecar,
};

fn rec(issue: u64) -> HoldRecord {
    HoldRecord { issue }
}

/// The exact line from the incident: an annotated hold.
const INCIDENT_LINE: &str = "3550 repeatedly-dispatched-no-patch 2026-09-11T23:03:45-07:00";

fn sandbox(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "autospec-core-control-file-{}-{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox");
    dir
}

// ── The reader's predicate defines the record's format ──────────────────

#[test]
fn the_record_renders_exactly_what_the_consumer_matches() {
    // The dispatcher runs `grep -qx "$n" file`: a whole-line match against
    // the bare issue number. The record's rendering is that line and
    // nothing else — no annotation, no timestamp, no padding.
    assert_eq!(rec(3550).to_string(), "3550");
}

#[test]
fn the_incident_line_is_off_format_and_the_consumer_cannot_see_it() {
    // Regression: the annotated line does not match the consumer's
    // whole-line predicate, so a hold written in that format is invisible.
    assert!(!ControlFile::<HoldRecord>::visible(
        INCIDENT_LINE,
        &rec(3550)
    ));

    // And the typed parse rejects it at load time, naming the line — the
    // defect caught at load instead of at dispatch.
    let err = ControlFile::<HoldRecord>::parse(INCIDENT_LINE).unwrap_err();
    match err {
        ControlFileError::OffFormatLine { line } => assert_eq!(line, INCIDENT_LINE),
        other => panic!("expected OffFormatLine, got {other:?}"),
    }
}

#[test]
fn a_free_text_hold_list_cannot_be_loaded_as_one() {
    // The file the incident produced: two bare numbers plus one annotated
    // line. It cannot be loaded as a hold list at all, so it cannot
    // masquerade as one.
    let content = format!("3583\n3584\n{INCIDENT_LINE}\n");
    let err = ControlFile::<HoldRecord>::parse(&content).unwrap_err();
    match err {
        ControlFileError::OffFormatLine { line } => assert_eq!(line, INCIDENT_LINE),
        other => panic!("expected OffFormatLine, got {other:?}"),
    }
}

#[test]
fn the_whole_line_predicate_ignores_prefixes_and_suffixes() {
    // `grep -qx` semantics: `3550` does not match `35500`, and it does
    // not match `3550 anything` either.
    assert!(ControlFile::<HoldRecord>::visible("3550\n", &rec(3550)));
    assert!(!ControlFile::<HoldRecord>::visible("35500\n", &rec(3550)));
    assert!(!ControlFile::<HoldRecord>::visible("35500\n", &rec(355)));
    assert!(!ControlFile::<HoldRecord>::visible(
        INCIDENT_LINE,
        &rec(3550)
    ));
    assert!(ControlFile::<HoldRecord>::visible(
        "3583\n3584\n",
        &rec(3584)
    ));
}

#[test]
fn non_canonical_numbers_are_not_the_record_format() {
    // `03550` parses as 3550 but would render back as `3550`, changing the
    // record's identity under the reader's feet. It is off-format.
    assert!(HoldRecord::parse_line("03550").is_none());
    assert!(HoldRecord::parse_line(" 3550").is_none());
    assert!(HoldRecord::parse_line("+3550").is_none());
    assert!(HoldRecord::parse_line("3550.0").is_none());
    assert_eq!(HoldRecord::parse_line("3550"), Some(rec(3550)));
}

#[test]
fn parse_round_trips_the_file_byte_for_byte() {
    let content = "3584\n3583\n";
    let file = ControlFile::<HoldRecord>::parse(content).unwrap();
    // Sorted, deduplicated, one bare number per line.
    assert_eq!(file.records(), &[rec(3583), rec(3584)]);
    assert_eq!(file.render(), "3583\n3584\n");
}

#[test]
fn a_duplicate_hold_is_a_no_op_never_a_second_line() {
    let mut file = ControlFile::<HoldRecord>::new();
    assert!(file.add(rec(3550)));
    assert!(!file.add(rec(3550)));
    assert_eq!(file.render(), "3550\n");
}

// ── A write verifies its own post-condition ─────────────────────────────

#[test]
fn verify_visible_names_every_record_the_predicate_cannot_see() {
    // The post-condition as a checkable unit: the incident's content is
    // checked against the record it was meant to establish.
    let invisible =
        ControlFile::<HoldRecord>::verify_visible("3583\n3584\n", &[rec(3550), rec(3584)]);
    assert_eq!(invisible, vec!["3550".to_string()]);

    assert_eq!(
        ControlFile::<HoldRecord>::verify_visible(INCIDENT_LINE, &[rec(3550)]),
        vec!["3550".to_string()]
    );

    assert!(ControlFile::<HoldRecord>::verify_visible("3550\n", &[rec(3550)]).is_empty());
    assert!(ControlFile::<HoldRecord>::verify_visible("", &[]).is_empty());
}

#[test]
fn write_verified_publishes_exactly_the_consumer_format() {
    let dir = sandbox("write-verified");
    let path = dir.join("queue-hold.txt");
    let mut file = ControlFile::<HoldRecord>::new();
    file.add(rec(3584));
    file.add(rec(3583));

    file.write_verified(&path).expect("write must verify");
    assert_eq!(fs::read_to_string(&path).unwrap(), "3583\n3584\n");
    let _ = fs::remove_dir_all(&dir);
}

// ── The sidecar keeps the annotation out of the matched record ──────────

#[test]
fn the_sidecar_keeps_reason_and_timestamp_keyed_by_issue() {
    let mut sidecar = HoldSidecar::new();
    sidecar.record(
        3550,
        "repeatedly-dispatched-no-patch",
        "2026-09-11T23:03:45-07:00",
    );
    sidecar.record(3551, "worker pool saturated", "2026-09-11T23:04:01-07:00");

    let annotation = sidecar.annotation(3550).expect("annotation recorded");
    assert_eq!(annotation.reason, "repeatedly-dispatched-no-patch");
    assert_eq!(annotation.recorded_at, "2026-09-11T23:03:45-07:00");
    assert_eq!(sidecar.issues(), vec![3550, 3551]);
    assert!(sidecar.annotation(9999).is_none());

    // The rendered sidecar round-trips byte for byte.
    let rendered = sidecar.render_json();
    let re_read = HoldSidecar::parse_json(&rendered).expect("sidecar parses");
    assert_eq!(re_read, sidecar);
}

#[test]
fn a_corrupt_sidecar_is_an_error_not_an_empty_sidecar() {
    let err = HoldSidecar::parse_json("{ not json").unwrap_err();
    match err {
        ControlFileError::InvalidSidecar { message, .. } => assert!(!message.is_empty()),
        other => panic!("expected InvalidSidecar, got {other:?}"),
    }
}

// ── The procedure: hold and release ─────────────────────────────────────

#[test]
fn hold_writes_the_bare_number_and_the_annotation_side_by_side() {
    let dir = sandbox("hold");
    let holds = dir.join("queue-hold.txt");
    let sidecar = dir.join("queue-hold-annotations.json");

    let receipt = hold(
        &holds,
        &sidecar,
        3550,
        "repeatedly-dispatched-no-patch",
        "2026-09-11T23:03:45-07:00",
    )
    .expect("hold must verify");
    assert_eq!(
        receipt.line(),
        "held #3550 [2026-09-11T23:03:45-07:00]: repeatedly-dispatched-no-patch \
         (verified against the consumer's whole-line predicate; annotation in sidecar)"
    );

    // The hold list carries exactly what `grep -qx 3550` matches.
    assert_eq!(fs::read_to_string(&holds).unwrap(), "3550\n");

    // The richer line lives in the sidecar, keyed by issue.
    let side = HoldSidecar::parse_json(&fs::read_to_string(&sidecar).unwrap()).unwrap();
    assert_eq!(
        side.annotation(3550).map(|a| a.reason.as_str()),
        Some("repeatedly-dispatched-no-patch")
    );

    // And the consumer's own command confirms the post-condition.
    let file = ControlFile::<HoldRecord>::parse(&fs::read_to_string(&holds).unwrap()).unwrap();
    assert!(file.contains(&rec(3550)));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn hold_appends_to_an_existing_hold_list_without_corrupting_it() {
    let dir = sandbox("hold-append");
    let holds = dir.join("queue-hold.txt");
    let sidecar = dir.join("queue-hold-annotations.json");
    fs::write(&holds, "3583\n3584\n").expect("seed hold list");

    hold(
        &holds,
        &sidecar,
        3550,
        "repeatedly-dispatched-no-patch",
        "2026-09-11T23:03:45-07:00",
    )
    .expect("hold must verify");

    assert_eq!(fs::read_to_string(&holds).unwrap(), "3550\n3583\n3584\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn holding_a_free_text_file_fails_loudly_instead_of_extending_it() {
    // The incident file: two bare numbers plus the annotated line. The
    // typed load refuses it before anything is written — the write does
    // not get to claim a hold the dispatcher cannot see.
    let dir = sandbox("hold-off-format");
    let holds = dir.join("queue-hold.txt");
    let sidecar = dir.join("queue-hold-annotations.json");
    fs::write(&holds, format!("3583\n3584\n{INCIDENT_LINE}\n")).expect("seed");

    let err = hold(
        &holds,
        &sidecar,
        3550,
        "another reason",
        "2026-09-11T23:05:00-07:00",
    )
    .expect_err("a free-text hold list must not be appended to");
    match err {
        ControlFileError::OffFormatLine { line } => assert_eq!(line, INCIDENT_LINE),
        other => panic!("expected OffFormatLine, got {other:?}"),
    }
    // Nothing was written: the file is byte-for-byte what it was.
    assert_eq!(
        fs::read_to_string(&holds).unwrap(),
        format!("3583\n3584\n{INCIDENT_LINE}\n")
    );
    assert!(!sidecar.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_hold_without_a_reason_is_refused() {
    let dir = sandbox("hold-blank");
    let holds = dir.join("queue-hold.txt");
    let sidecar = dir.join("queue-hold-annotations.json");

    let err = hold(&holds, &sidecar, 3550, "   ", "2026-09-11T23:05:00-07:00")
        .expect_err("a blank reason is a usage error");
    match err {
        ControlFileError::BlankReason { issue } => assert_eq!(issue, 3550),
        other => panic!("expected BlankReason, got {other:?}"),
    }
    assert!(!holds.exists());
    assert!(!sidecar.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn release_removes_the_hold_and_keeps_the_record_of_why() {
    let dir = sandbox("release");
    let holds = dir.join("queue-hold.txt");
    let sidecar = dir.join("queue-hold-annotations.json");
    hold(
        &holds,
        &sidecar,
        3550,
        "repeatedly-dispatched-no-patch",
        "2026-09-11T23:03:45-07:00",
    )
    .expect("hold");

    release(&holds, &sidecar, 3550).expect("release must verify");

    // The hold list no longer matches the consumer's predicate for 3550.
    assert_eq!(fs::read_to_string(&holds).unwrap(), "");
    assert!(!ControlFile::<HoldRecord>::visible(
        &fs::read_to_string(&holds).unwrap(),
        &rec(3550)
    ));

    // The annotation survives: it is the record, not control state.
    let side = HoldSidecar::parse_json(&fs::read_to_string(&sidecar).unwrap()).unwrap();
    assert_eq!(
        side.annotation(3550).map(|a| a.reason.as_str()),
        Some("repeatedly-dispatched-no-patch")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_release_of_an_unheld_issue_still_verifies_the_write() {
    let dir = sandbox("release-absent");
    let holds = dir.join("queue-hold.txt");
    let sidecar = dir.join("queue-hold-annotations.json");
    fs::write(&holds, "3583\n").expect("seed");

    release(&holds, &sidecar, 9999).expect("releasing an absent hold is a no-op write");
    assert_eq!(fs::read_to_string(&holds).unwrap(), "3583\n");
    let _ = fs::remove_dir_all(&dir);
}
