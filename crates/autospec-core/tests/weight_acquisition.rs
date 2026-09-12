//! Regression tests for the verify-runtime-support-before-acquiring-weights
//! invariants (issue #4355).
//!
//! Incident configuration: DeepSeek V4.1 is released; the runtime (llama.cpp)
//! has exactly one reference to the `deepseek_v41` architecture — an *open*
//! conversion PR — so it cannot load it yet. The only policy-compliant quant
//! is a 508 GB Q8 from an unvetted publisher (the trusted publishers 404), and
//! the only other third-party quant is a Q2 below the fleet's Q6 floor. Two
//! attempts to read architecture support out of the shipped binary returned
//! zero — even for architectures the fleet serves today — so that method was
//! broken and its zeros meant nothing.
//!
//! Wrapped in `mod weight_acquisition` so the smoke command,
//! `cargo test -p autospec-core weight_acquisition`, selects this binary's
//! tests.

mod weight_acquisition {
    use autospec_core::weight_acquisition::{
        decide, AcquisitionDecision, AcquisitionGate, DeploymentState, Placement, Probe,
        ProbeValidity, Quant, QuantPolicy, RejectedQuant, RuntimeSupport, TrustedPublishers,
        GATE_ORDER,
    };

    fn quant(name: &str, level: u32, publisher: &str, weights_mib: u64) -> Quant {
        Quant {
            name: name.to_string(),
            level,
            publisher: publisher.to_string(),
            weights_mib,
        }
    }

    fn trusted(names: &[&str]) -> TrustedPublishers {
        TrustedPublishers::new(names.iter().map(|s| s.to_string()).collect())
    }

    fn policy() -> QuantPolicy {
        QuantPolicy::default()
    }

    /// A *validated* support answer (the source produced a positive on a
    /// known-good input), so its negatives are findings.
    fn supported(source: &str, reference: Option<String>) -> RuntimeSupport {
        RuntimeSupport {
            supported: true,
            source_valid: true,
            source: source.to_string(),
            reference,
        }
    }

    /// A *validated* "no support" from the upstream tracker (invariant 1 + 4).
    fn unsupported_validated(source: &str, reference: Option<String>) -> RuntimeSupport {
        RuntimeSupport {
            supported: false,
            source_valid: true,
            source: source.to_string(),
            reference,
        }
    }

    // ── Invariant 1: the runtime-support gate stops before the download ──

    #[test]
    fn incident_deepseek_v41_stops_at_runtime_support() {
        // The validated upstream tracker: one reference, an open PR. The
        // 508 GB quant must never be reached.
        let support = unsupported_validated(
            "upstream tracker",
            Some("llama.cpp convert: add DeepSeek V4.1 (DeepseekV41ForCausalLM)".to_string()),
        );
        let quants = vec![
            quant(
                "deepseek-v41-flash-q8",
                8,
                "unvetted-account",
                508 * 1024 * 1024,
            ),
            quant(
                "deepseek-v41-flash-q2",
                2,
                "unvetted-account",
                100 * 1024 * 1024,
            ),
        ];
        let decision = decide(
            &support,
            &quants,
            &policy(),
            &trusted(&["trusted-publisher-a", "trusted-publisher-b"]),
            &Placement {
                available_mib: 0,
                kv_mib: 0,
            },
        );

        assert_eq!(
            decision,
            AcquisitionDecision::RuntimeUnsupported {
                reference: Some(
                    "llama.cpp convert: add DeepSeek V4.1 (DeepseekV41ForCausalLM)".to_string()
                )
            }
        );
        assert_eq!(decision.gate(), AcquisitionGate::RuntimeSupport);
        assert!(!decision.acquires(), "the download is never reached");
        let line = decision.line();
        assert!(
            line.contains("DeepseekV41ForCausalLM") && line.contains("still open"),
            "the refusal names the open upstream PR: {line}"
        );
        assert!(
            line.contains("download is not reached"),
            "the refusal says the expensive step was not taken: {line}"
        );
    }

