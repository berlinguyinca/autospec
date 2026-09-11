//! Tests for [`autospec_core::fleet_models`] (issue #3664): the two-registry
//! name bridge, the loud lookup-miss distinction, the startup refusal line,
//! the derived per-card fit check (issue #4244), and the scale ratchet that
//! may not ratchet against dying workers.
//!
//! Wrapped in `mod fleet_models` so the issue's smoke command,
//! `cargo test -p autospec-core fleet_models`, selects this binary's tests.

mod fleet_models {
    use autospec_core::fleet_models::{
        AuditFinding, FitCheck, FleetModelEntry, FleetRegistry, GpuCard, Registries, ResolveError,
        ScaleAction, ScaleRatchet, ServingRegistry, StartupRejection,
    };

    fn entry(canonical: &str, quant: &str) -> FleetModelEntry {
        FleetModelEntry {
            canonical: canonical.to_string(),
            quant: quant.to_string(),
            vram_mib: 40_960,
        }
    }

    fn registries(serving: &[&str]) -> Registries {
        Registries {
            fleet: FleetRegistry::new(vec![entry("qwen3.8-27b", "q8"), entry("deepseek-r1", "q4")])
                .unwrap(),
            serving: ServingRegistry::new(serving.iter().map(|s| s.to_string()).collect()).unwrap(),
        }
    }

    fn card(class: &str, vram_mib: u64) -> GpuCard {
        GpuCard {
            class: class.to_string(),
            vram_mib,
        }
    }

    fn fleet() -> FleetRegistry {
        // `entry()` pins vram_mib at 40_960: the model needs exactly 40 GiB.
        FleetRegistry::new(vec![entry("qwen3.8-27b", "q8")]).unwrap()
    }

    #[test]
    fn serving_name_is_derived_from_quant_column() {
        assert_eq!(entry("qwen3.8-27b", "q8").serving_name(), "qwen3.8-27b-q8");
        assert_eq!(entry("deepseek-r1", "q4").serving_name(), "deepseek-r1-q4");
    }

    #[test]
    fn tsv_parse_reads_quant_column() {
        let reg = FleetRegistry::parse_tsv(
            "# fleet models\n\
             qwen3.8-27b\tq8\t40960\n\
             \n\
             deepseek-r1\tq4\t28672\n",
        )
        .unwrap();
        assert_eq!(reg.len(), 2);
        assert_eq!(
            reg.get("qwen3.8-27b").unwrap().serving_name(),
            "qwen3.8-27b-q8"
        );
        assert_eq!(reg.get("deepseek-r1").unwrap().vram_mib, 28_672);
    }

    #[test]
    fn tsv_parse_rejects_malformed_rows() {
        assert!(FleetRegistry::parse_tsv("qwen3.8-27b\tq8\n").is_err());
        assert!(FleetRegistry::parse_tsv("qwen3.8-27b\tq8\tlots\n").is_err());
        assert!(FleetRegistry::parse_tsv("qwen3.8-27b\tq8\t0\n").is_err());
        assert!(FleetRegistry::parse_tsv("qwen3.8-27b\tq8\t1\nqwen3.8-27b\tq4\t2\n").is_err());
    }

    #[test]
    fn resolve_happy_path_uses_derived_serving_name() {
        let r = registries(&["qwen3.8-27b-q8", "deepseek-r1-q4"]);
        let resolved = r.resolve("qwen3.8-27b").unwrap();
        assert_eq!(resolved.serving, "qwen3.8-27b-q8");
        assert_eq!(resolved.entry.canonical, "qwen3.8-27b");
    }

    #[test]
    fn lookup_miss_on_known_canonical_is_registry_mismatch() {
        // The incident: pick-config regenerated without the suffixed name.
        let r = registries(&["qwen3.8-27b", "deepseek-r1-q4"]);
        let err = r.resolve("qwen3.8-27b").unwrap_err();
        assert_eq!(
            err,
            ResolveError::RegistryMismatch {
                canonical: "qwen3.8-27b".to_string(),
                serving: "qwen3.8-27b-q8".to_string(),
            }
        );
        assert_eq!(err.to_string(), "model qwen3.8-27b is in the fleet registry with derived serving name qwen3.8-27b-q8, but the serving registry (pick-config) does not know it");
    }

