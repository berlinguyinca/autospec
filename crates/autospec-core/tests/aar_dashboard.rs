//! AAR M12 (issue #3327): live performance card and benchmark-derived
//! routing advice. Every acceptance criterion gets deterministic coverage:
//! the card exposes at least 10 execution fields, the historical summary
//! reports p50/p90/p95, fewer than 20 samples preserve static selection, and
//! advice never moves the model family away from the locked one.

use autospec_core::aar::dashboard::{
    advise, elapsed_ratio, percentile_nearest_rank, summarize_history, AdviceConfig, AdviceSource,
    IssueSample, LiveWorkItem, Liveness, ProfileSample, DEFAULT_LIVENESS_THRESHOLD_MS,
    DEFAULT_LOCKED_MODEL_FAMILY, DEFAULT_MIN_SAMPLES, LIVE_CARD_FIELDS, MIN_SUCCESS_RATE,
};

fn live() -> LiveWorkItem {
    LiveWorkItem {
        issue_id: "3327".to_string(),
        state: "running".to_string(),
        node_id: "node-1".to_string(),
        profile: "qwen3.8-27b-q4/rtx4090".to_string(),
        turns: 12,
        context_tokens: 41_200,
        ttft_ms: 850,
        decode_tokens_per_second: 34.5,
        cache_hit_rate: 0.83,
        tool_ms: 95_400,
        test_ms: 61_200,
        repair_count: 1,
        queue_ms: 2_400,
        started_ms: 1_700_000_000_000,
        last_heartbeat_ms: 1_700_000_300_000,
    }
}

fn candidate(
    family: &str,
    key: &str,
    samples: u32,
    successes: u32,
    cost: u64,
    wall: u64,
) -> ProfileSample {
    ProfileSample {
        model_family: family.to_string(),
        profile_key: key.to_string(),
        samples,
        successes,
        mean_cost_micros: cost,
        mean_wall_ms: wall,
    }
}

/// AC1: the live card exposes at least 10 required execution fields.
#[test]
fn live_card_exposes_at_least_ten_required_execution_fields() {
    assert!(
        LIVE_CARD_FIELDS.len() >= 10,
        "card contract must name at least 10 execution fields"
    );
    let rendered = live().render();
    for field in LIVE_CARD_FIELDS {
        assert!(
            rendered.contains(&format!("{field}: ")),
            "live card is missing field {field}\n{rendered}"
        );
    }
    assert!(rendered.contains("issue_id: 3327"));
}

#[test]
fn live_card_rejects_out_of_range_cache_hit_rate() {
    let mut item = live();
    item.cache_hit_rate = 1.2;
    assert!(item.validate().is_err());
    item.cache_hit_rate = -0.1;
    assert!(item.validate().is_err());
    item.cache_hit_rate = 0.83;
    assert!(item.validate().is_ok());
}

/// AC2: the historical summary reports p50, p90 and p95 issue duration.
#[test]
fn history_reports_p50_p90_and_p95_issue_duration() {
    let samples: Vec<IssueSample> = (1..=100)
        .map(|i| IssueSample {
            issue_id: format!("issue-{i}"),
            duration_ms: i * 1000,
            succeeded: i % 2 == 0,
            time_to_passing_change_ms: Some(i * 500),
        })
        .collect();
    let history = summarize_history(&samples, 24.0);
    assert_eq!(history.samples, 100);
    assert_eq!(history.p50_ms, 50_000);
    assert_eq!(history.p90_ms, 90_000);
    assert_eq!(history.p95_ms, 95_000);
    // mean of 1000..=100000 step 1000 is 50_500
    assert_eq!(history.mean_duration_ms, 50_500);
}

#[test]
fn history_reports_success_throughput_and_median_time_to_passing() {
    let samples = vec![
        IssueSample {
            issue_id: "a".to_string(),
            duration_ms: 90_000,
            succeeded: true,
            time_to_passing_change_ms: Some(1_000),
        },
        IssueSample {
            issue_id: "b".to_string(),
            duration_ms: 60_000,
            succeeded: true,
            time_to_passing_change_ms: Some(2_000),
        },
        IssueSample {
            issue_id: "c".to_string(),
            duration_ms: 120_000,
            succeeded: true,
            time_to_passing_change_ms: Some(3_000),
        },
        IssueSample {
            issue_id: "d".to_string(),
            duration_ms: 30_000,
            succeeded: true,
            time_to_passing_change_ms: Some(4_000),
        },
        IssueSample {
            issue_id: "e".to_string(),
            duration_ms: 150_000,
            succeeded: false,
            time_to_passing_change_ms: None,
        },
    ];
    let history = summarize_history(&samples, 8.0);
    assert!((history.successful_issues_per_hour - 0.5).abs() < f64::EPSILON);
    // nearest-rank p50 of [1000, 2000, 3000, 4000] is the 2nd value
    assert_eq!(history.median_time_to_passing_ms, Some(2_000));
}