    // ── Invariant 1: the ordering itself — the download gate is last ──

    #[test]
    fn gate_order_has_download_last_and_only_it_is_expensive() {
        assert_eq!(GATE_ORDER.len(), 4);
        assert_eq!(
            GATE_ORDER[3],
            AcquisitionGate::Download,
            "the download is the last gate"
        );
        // GATE_ORDER is cheap-first: the download is the maximum.
        let mut ordered = GATE_ORDER.to_vec();
        let sorted = ordered.clone();
        ordered.sort();
        assert_eq!(sorted, GATE_ORDER.to_vec());
        for gate in GATE_ORDER.iter().copied() {
            assert_eq!(
                gate.is_expensive(),
                gate == AcquisitionGate::Download,
                "only the download gate is expensive: {gate:?}"
            );
        }
    }

    // ── Invariant 2: publisher trust is part of the model decision ──

    #[test]
    fn incident_quant_policy_when_runtime_supported() {
        // If the runtime *could* load it, the fleet's trusted publishers both
        // 404 and the only third-party quants are a Q2 (below floor) and a 508
        // GB Q8 from an unvetted account. Neither is admissible.
        let support = supported("upstream tracker", None);
        let quants = vec![
            quant(
                "deepseek-v41-flash-q2",
                2,
                "unvetted-account",
                100 * 1024 * 1024,
            ),
            quant(
                "deepseek-v41-flash-q8",
                8,
                "unvetted-account",
                508 * 1024 * 1024,
            ),
        ];
        let trusted = trusted(&["trusted-publisher-a", "trusted-publisher-b"]);
        let decision = decide(
            &support,
            &quants,
            &policy(),
            &trusted,
            &Placement {
                available_mib: u64::MAX,
                kv_mib: 0,
            },
        );

        assert_eq!(decision.gate(), AcquisitionGate::QuantPolicy);
        assert!(!decision.acquires());
        let AcquisitionDecision::NoTrustedQuant { rejected } = &decision else {
            panic!("expected NoTrustedQuant, got {}", decision.line());
        };
        assert_eq!(rejected.len(), 2);
        // The Q2 fails BOTH: below the floor and an untrusted publisher.
        let q2 = rejected
            .iter()
            .find(|q| q.name == "deepseek-v41-flash-q2")
            .unwrap();
        assert_eq!(
            q2,
            &RejectedQuant {
                name: "deepseek-v41-flash-q2".to_string(),
                reasons: vec![
                    autospec_core::weight_acquisition::QuantRejection::BelowFloor {
                        level: 2,
                        floor: 6,
                    },
                    autospec_core::weight_acquisition::QuantRejection::UntrustedPublisher {
                        publisher: "unvetted-account".to_string(),
                    },
                ],
            },
            "the Q2 is rejected for both being below the floor and an untrusted publisher"
        );
        // The Q8 clears the floor but is still rejected for the publisher.
        let q8 = rejected
            .iter()
            .find(|q| q.name == "deepseek-v41-flash-q8")
            .unwrap();
        assert_eq!(q8.reasons.len(), 1);
        assert!(
            q8.line().contains("untrusted publisher `unvetted-account`"),
            "{}",
            q8.line()
        );
    }

    #[test]
    fn a_trusted_quant_at_or_above_the_floor_is_admissible() {
        let q = quant("model-q8", 8, "trusted-publisher-a", 500_000);
        assert!(
            q.admissible(&policy(), &trusted(&["trusted-publisher-a"])),
            "a trusted Q8 at the Q6 floor is admissible"
        );
        let q6 = quant("model-q6", 6, "trusted-publisher-a", 400_000);
        assert!(q6.admissible(&policy(), &trusted(&["trusted-publisher-a"])));
        // Q5 is just below the floor.
        let q5 = quant("model-q5", 5, "trusted-publisher-a", 400_000);
        assert!(!q5.admissible(&policy(), &trusted(&["trusted-publisher-a"])));
    }

