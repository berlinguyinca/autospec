//! A priority was attached to an unmeasured number (issue #4561).
//!
//! #4560 claimed one conflict shape was "the dominant conflict in this
//! backlog" and "the highest-yield single change available to
//! conversion throughput" — from 5 of 15 region instances, a file
//! contention count, and a 15-region sample. Measured properly (each
//! patch applied to a scratch worktree, both sides of every conflict
//! region classified), the shape was worth 11% of conflict-held
//! patches and the dominant shape was 76% `code × code`.
//!
//! The regression tests run in the configuration the error required:
//! a per-part share promoted to a per-item yield, a small sample, a
//! priority attached — and then the direct measurement that corrects
//! it.

use autospec_core::ranking_evidence::{
    any_of_gap, any_of_item_yield, judge_claim, unknown_ordering_line, Backlog, ClaimVerdict,
    EvidenceKind, Metric, RankingClaim, RankingTarget, Share, ShareStatus, MIN_MEASURED_N,
};

/// The incident, at scale: 57 held patches, 261 conflict regions.
///
/// - 6 patches are pure `additive` — a classifier for `additive`
///   frees exactly these 6 (11% of patches).
/// - 46 patches are `code × code` carrying one `additive` region each
///   — the shape appears in them, and frees none of them.
/// - 5 patches are pure `code`.
///
/// Per-part, `additive` is ~20% of regions; per-item, it is 11% of
/// patches. The region-instance share was the number #4560 filed.
fn incident_backlog() -> Backlog {
    let mut items = Vec::new();
    for _ in 0..6 {
        items.push(vec!["additive".to_string()]);
    }
    for _ in 0..46 {
        let mut parts = vec!["code".to_string(); 4];
        parts.push("additive".to_string());
        items.push(parts);
    }
    for _ in 0..5 {
        items.push(vec!["code".to_string(); 5]);
    }
    Backlog::new(items)
}

#[test]
fn incident_per_part_share_is_not_per_item_yield() {
    let backlog = incident_backlog();
    assert_eq!(backlog.item_count(), 57);
    assert_eq!(backlog.part_count(), 261);

    // The number that was filed: region-instance share.
    let per_part = backlog
        .per_part_share("additive")
        .expect("additive appears");
    assert_eq!(per_part, Share::new(52, 261).expect("well-formed"));
    assert!(per_part.fraction() > 0.15 && per_part.fraction() < 0.30);

    // The number the decision needs: patches whose regions are all
    // resolvable.
    let per_item = backlog
        .per_item_yield(&["additive"])
        .expect("non-empty backlog");
    assert_eq!(per_item, Share::new(6, 57).expect("well-formed"));

    // The shape appears in many patches alongside other shapes and
    // frees none of them: per-part overstates per-item.
    let comparison = backlog
        .yield_comparison("additive", &["additive"])
        .expect("both denominators non-empty");
    assert!(comparison.gap_points() > 0.0);
    assert!(
        comparison.per_part.fraction() > comparison.per_item.fraction() * 1.5,
        "region-instance share must not read as per-patch yield: {:?}",
        comparison.line()
    );

    // The dominant shape is the one no classifier resolves.
    let code = backlog.per_part_share("code").expect("code appears");
    assert!(code.fraction() > 0.70, "dominant shape: {}", code.line());
    assert_eq!(
        backlog.per_item_yield(&["code"]),
        Some(Share::new(5, 57).expect("well-formed")),
        "resolving code alone frees only the pure-code patches"
    );
}

#[test]
fn any_of_blocks_all_of_and_grows_with_parts() {
    // An item is freed only when ALL parts are resolvable.
    assert!((any_of_item_yield(0.5, 1) - 0.5).abs() < 1e-12);
    assert!((any_of_item_yield(0.5, 3) - 0.125).abs() < 1e-12);
    assert_eq!(any_of_item_yield(1.0, 10), 1.0);
    assert_eq!(any_of_item_yield(0.0, 3), 0.0);

    // The gap between per-part frequency and per-item yield is
    // non-decreasing in the number of parts, for 0 < p < 1.
    for &success in &[0.2f64, 0.5, 0.8, 0.95] {
        let mut prev = any_of_gap(success, 1);
        assert!(prev.abs() < 1e-12, "gap at 1 part is zero: {success}");
        for parts in 2..=20 {
            let gap = any_of_gap(success, parts);
            assert!(gap >= prev - 1e-12, "gap must grow with parts: {success}");
            assert!(gap > 0.0, "positive from 2 parts up: {success}");
            prev = gap;
        }
    }
}