#[test]
fn percentile_nearest_rank_is_exact() {
    let values: Vec<u64> = (1..=10).collect();
    assert_eq!(percentile_nearest_rank(&values, 50), Some(5));
    assert_eq!(percentile_nearest_rank(&values, 90), Some(9));
    assert_eq!(percentile_nearest_rank(&values, 95), Some(10));
    assert_eq!(percentile_nearest_rank(&[], 50), None);
    assert_eq!(percentile_nearest_rank(&values, 101), None);
}

/// AC3: fewer than the configured 20 samples preserve static selection.
#[test]
fn below_min_samples_preserves_static_profile_selection() {
    let config = AdviceConfig::default();
    assert_eq!(config.min_samples, DEFAULT_MIN_SAMPLES);
    assert_eq!(config.min_samples, 20);
    assert_eq!(config.locked_model_family, "qwen3.8");

    let candidates = vec![candidate(
        "qwen3.8",
        "qwen3.8-27b-bf16/dual-turing",
        19,
        19,
        0,
        10_000,
    )];
    let advice = advise(&candidates, &config);
    assert_eq!(advice.source, AdviceSource::Static);
    assert_eq!(advice.profile, config.static_profile);
    assert_eq!(advice.model_family, "qwen3.8");
    assert!(
        advice
            .rationale
            .iter()
            .any(|line| line.contains("19 samples below configured 20")),
        "rationale must show the sample shortfall\n{}",
        advice.rationale.join("\n")
    );
}

#[test]
fn sufficient_samples_enable_benchmark_selection() {
    let config = AdviceConfig::default();
    let candidates = vec![
        candidate(
            "qwen3.8",
            "qwen3.8-27b-bf16/dual-turing",
            24,
            24,
            1_000,
            90_000,
        ),
        candidate("qwen3.8", "qwen3.8-27b-q4/rtx4090", 24, 24, 0, 60_000),
    ];
    let advice = advise(&candidates, &config);
    assert_eq!(advice.source, AdviceSource::Benchmark);
    assert_eq!(advice.profile, "qwen3.8-27b-q4/rtx4090");
    assert_eq!(advice.model_family, "qwen3.8");
}

#[test]
fn empty_candidates_keep_the_static_profile() {
    let config = AdviceConfig::default();
    let advice = advise(&[], &config);
    assert_eq!(advice.source, AdviceSource::Static);
    assert_eq!(advice.profile, config.static_profile);
    assert_eq!(advice.model_family, DEFAULT_LOCKED_MODEL_FAMILY);
}

#[test]
fn low_success_candidates_fail_the_wilson_bar() {
    let config = AdviceConfig::default();
    let candidates = vec![candidate(
        "qwen3.8",
        "qwen3.8-27b-q4/rtx4090",
        24,
        22,
        0,
        60_000,
    )];
    assert!(
        candidates[0].success_lower_bound() < MIN_SUCCESS_RATE,
        "22-of-24 must sit below the Wilson bar"
    );
    let advice = advise(&candidates, &config);
    assert_eq!(advice.source, AdviceSource::Static);
    assert!(
        advice
            .rationale
            .iter()
            .any(|line| line.contains("success lower bound")),
        "rationale must show the Wilson rejection\n{}",
        advice.rationale.join("\n")
    );
}