    // ── Gate 3 + gate 4: placement room, then the happy-path download ──

    #[test]
    fn control_happy_path_acquires_the_smallest_admissible_quant() {
        let support = supported("upstream tracker", None);
        let trusted = trusted(&["trusted-publisher-a"]);
        let quants = vec![
            quant("model-q8", 8, "trusted-publisher-a", 500_000),
            quant("model-q6", 6, "trusted-publisher-a", 400_000),
        ];
        let placement = Placement {
            available_mib: 500_000,
            kv_mib: 40_000,
        };
        let decision = decide(&support, &quants, &policy(), &trusted, &placement);
        // The smallest admissible quant (Q6) is chosen, not the Q8.
        assert_eq!(
            decision,
            AcquisitionDecision::Acquire {
                quant: "model-q6".to_string(),
                weights_mib: 400_000,
            }
        );
        assert_eq!(decision.gate(), AcquisitionGate::Download);
        assert!(decision.acquires());
        assert!(decision.line().contains("acquire authorised"));
    }

    #[test]
    fn gate3_placement_refuses_when_no_room_for_weights_plus_kv() {
        let support = supported("upstream tracker", None);
        let trusted = trusted(&["trusted-publisher-a"]);
        let quants = vec![quant("model-q8", 8, "trusted-publisher-a", 500_000)];
        // 500_000 (weights) + 40_000 (KV) = 540_000 > 520_000 available.
        let placement = Placement {
            available_mib: 520_000,
            kv_mib: 40_000,
        };
        let decision = decide(&support, &quants, &policy(), &trusted, &placement);
        assert_eq!(
            decision,
            AcquisitionDecision::NoPlacementRoom {
                quant: "model-q8".to_string(),
                needed_mib: 540_000,
                available_mib: 520_000,
            }
        );
        assert_eq!(decision.gate(), AcquisitionGate::PlacementRoom);
        assert!(!decision.acquires());
        let line = decision.line();
        assert!(line.contains("540000") && line.contains("520000"), "{line}");
    }

    // ── Invariant 3: "acquired but unservable" is a distinct state ──

    #[test]
    fn incident_glm_is_acquired_but_unservable() {
        assert_ne!(
            DeploymentState::AcquiredUnservable,
            DeploymentState::Servable
        );
        assert_ne!(
            DeploymentState::AcquiredUnservable,
            DeploymentState::NotAcquired
        );
        assert!(DeploymentState::AcquiredUnservable.parked());
        assert!(!DeploymentState::Servable.parked());
        assert!(!DeploymentState::NotAcquired.parked());
        assert!(!DeploymentState::Acquiring.parked());
        let line = DeploymentState::AcquiredUnservable.line();
        assert!(
            line.contains("acquired but unservable") && line.contains("capital spent and parked"),
            "{line}"
        );
    }

    // ── Invariant 4: a negative from a tool that cannot produce a positive ──

    #[test]
    fn incident_binary_inspection_negative_is_not_a_finding() {
        // The binary-inspection method returned zero even for architectures
        // the fleet serves today (known-good controls) — it is broken.
        let mut binary = Probe::new("binary inspection");
        binary.record_control("llama", false);
        binary.record_control("qwen3.8-27b", false);
        assert_eq!(
            binary.validity(),
            &ProbeValidity::Broken {
                failed_control: "llama".to_string()
            }
        );
        let verdict = binary.negative_verdict();
        assert!(
            !verdict.is_finding(),
            "a broken probe's zero is not a finding"
        );
        assert!(
            verdict
                .line()
                .contains("failed the known-good input `llama`"),
            "{}",
            verdict.line()
        );

        // Contrast: the upstream tracker produced a positive on a known-good
        // architecture, so its negative for a new one is a real finding.
        let mut tracker = Probe::new("upstream tracker");
        tracker.record_control("llama", true);
        assert_eq!(tracker.validity(), &ProbeValidity::Validated);
        assert!(tracker.negative_verdict().is_finding());
    }