    #[test]
    fn lookup_miss_on_unknown_name_is_unknown_not_mismatch() {
        let r = registries(&["qwen3.8-27b-q8"]);
        assert_eq!(
            r.resolve("mistral-7b").unwrap_err(),
            ResolveError::Unknown {
                name: "mistral-7b".to_string(),
            }
        );
    }

    #[test]
    fn no_substring_or_case_fallback() {
        let r = registries(&["qwen3.8-27b-q8"]);
        // Prefix, case variant, and the serving name itself: none of these
        // is the canonical name, and none of them may resolve.
        for name in ["qwen", "Qwen3.8-27B", "qwen3.8-27b-q8"] {
            match r.resolve(name) {
                Err(ResolveError::Unknown { name: got }) => assert_eq!(got, name),
                other => panic!("{name} must be Unknown, got {other:?}"),
            }
        }
    }

    #[test]
    fn audit_names_every_model_pick_config_does_not_know() {
        let r = registries(&["qwen3.8-27b-q8", "retired-model-q2"]);
        let findings = r.audit();
        assert_eq!(
            findings,
            vec![
                AuditFinding::ServingNameMissing {
                    canonical: "deepseek-r1".to_string(),
                    serving: "deepseek-r1-q4".to_string(),
                },
                AuditFinding::OrphanServingName {
                    serving: "retired-model-q2".to_string(),
                },
            ]
        );
        assert_eq!(findings.iter().filter(|f| f.is_blocking()).count(), 1);
    }

    #[test]
    fn audit_is_clean_when_registries_agree() {
        let r = registries(&["qwen3.8-27b-q8", "deepseek-r1-q4"]);
        assert!(r.audit().is_empty());
    }

    #[test]
    fn startup_line_names_the_refusing_serving_registry() {
        let r = registries(&["deepseek-r1-q4"]);
        let err = r.resolve("qwen3.8-27b").unwrap_err();
        let line = StartupRejection::from_resolve("qwen3.8-27b", &err).log_line();
        assert!(!line.contains('\n'));
        assert!(line.contains("serving registry (pick-config)"), "{line}");
        assert!(line.contains("`qwen3.8-27b`"), "{line}");
        assert!(line.contains("qwen3.8-27b-q8"), "{line}");
    }

    #[test]
    fn startup_line_names_the_refusing_fleet_registry() {
        let r = registries(&["qwen3.8-27b-q8"]);
        let err = r.resolve("mistral-7b").unwrap_err();
        let line = StartupRejection::from_resolve("mistral-7b", &err).log_line();
        assert!(!line.contains('\n'));
        assert!(line.contains("fleet registry (models.tsv)"), "{line}");
        assert!(line.contains("`mistral-7b`"), "{line}");
    }

    #[test]
    fn eligible_classes_are_derived_from_catalog_vram_not_listed() {
        // The old shape was a `case "$MODEL"` allowlist; the new check reads
        // the catalog's vram_mib and compares it to each candidate card.
        let fleet = fleet();
        let cards = vec![
            card("gpu:6000_blackwell", 49_152), // fits: 49 GiB >= 40 GiB
            card("gpu:a100", 81_920),           // fits
            card("gpu:t4", 16_384),             // does not fit
        ];
        assert_eq!(
            fleet.eligible_classes("qwen3.8-27b", &cards),
            FitCheck::Fits {
                canonical: "qwen3.8-27b".to_string(),
                classes: vec!["gpu:6000_blackwell".to_string(), "gpu:a100".to_string()],
            }
        );
    }

