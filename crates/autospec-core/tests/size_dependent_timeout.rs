//! A fixed timeout on a size-dependent operation is a silent capability
//! filter, not a flaky check (issue #4402).
//!
//! The regression runs in the configuration the bug required: five models, the
//! two small ones registered and the three large ones never did, probed with a
//! constant timeout on a size-dependent read. Sizes are in tenths of a GB to
//! preserve the issue's figures (15.7, 16.6, 107, 149, 281 GB); only ordering
//! matters to the detector.

use autospec_core::size_dependent_timeout::{
    attribute_failure, attribute_from_inputs, audit_timeout, classify_capacity, detect_size_filter,
    Attribution, CapacityEvidence, CapacityStatus, FilterThreshold, SizeSensitivity, SizedInput,
    TimeoutSource, TimeoutVerdict,
};

/// The incident, in the order the catalogue lists it.
fn incident_models() -> Vec<SizedInput> {
    vec![
        SizedInput {
            name: "qwen3.8-27b".into(),
            size: 157, // 15.7 GB
            ever_succeeded: true,
        },
        SizedInput {
            name: "qwen3.8-27b-vision".into(),
            size: 166, // 16.6 GB
            ever_succeeded: true,
        },
        SizedInput {
            name: "qwen3.8-flash-next".into(),
            size: 1070, // 107 GB
            ever_succeeded: false,
        },
        SizedInput {
            name: "deepseek-v4-flash".into(),
            size: 1490, // 149 GB
            ever_succeeded: false,
        },
        SizedInput {
            name: "glm-5.3-flash".into(),
            size: 2810, // 281 GB
            ever_succeeded: false,
        },
    ]
}

fn model(name: &str, size: u64, ever_succeeded: bool) -> SizedInput {
    SizedInput {
        name: name.into(),
        size,
        ever_succeeded,
    }
}

// ---- Invariant 1: the timeout must be derived, not chosen ----

#[test]
fn constant_timeout_on_size_dependent_operation_is_a_filter() {
    // The worker's `curl --max-time 10` on a load whose duration scales with
    // model size.
    assert_eq!(
        audit_timeout(SizeSensitivity::ScalesWithInput, TimeoutSource::Constant),
        TimeoutVerdict::SilentCapabilityFilter
    );
}

#[test]
fn derived_timeouts_are_sound() {
    assert_eq!(
        audit_timeout(
            SizeSensitivity::ScalesWithInput,
            TimeoutSource::FromInputSize
        ),
        TimeoutVerdict::Sound
    );
    assert_eq!(
        audit_timeout(
            SizeSensitivity::ScalesWithInput,
            TimeoutSource::FromGoverningBound
        ),
        TimeoutVerdict::Sound
    );
}

#[test]
fn constant_is_defensible_only_when_input_independent() {
    // A constant is the one case that is not a filter: the duration genuinely
    // does not depend on the input.
    assert_eq!(
        audit_timeout(SizeSensitivity::InputIndependent, TimeoutSource::Constant),
        TimeoutVerdict::Sound
    );
}

#[test]
fn silent_filter_line_names_the_filter() {
    let line = TimeoutVerdict::SilentCapabilityFilter.line();
    assert!(line.contains("silent capability filter"));
    assert!(line.contains("excludes"));
}

// ---- Invariant 1, made measurable: the clean step ----

#[test]
fn incident_models_show_a_clean_size_threshold() {
    // The tell of the constant timeout: every model below ~30 GB registered,
    // every model above it never did.
    let threshold = detect_or_panic(&incident_models());
    assert_eq!(
        threshold,
        FilterThreshold {
            admits_through: 166,
            excludes_from: 1070
        }
    );
    // The boundary sits between the two admitted and the three excluded
    // models — the "clean threshold at roughly 30 GB".
    assert!(threshold.admits_through < 300);
    assert!(threshold.excludes_from > 300);
}

#[test]
fn interleaved_pattern_is_not_a_size_filter() {
    // A small model that fails beside a larger one that succeeds: the pattern
    // does not separate on size, so it correlates with something else.
    let inputs = vec![
        model("a", 10, false),
        model("b", 50, true),
        model("c", 90, false),
    ];
    assert_eq!(detect_size_filter(&inputs), None);
}

#[test]
fn all_success_or_all_failure_forms_no_boundary() {
    let all_ok = vec![model("a", 10, true), model("b", 90, true)];
    let all_bad = vec![model("a", 10, false), model("b", 90, false)];
    assert_eq!(detect_size_filter(&all_ok), None);
    assert_eq!(detect_size_filter(&all_bad), None);
}

// ---- Invariant 2: absent is not failing ----

#[test]
fn absent_and_failing_are_distinct_statuses() {
    // Never registered, not serving: no capacity at all.
    assert_eq!(
        classify_capacity(&CapacityEvidence {
            ever_registered: false,
            currently_healthy: false,
        }),
        CapacityStatus::Absent
    );
    // Registered but not serving: capacity that is unhealthy.
    assert_eq!(
        classify_capacity(&CapacityEvidence {
            ever_registered: true,
            currently_healthy: false,
        }),
        CapacityStatus::Failing
    );
    // Serving.
    assert_eq!(
        classify_capacity(&CapacityEvidence {
            ever_registered: true,
            currently_healthy: true,
        }),
        CapacityStatus::Healthy
    );
}

#[test]
fn absent_and_failing_require_different_responses() {
    // The incident fleet could not tell "we have no flash-next capacity" from
    // "flash-next is unhealthy"; those need different responses.
    let absent = CapacityStatus::Absent.response();
    let failing = CapacityStatus::Failing.response();
    assert_ne!(absent, failing);
    assert!(absent.contains("registration"));
    assert!(failing.contains("repair"));
}

#[test]
fn flash_next_is_absent_not_failing() {
    // flash-next has never registered: it is absent capacity, and the response
    // points at registration — not at repairing a worker that was never there.
    let status = classify_capacity(&CapacityEvidence {
        ever_registered: false,
        currently_healthy: false,
    });
    assert_eq!(status, CapacityStatus::Absent);
    assert!(status.response().contains("has ever registered"));
}

// ---- Invariant 3: suspect the harness before the subject ----

#[test]
fn correlated_failure_points_at_the_harness() {
    // The incident's pattern separates cleanly on size, so "flash-next keeps
    // failing" should first point at what the pipeline does differently for it
    // — which is nothing, except take longer.
    assert_eq!(
        attribute_from_inputs(&incident_models()),
        Attribution::Harness
    );
}

#[test]
fn uncorrelated_failure_points_at_the_subject() {
    // No clean step: the discriminator is not a property of the input, so the
    // subject is the suspect.
    let inputs = vec![
        model("a", 10, false),
        model("b", 50, true),
        model("c", 90, false),
    ];
    assert_eq!(attribute_from_inputs(&inputs), Attribution::Subject);
    assert_eq!(attribute_failure(false), Attribution::Subject);
    assert_eq!(attribute_failure(true), Attribution::Harness);
}

fn detect_or_panic(inputs: &[SizedInput]) -> FilterThreshold {
    detect_size_filter(inputs).expect("incident models must show a clean size threshold")
}