    #[test]
    fn unvalidated_probe_negative_is_not_a_finding() {
        // A probe that has never been shown to produce a positive is
        // unvalidated; its negative is not a finding yet.
        let probe = Probe::new("fresh probe");
        assert_eq!(probe.validity(), &ProbeValidity::Unvalidated);
        let verdict = probe.negative_verdict();
        assert!(!verdict.is_finding());
        assert!(
            verdict.line().contains("has never produced a positive"),
            "{}",
            verdict.line()
        );
    }

    #[test]
    fn a_validated_negative_stops_acquisition_but_an_unverified_one_is_inconclusive() {
        // The same "no support" from two sources: a validated upstream tracker
        // is a stop; an unvalidated binary inspection is inconclusive, not a
        // stop and not a download.
        let quants = vec![quant("model-q8", 8, "trusted-publisher-a", 500_000)];
        let trusted = trusted(&["trusted-publisher-a"]);
        let placement = Placement {
            available_mib: 1_000_000,
            kv_mib: 0,
        };

        let validated =
            unsupported_validated("upstream tracker", Some("llama.cpp#1234".to_string()));
        let stopped = decide(&validated, &quants, &policy(), &trusted, &placement);
        assert_eq!(stopped.gate(), AcquisitionGate::RuntimeSupport);
        assert!(!stopped.acquires());

        let unverified = RuntimeSupport {
            supported: false,
            source_valid: false,
            source: "binary inspection".to_string(),
            reference: None,
        };
        let inconclusive = decide(&unverified, &quants, &policy(), &trusted, &placement);
        assert_eq!(
            inconclusive,
            AcquisitionDecision::SupportUnverified {
                source: "binary inspection".to_string()
            }
        );
        assert_eq!(inconclusive.gate(), AcquisitionGate::RuntimeSupport);
        assert!(
            !inconclusive.acquires(),
            "an unverified negative does not authorise a download"
        );
        let line = inconclusive.line();
        assert!(
            line.contains("not a finding") && line.contains("binary inspection"),
            "{line}"
        );
    }

    // ── Regression: decide never reaches the download before the cheap gates ──

    #[test]
    fn decide_never_reaches_download_before_cheap_gates_pass() {
        let trusted = trusted(&["trusted-publisher-a"]);
        let placement = Placement {
            available_mib: 1_000_000,
            kv_mib: 0,
        };
        let quants = vec![quant("model-q8", 8, "trusted-publisher-a", 500_000)];

        // Every refusal stops at a cheap gate strictly before the download.
        for support in [
            unsupported_validated("upstream tracker", None),
            RuntimeSupport {
                supported: false,
                source_valid: false,
                source: "binary inspection".to_string(),
                reference: None,
            },
        ] {
            let decision = decide(&support, &quants, &policy(), &trusted, &placement);
            assert!(
                decision.gate() < AcquisitionGate::Download,
                "a gate-1 refusal stops before the download: {:?}",
                decision
            );
        }

        // Gate 2 refusal.
        let decision = decide(
            &supported("upstream tracker", None),
            &[quant("model-q2", 2, "stranger", 100_000)],
            &policy(),
            &trusted,
            &placement,
        );
        assert!(
            decision.gate() < AcquisitionGate::Download,
            "a gate-2 refusal stops before the download: {:?}",
            decision
        );

        // Only the authorised download reaches the download gate.
        let acquire = decide(
            &supported("upstream tracker", None),
            &quants,
            &policy(),
            &trusted,
            &placement,
        );
        assert_eq!(acquire.gate(), AcquisitionGate::Download);
        assert!(acquire.acquires());
    }
}