#[test]
fn share_carries_its_denominator_and_status() {
    assert_eq!(Share::new(0, 0), None, "no denominator, no share");
    assert_eq!(
        Share::new(16, 15),
        None,
        "numerator cannot exceed population"
    );
    assert_eq!(
        Share::new(0, 15),
        Some(Share {
            numerator: 0,
            denominator: 15
        })
    );

    let small = Share::new(5, 15).expect("well-formed");
    assert_eq!(small.status(), ShareStatus::Hypothesis);
    assert_eq!(small.line(), "5 of 15 (33%, n=15, hypothesis)");

    let measured = Share::new(6, 57).expect("well-formed");
    assert_eq!(measured.status(), ShareStatus::Measured);
    assert_eq!(measured.line(), "6 of 57 (11%, n=57, measured)");

    let boundary = Share::new(1, MIN_MEASURED_N).expect("well-formed");
    assert_eq!(boundary.status(), ShareStatus::Measured);
    assert_eq!(
        Share::new(1, MIN_MEASURED_N - 1)
            .expect("well-formed")
            .status(),
        ShareStatus::Hypothesis
    );
}

#[test]
fn filed_issue_is_metric_mismatch() {
    // #4560 as filed: a per-part number ranking a per-item decision,
    // with a priority attached. The metric check fires first — a
    // priority cannot launder the wrong quantity.
    let claim = RankingClaim {
        subject: "additive_declarations".to_string(),
        target: RankingTarget::FreeItems,
        metric: Metric::PerPartFrequency,
        share: Share::new(5, 15).expect("well-formed"),
        evidence: EvidenceKind::Direct,
        priority: true,
    };
    let verdict = judge_claim(&claim);
    assert_eq!(verdict, ClaimVerdict::MetricMismatch);
    assert!(verdict.line(&claim).contains("per-item yield"));

    // The same mistake without a priority is still a mismatch: a
    // recommendation is a decision.
    let unprioritized = RankingClaim {
        priority: false,
        ..claim.clone()
    };
    assert_eq!(judge_claim(&unprioritized), ClaimVerdict::MetricMismatch);
}

#[test]
fn priority_on_small_sample_is_a_hypothesis() {
    // Right metric, but n=15: "dominant" from 15 observations is a
    // hypothesis, and a priority cannot ride on it.
    let claim = RankingClaim {
        subject: "additive_declarations".to_string(),
        target: RankingTarget::FreeItems,
        metric: Metric::PerItemYield,
        share: Share::new(5, 15).expect("well-formed"),
        evidence: EvidenceKind::Direct,
        priority: true,
    };
    assert_eq!(judge_claim(&claim), ClaimVerdict::SmallSample { n: 15 });

    // Without a priority the same number is admissible: it is stated
    // with n and its status, and no work is being ordered by it.
    let report = RankingClaim {
        target: RankingTarget::Describe,
        priority: false,
        ..claim.clone()
    };
    assert_eq!(judge_claim(&report), ClaimVerdict::Admissible);
}

#[test]
fn priority_on_indirect_evidence_is_refused() {
    // Right metric and a full denominator, but the number was
    // reasoned from file contention rather than measured: a priority
    // will not ride on it.
    let claim = RankingClaim {
        subject: "additive_declarations".to_string(),
        target: RankingTarget::FreeItems,
        metric: Metric::PerItemYield,
        share: Share::new(20, 57).expect("well-formed"),
        evidence: EvidenceKind::Indirect,
        priority: true,
    };
    assert_eq!(judge_claim(&claim), ClaimVerdict::IndirectEvidence);

    // The same number, measured by the scan, stands.
    let measured = RankingClaim {
        evidence: EvidenceKind::Direct,
        ..claim.clone()
    };
    assert_eq!(judge_claim(&measured), ClaimVerdict::Admissible);
}

#[test]
fn corrected_claim_is_admissible() {
    // The corrected #4560: the scan's per-item yield, n=57, direct.
    let claim = RankingClaim {
        subject: "additive_declarations".to_string(),
        target: RankingTarget::FreeItems,
        metric: Metric::PerItemYield,
        share: Share::new(6, 57).expect("well-formed"),
        evidence: EvidenceKind::Direct,
        priority: true,
    };
    let verdict = judge_claim(&claim);
    assert_eq!(verdict, ClaimVerdict::Admissible);
    assert!(verdict.line(&claim).contains("6 of 57"));
}

#[test]
fn unmeasurable_ranking_is_reported_as_unknown() {
    // An agent that cannot measure the ranking reports the ordering
    // as unknown, with what is known — never infers it from the
    // counts nearest to hand.
    let line = unknown_ordering_line(
        "additive_declarations",
        Share::new(5, 15).expect("well-formed"),
    );
    assert!(line.starts_with("ordering unknown:"));
    assert!(line.contains("5 of 15 (33%, n=15, hypothesis)"));
    assert!(line.contains("no priority attached"));
}

#[test]
fn empty_backlog_has_no_denominator() {
    let backlog = Backlog::new(Vec::new());
    assert_eq!(backlog.per_part_share("additive"), None);
    assert_eq!(backlog.per_item_yield(&["additive"]), None);
    assert_eq!(backlog.yield_comparison("additive", &["additive"]), None);

    // An item with no parts is held by nothing: never counted as
    // freed.
    let partsless = Backlog::new(vec![Vec::new()]);
    assert_eq!(
        partsless.per_item_yield(&["additive"]),
        Some(Share::new(0, 1).expect("well-formed"))
    );
}