    #[test]
    fn fit_boundary_is_exact_with_no_invented_margin() {
        // Trap 1: no `* 1.2` safety margin invented at the guard. The
        // catalog's vram_mib is the only model-side input, so equality
        // fits and one MiB short does not.
        let fleet = fleet();
        assert_eq!(
            fleet.eligible_classes("qwen3.8-27b", &[card("gpu:a100", 40_960)]),
            FitCheck::Fits {
                canonical: "qwen3.8-27b".to_string(),
                classes: vec!["gpu:a100".to_string()],
            }
        );
        assert_eq!(
            fleet.eligible_classes("qwen3.8-27b", &[card("gpu:a100", 40_959)]),
            FitCheck::Fits {
                canonical: "qwen3.8-27b".to_string(),
                classes: vec![],
            }
        );
    }

    #[test]
    fn unknown_model_is_unknown_not_an_empty_fit() {
        // Trap 2: a narrow vram probe (exit 0 on some budget) is not a
        // membership test. An unlisted model is `Unknown` — a catalog gap
        // to file — never `Fits { classes: [] }`, which would read as
        // "fits nowhere" and steer the operator to add cards instead of a
        // models.tsv row.
        let fleet = fleet();
        let cards = vec![card("gpu:a100", 81_920)];
        assert_eq!(
            fleet.eligible_classes("glm-5.3-flash", &cards),
            FitCheck::Unknown {
                name: "glm-5.3-flash".to_string()
            }
        );
    }

    #[test]
    fn cataloged_model_that_fits_nowhere_is_an_empty_fit() {
        // The capacity fact a known model can hit: every candidate card is
        // too small. This is `Fits` with an empty class list, distinct
        // from `Unknown`.
        let fleet = fleet();
        let cards = vec![card("gpu:t4", 16_384), card("gpu:rtx4090", 24_576)];
        assert_eq!(
            fleet.eligible_classes("qwen3.8-27b", &cards),
            FitCheck::Fits {
                canonical: "qwen3.8-27b".to_string(),
                classes: vec![],
            }
        );
    }

    #[test]
    fn unmeasured_card_refuses_instead_of_guessing() {
        // A card declaring no VRAM must not be treated as fitting nothing
        // (silently excluded) or as fitting everything (permissive
        // default): both are lies. The check refuses and names the class.
        let fleet = fleet();
        let cards = vec![card("gpu:turing", 0), card("gpu:a100", 81_920)];
        assert_eq!(
            fleet.eligible_classes("qwen3.8-27b", &cards),
            FitCheck::UnmeasuredCard {
                class: "gpu:turing".to_string()
            }
        );
    }

    #[test]
    fn unknown_model_is_checked_before_card_measurement() {
        // The catalog is consulted first: a name it does not know is a
        // catalog gap regardless of what the candidate cards declare.
        let fleet = fleet();
        let cards = vec![card("gpu:turing", 0)];
        assert_eq!(
            fleet.eligible_classes("deepseek-v4-flash", &cards),
            FitCheck::Unknown {
                name: "deepseek-v4-flash".to_string()
            }
        );
    }

    #[test]
    fn duplicate_card_classes_are_deduplicated_in_input_order() {
        // Eligibility is per class, not per physical card: four A100s are
        // one scheduling constraint, first occurrence wins the position.
        let fleet = fleet();
        let cards = vec![
            card("gpu:a100", 81_920),
            card("gpu:6000_blackwell", 49_152),
            card("gpu:a100", 81_920),
            card("gpu:t4", 16_384),
            card("gpu:a100", 81_920),
        ];
        assert_eq!(
            fleet.eligible_classes("qwen3.8-27b", &cards),
            FitCheck::Fits {
                canonical: "qwen3.8-27b".to_string(),
                classes: vec!["gpu:a100".to_string(), "gpu:6000_blackwell".to_string()],
            }
        );
    }

    #[test]
    fn fit_line_for_unknown_names_models_tsv_and_refuses_to_guess() {
        let fleet = fleet();
        let line = fleet
            .eligible_classes("glm-5.3-flash", &[card("gpu:a100", 81_920)])
            .line();
        assert!(!line.contains('\n'));
        assert!(line.contains("models.tsv"), "{line}");
        assert!(line.contains("`glm-5.3-flash`"), "{line}");
        assert!(line.to_lowercase().contains("refus"), "{line}");
    }

