#![cfg(unix)]

use autospec_core::autonomous::no_work::NoWorkTier;

#[path = "support/foreground_waterfall_fixture.rs"]
mod foreground_waterfall_fixture;

use foreground_waterfall_fixture::ForegroundWaterfallFixture;

#[test]
fn repeated_empty_foreground_cycles_reach_and_retain_failed_native_tier2() {
    let fixture = ForegroundWaterfallFixture::empty_repository();
    fixture.run_foreground_three_times().assert_success();

    assert_eq!(fixture.cursor(), NoWorkTier::Tier2);
    assert_eq!(fixture.receipt_status(NoWorkTier::Tier2), "failed");
    assert_eq!(fixture.executor_launches(), 1);
    assert!(!fixture.tier_directory_exists(NoWorkTier::Tier3));
    fixture.assert_no_forbidden_waterfall_side_effects(true);
}

#[test]
fn tier15_produced_retains_cursor_without_claim_or_executor() {
    let fixture = ForegroundWaterfallFixture::with_clear_tier15_candidate();
    fixture.run_until_tier15().assert_success();

    assert_eq!(fixture.cursor(), NoWorkTier::Tier1_5);
    assert_eq!(fixture.receipt_status(NoWorkTier::Tier1_5), "produced");
    assert_eq!(fixture.claim_mutations(), 0);
    assert_eq!(fixture.executor_launches(), 0);
    fixture.assert_no_forbidden_waterfall_side_effects(false);
}

#[test]
fn tier15_read_failure_is_sealed_and_never_dry() {
    let fixture = ForegroundWaterfallFixture::with_tier15_page_failure();
    fixture.run_until_tier15().assert_success();

    assert_eq!(fixture.cursor(), NoWorkTier::Tier1_5);
    assert_eq!(fixture.receipt_status(NoWorkTier::Tier1_5), "failed");
    assert!(!fixture.why_no_work_exists());
    fixture.assert_no_forbidden_waterfall_side_effects(false);
}

#[test]
fn retained_tier15_produced_replays_without_another_read_or_waterfall_mutation() {
    let fixture = ForegroundWaterfallFixture::with_clear_tier15_candidate();
    fixture.run_until_tier15().assert_success();
    let reads_before = fixture.tier15_reads();
    let waterfall_before = fixture.waterfall_snapshot();

    fixture.run_foreground_once().assert_success();

    assert_eq!(fixture.tier15_reads(), reads_before);
    assert_eq!(fixture.waterfall_snapshot(), waterfall_before);
    assert_eq!(fixture.receipt_status(NoWorkTier::Tier1_5), "produced");
}

#[test]
fn retained_tier15_failure_replays_without_another_read_or_waterfall_mutation() {
    let fixture = ForegroundWaterfallFixture::with_tier15_page_failure();
    fixture.run_until_tier15().assert_success();
    let reads_before = fixture.tier15_reads();
    let waterfall_before = fixture.waterfall_snapshot();

    fixture.run_foreground_once().assert_success();

    assert_eq!(fixture.tier15_reads(), reads_before);
    assert_eq!(fixture.waterfall_snapshot(), waterfall_before);
    assert_eq!(fixture.receipt_status(NoWorkTier::Tier1_5), "failed");
}

#[test]
fn newly_ready_work_preempts_a_retained_waterfall_cursor() {
    let fixture = ForegroundWaterfallFixture::retained_at_tier2_with_ready_issue();
    fixture.run_foreground_once().assert_success();

    assert_eq!(fixture.safety_reviews(), vec![42]);
    assert_eq!(fixture.tier2_record_attempts(), 0);
    assert_eq!(fixture.cursor(), NoWorkTier::Tier2);
}

#[test]
fn reached_worker_cap_does_not_resume_a_retained_waterfall_cursor() {
    let fixture = ForegroundWaterfallFixture::retained_at_tier2_with_reached_worker_cap();
    fixture.run_foreground_once().assert_success();

    assert_eq!(fixture.tier2_record_attempts(), 0);
    assert_eq!(fixture.cursor(), NoWorkTier::Tier2);
}
