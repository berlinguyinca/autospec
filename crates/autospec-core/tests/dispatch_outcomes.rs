//! Integration tests for the dispatch outcome ledger and report (#4025).

use autospec_core::dispatch_outcomes::*;
use std::fs;

fn record(id: &str, model: &str, spec_bytes: u64) -> DispatchRecord {
    DispatchRecord::new(id, "3941", model, spec_bytes, 1800, 1_700_000_000)
}

fn ledger(dir: &std::path::Path) -> (DispatchLedger, std::path::PathBuf) {
    let path = dir.join("dispatch-outcomes.jsonl");
    (DispatchLedger::open(&path), path)
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "autospec-dispatch-outcomes-{}-{}",
        tag,
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn bands_boundaries() {
    assert_eq!(spec_band_for(0), SpecSizeBand::Small);
    assert_eq!(
        spec_band_for(SPEC_BAND_SMALL_MAX_BYTES - 1),
        SpecSizeBand::Small
    );
    assert_eq!(
        spec_band_for(SPEC_BAND_SMALL_MAX_BYTES),
        SpecSizeBand::Medium
    );
    assert_eq!(
        spec_band_for(SPEC_BAND_MEDIUM_MAX_BYTES - 1),
        SpecSizeBand::Medium
    );
    assert_eq!(
        spec_band_for(SPEC_BAND_MEDIUM_MAX_BYTES),
        SpecSizeBand::Large
    );
}

#[test]
fn ledger_round_trip_and_last_line_wins() {
    let dir = tempdir("round-trip");
    let (ledger, path) = ledger(&dir);
    let dispatch = record("d-1", "q27-a", 2048);
    ledger.append(&dispatch).unwrap();
    ledger
        .record_terminal("d-1", 1_700_000_900, TerminalOutcome::PatchProduced)
        .unwrap();
    ledger
        .record_conversion(
            "d-1",
            ConversionOutcome::Held {
                reason: "scope".into(),
            },
        )
        .unwrap();

    let raw = fs::read_to_string(&path).unwrap();
    assert_eq!(
        raw.lines().count(),
        3,
        "append-only: one line per state change"
    );

    let loaded = ledger.load().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].wall_clock_secs(), Some(900));
    assert_eq!(loaded[0].terminal, Some(TerminalOutcome::PatchProduced));
    assert_eq!(
        loaded[0].conversion,
        Some(ConversionOutcome::Held {
            reason: "scope".into()
        })
    );
}

#[test]
fn load_missing_file_is_empty_and_unknown_ids_fail() {
    let dir = tempdir("missing");
    let (ledger, _) = ledger(&dir);
    assert!(ledger.load().unwrap().is_empty());
    assert!(ledger
        .record_terminal("nope", 0, TerminalOutcome::Timeout)
        .is_err());
    assert!(ledger
        .record_conversion("nope", ConversionOutcome::Converted)
        .is_err());
}

#[test]
fn corrupt_line_fails_load() {
    let dir = tempdir("corrupt");
    let (_, path) = ledger(&dir);
    fs::write(&path, "not json\n").unwrap();
    assert!(DispatchLedger::open(&path).load().is_err());
}

#[test]
fn conversion_requires_terminal_and_held_requires_reason() {
    let mut r = record("d-2", "q27-a", 100);
    r.conversion = Some(ConversionOutcome::Converted);
    assert!(
        r.validate().is_err(),
        "conversion before terminal is rejected"
    );
    r.terminal = Some(TerminalOutcome::NoOutput);
    r.conversion = Some(ConversionOutcome::Held {
        reason: "  ".into(),
    });
    assert!(r.validate().is_err(), "blank hold reason is rejected");
    r.conversion = Some(ConversionOutcome::Held { reason: "x".into() });
    assert!(
        r.validate().is_ok(),
        "a patch written before a budget kill still converts"
    );
}

#[test]
fn report_separates_two_models_with_differing_outcomes() {
    let mut records = Vec::new();
    for i in 0..12 {
        let mut r = record(&format!("a-{i}"), "q27-a", 2048);
        r.terminal = Some(TerminalOutcome::PatchProduced);
        r.conversion = Some(ConversionOutcome::Held {
            reason: "format".into(),
        });
        records.push(r);
    }
    for i in 0..20 {
        let mut r = record(&format!("b-{i}"), "qwen3.8-27b", 80_000);
        r.terminal = Some(TerminalOutcome::PatchProduced);
        r.conversion = if i < 10 {
            Some(ConversionOutcome::Converted)
        } else {
            Some(ConversionOutcome::RetiredNonApplying)
        };
        records.push(r);
    }

    let report = outcome_report(&records, 10);
    let a = report.by_model.iter().find(|r| r.model == "q27-a").unwrap();
    let b = report
        .by_model
        .iter()
        .find(|r| r.model == "qwen3.8-27b")
        .unwrap();
    assert_eq!(a.sample, 12);
    assert_eq!(a.status, RateStatus::Rate(0.0));
    assert_eq!(b.sample, 20);
    assert_eq!(b.converted, 10);
    assert_eq!(b.retired, 10);
    assert!(matches!(b.status, RateStatus::Rate(r) if (r - 0.5).abs() < 1e-9));

    // model x band: the two models landed in different bands.
    let band = |model: &str| {
        report
            .by_model_band
            .iter()
            .find(|r| r.model == model)
            .unwrap()
            .band
            .unwrap()
    };
    assert_eq!(band("q27-a"), SpecSizeBand::Small);
    assert_eq!(band("qwen3.8-27b"), SpecSizeBand::Large);
}

#[test]
fn small_sample_is_insufficient_data_never_a_rate() {
    let mut records = Vec::new();
    for i in 0..3 {
        let mut r = record(&format!("c-{i}"), "glm-4.6", 2_000_000);
        r.terminal = Some(TerminalOutcome::Timeout);
        r.conversion = Some(ConversionOutcome::Converted);
        records.push(r);
    }
    let report = outcome_report(&records, DEFAULT_MIN_SAMPLES);
    let row = &report.by_model[0];
    assert_eq!(row.sample, 3);
    assert_eq!(row.status, RateStatus::InsufficientData);
    let markdown = report.to_markdown();
    assert!(markdown.contains("insufficient data (n=3)"));
    assert!(
        !markdown.contains("%"),
        "no percentage may appear for a small sample"
    );
}

#[test]
fn min_samples_boundary_is_inclusive_and_pending_rows_do_not_rate() {
    let mut enough = Vec::new();
    for i in 0..10 {
        let mut r = record(&format!("m-{i}"), "m", 1);
        r.terminal = Some(TerminalOutcome::PatchProduced);
        r.conversion = Some(ConversionOutcome::Converted);
        enough.push(r);
    }
    assert!(matches!(
        outcome_report(&enough, 10).by_model[0].status,
        RateStatus::Rate(1.0)
    ));
    let mut short = enough.clone();
    short.pop();
    assert_eq!(
        outcome_report(&short, 10).by_model[0].status,
        RateStatus::InsufficientData
    );

    let pending = vec![record("p-1", "m", 1)];
    let row = &outcome_report(&pending, 1).by_model[0];
    assert_eq!(row.pending, 1);
    assert_eq!(row.sample, 0);
    assert_eq!(row.status, RateStatus::InsufficientData);
}

#[test]
fn empty_report_renders() {
    let report = outcome_report(&[], 10);
    assert!(report.to_markdown().contains("_no dispatch records_"));
}