/// AC4: advice never changes model_family away from the locked one.
#[test]
fn advice_never_changes_model_family_away_from_the_locked_one() {
    let config = AdviceConfig::default();

    // A different family has more samples and lower cost: still excluded,
    // and the in-family profile is chosen instead.
    let candidates = vec![
        candidate("claude", "claude-sonnet-5/provider-api", 30, 30, 0, 10_000),
        candidate("qwen3.8", "qwen3.8-27b-q4/rtx4090", 24, 24, 0, 60_000),
    ];
    let advice = advise(&candidates, &config);
    assert_eq!(advice.source, AdviceSource::Benchmark);
    assert_eq!(advice.model_family, "qwen3.8");
    assert!(
        advice.profile.starts_with("qwen3.8-"),
        "selected profile left the locked family: {}",
        advice.profile
    );
    assert!(
        advice
            .rationale
            .iter()
            .any(|line| line.contains("is not the locked family qwen3.8")),
        "rationale must show the family exclusion\n{}",
        advice.rationale.join("\n")
    );

    // With no in-family candidate at all, the family still does not move.
    let advice = advise(&candidates[..1], &config);
    assert_eq!(advice.source, AdviceSource::Static);
    assert_eq!(advice.model_family, "qwen3.8");
}

/// Issue #3723: the live card records the run's start and its last heartbeat
/// timestamp, so liveness comes from the run's own artifacts.
#[test]
fn live_card_records_start_and_last_heartbeat() {
    let rendered = live().render();
    assert!(
        rendered.contains("started_ms: 1700000000000"),
        "live card is missing started_ms\n{rendered}"
    );
    assert!(
        rendered.contains("last_heartbeat_ms: 1700000300000"),
        "live card is missing last_heartbeat_ms\n{rendered}"
    );
    assert!(LIVE_CARD_FIELDS.contains(&"started_ms"));
    assert!(LIVE_CARD_FIELDS.contains(&"last_heartbeat_ms"));
}

/// Issue #3723 item 4: liveness answers in one line — `progressing` or
/// `no output for N minutes` — from the last heartbeat versus now.
#[test]
fn liveness_answers_in_one_line() {
    let heartbeat = 1_700_000_000_000_u64;
    let threshold = DEFAULT_LIVENESS_THRESHOLD_MS;
    assert_eq!(threshold, 5 * 60 * 1000);

    // A fresh heartbeat is progress; one exactly at the edge still is.
    let fresh = Liveness::assess(heartbeat + threshold - 1, heartbeat, threshold);
    assert_eq!(fresh, Liveness::Progressing);
    assert_eq!(fresh.render(), "progressing");
    assert_eq!(
        Liveness::assess(heartbeat + threshold, heartbeat, threshold),
        Liveness::Progressing
    );

    // 381 quiet minutes — the shape of the 6-hour keystone run from the issue.
    let stalled = Liveness::assess(heartbeat + 381 * 60_000 + 30_000, heartbeat, threshold);
    assert_eq!(stalled, Liveness::NoOutput { minutes: 381 });
    assert_eq!(stalled.render(), "no output for 381 minutes");

    // A heartbeat ahead of now can only count as progress.
    assert_eq!(
        Liveness::assess(heartbeat - 1, heartbeat + 100, threshold),
        Liveness::Progressing
    );
}

/// Issue #3723 item 3: the history summary carries the mean completed-issue
/// duration a live run can be reported against.
#[test]
fn history_reports_mean_completed_issue_duration() {
    let samples = vec![
        IssueSample {
            issue_id: "a".to_string(),
            duration_ms: 60_000,
            succeeded: true,
            time_to_passing_change_ms: None,
        },
        IssueSample {
            issue_id: "b".to_string(),
            duration_ms: 90_000,
            succeeded: true,
            time_to_passing_change_ms: None,
        },
        IssueSample {
            issue_id: "c".to_string(),
            duration_ms: 120_000,
            succeeded: false,
            time_to_passing_change_ms: None,
        },
        IssueSample {
            issue_id: "d".to_string(),
            duration_ms: 30_000,
            succeeded: true,
            time_to_passing_change_ms: None,
        },
    ];
    let history = summarize_history(&samples, 24.0);
    assert_eq!(history.mean_duration_ms, 75_000);

    let empty = summarize_history(&[], 24.0);
    assert_eq!(empty.mean_duration_ms, 0);
}

/// Issue #3723 item 3: elapsed-versus-expected is the multiplier the
/// supervisor sees at a glance (6.8x) instead of computing it.
#[test]
fn elapsed_ratio_is_the_multiplier_against_the_mean() {
    assert!((elapsed_ratio(20_400_000, 3_000_000) - 6.8).abs() < f64::EPSILON);
    assert_eq!(elapsed_ratio(10_000, 0), 0.0);
    assert_eq!(elapsed_ratio(0, 1_000), 0.0);
}