    #[test]
    fn fit_line_for_fits_names_the_derived_classes() {
        let fleet = fleet();
        let cards = vec![card("gpu:6000_blackwell", 49_152), card("gpu:t4", 16_384)];
        let line = fleet.eligible_classes("qwen3.8-27b", &cards).line();
        assert!(!line.contains('\n'));
        assert!(line.contains("`qwen3.8-27b`"), "{line}");
        assert!(line.contains("gpu:6000_blackwell"), "{line}");
        assert!(!line.contains("gpu:t4"), "{line}");
    }

    #[test]
    fn fit_line_for_empty_fit_states_the_capacity_fact() {
        let fleet = fleet();
        let line = fleet
            .eligible_classes("qwen3.8-27b", &[card("gpu:t4", 16_384)])
            .line();
        assert!(!line.contains('\n'));
        assert!(line.contains("`qwen3.8-27b`"), "{line}");
        assert!(line.contains("no candidate class"), "{line}");
    }

    #[test]
    fn fit_line_for_unmeasured_card_names_the_class() {
        let fleet = fleet();
        let line = fleet
            .eligible_classes("qwen3.8-27b", &[card("gpu:turing", 0)])
            .line();
        assert!(!line.contains('\n'));
        assert!(line.contains("`gpu:turing`"), "{line}");
        assert!(line.to_lowercase().contains("refus"), "{line}");
    }

    #[test]
    fn first_deficit_may_raise_once() {
        let mut ratchet = ScaleRatchet::new(5);
        assert_eq!(ratchet.tick(2), ScaleAction::Raise { next_desired: 6 });
        assert_eq!(ratchet.desired(), 6);
    }

    #[test]
    fn flat_deficit_holds_and_alarms_instead_of_ratcheting() {
        // The incident: every raised worker died at startup, so running
        // never moved and the old scaler went 5 -> 6 -> 7 -> ...
        let mut ratchet = ScaleRatchet::new(5);
        assert_eq!(ratchet.tick(2), ScaleAction::Raise { next_desired: 6 });
        for _ in 0..3 {
            assert_eq!(
                ratchet.tick(2),
                ScaleAction::Hold {
                    deficit: 4,
                    alarm: true
                }
            );
        }
        assert_eq!(ratchet.desired(), 6);
        assert_eq!(ratchet.alarm_count(), 3);
    }

    #[test]
    fn growing_deficit_holds_and_alarms() {
        let mut ratchet = ScaleRatchet::new(5);
        ratchet.tick(3); // deficit 2, first reading -> raise to 6
                         // A worker dies before the raised one comes up: deficit grows 2 -> 4.
        assert_eq!(
            ratchet.tick(2),
            ScaleAction::Hold {
                deficit: 4,
                alarm: true
            }
        );
    }

    #[test]
    fn shrinking_deficit_raises_again() {
        let mut ratchet = ScaleRatchet::new(5);
        ratchet.tick(2); // deficit 3 -> raise to 6
        assert_eq!(
            ratchet.tick(4), // deficit 2, shrinking -> raise to 7
            ScaleAction::Raise { next_desired: 7 }
        );
        assert_eq!(ratchet.alarm_count(), 0);
    }

    #[test]
    fn zero_deficit_is_a_quiet_hold() {
        let mut ratchet = ScaleRatchet::new(5);
        ratchet.tick(2); // deficit 3 -> raise to 6
        assert_eq!(
            ratchet.tick(6), // fleet caught up: quiet
            ScaleAction::Hold {
                deficit: 0,
                alarm: false
            }
        );
        // Then two workers die at once: deficit 0 -> 4 is not shrinking.
        assert_eq!(
            ratchet.tick(4),
            ScaleAction::Hold {
                deficit: 2,
                alarm: true
            }
        );
    }
}
