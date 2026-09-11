//! Integration tests for `autospec_core::toolchain_gate` (issue #4303).
//!
//! One fixture test per rule, plus a regression test that reconstructs the
//! incident: the pin at 1.91.0, the CI step that named `stable`, the lints
//! 1.91.0 does not carry (`manual_checked_division`,
//! `truncating_to_zero_length`), the gate red on every input, and the
//! standing instruction to ignore it with no filed report — and the
//! post-fix state where removing the override restores the pin.

use autospec_core::toolchain_gate::{
    audit, drift_line, gate_standing, parse_channel, pin_change_verdict, pin_drift, standing_line,
    workaround_line, workaround_verdict, ChannelKind, Drift, DriftDirection, GateRecord,
    GateStanding, PinChange, PinChangeVerdict, PinFile, StepToolchain, ToolchainChannel,
    ToolchainVersion, Workaround, WorkaroundVerdict, Workflow, WorkflowStep,
};

/// The repository's pin, as declared in `rust-toolchain.toml` (issue #3741).
const PIN_TOML: &str = r#"
# Pinned so formatting is reproducible across CI, developer machines and cluster
# agents.
#
# 1.91.0 specifically: it is the only toolchain available on the HPC cluster the
# agents run on. CI can install any version; the cluster cannot.
[toolchain]
channel = "1.91.0"
components = ["rustfmt", "clippy"]
"#;

fn v(major: u16, minor: u16, patch: u16) -> ToolchainChannel {
    ToolchainChannel::Version(ToolchainVersion::new(major, minor, patch))
}

fn stable() -> ToolchainChannel {
    ToolchainChannel::Floating(ChannelKind::Stable)
}

fn pin() -> PinFile {
    PinFile::from_toml("rust-toolchain.toml", PIN_TOML).expect("the repository pin parses")
}

// Rule 1: a gate runs the toolchain the repository declared.

#[test]
fn the_incident_gate_drifts_the_pin() {
    let pin = pin();
    let step = StepToolchain::Channel(stable());
    let drift = pin_drift(&pin, &step);
    let Drift::Overridden {
        pinned,
        requested,
        direction,
    } = drift
    else {
        panic!("the incident step named a different channel than the pin: expected drift");
    };
    assert_eq!(pinned, v(1, 91, 0));
    assert_eq!(requested, stable());
    // `stable` moves on its own schedule: the direction is not newer or
    // older, it is "moves independently".
    assert_eq!(direction, DriftDirection::MovesIndependently);
    let line = drift_line(&pin, "clippy", &drift);
    // The finding names both channels and the pin file.
    assert!(line.contains("1.91.0"), "line: {line}");
    assert!(line.contains("stable"), "line: {line}");
    assert!(line.contains("rust-toolchain.toml"), "line: {line}");
    assert!(line.contains("single source of truth"), "line: {line}");
}

#[test]
fn removing_the_override_restores_the_pin() {
    // The InferWeave fix (inferweave#323): the `toolchain:` input is
    // removed and the setup action reads the pin.
    let pin = pin();
    let step = StepToolchain::Unspecified;
    assert_eq!(pin_drift(&pin, &step), Drift::None);
}

#[test]
fn a_step_naming_the_pinned_version_is_not_drift() {
    let pin = pin();
    let step = StepToolchain::Channel(v(1, 91, 0));
    assert_eq!(pin_drift(&pin, &step), Drift::None);
}

// Rule 2: drift is directional, and never "stable is fine".

#[test]
fn a_newer_version_is_an_upgrade_made_by_accident() {
    let pin = pin();
    let step = StepToolchain::Channel(v(1, 93, 0));
    let Drift::Overridden { direction, .. } = pin_drift(&pin, &step) else {
        panic!("a newer version is drift");
    };
    assert_eq!(direction, DriftDirection::Newer);
    let line = drift_line(
        &pin,
        "test",
        &Drift::Overridden {
            pinned: pin.channel,
            requested: v(1, 93, 0),
            direction,
        },
    );
    assert!(line.contains("newer"), "line: {line}");
    assert!(line.contains("upgrade made by accident"), "line: {line}");
    assert!(line.contains("recorded decision"), "line: {line}");
}

#[test]
fn an_older_version_drifts_the_other_way() {
    let pin = pin();
    let step = StepToolchain::Channel(v(1, 80, 0));
    let Drift::Overridden { direction, .. } = pin_drift(&pin, &step) else {
        panic!("an older version is drift");
    };
    assert_eq!(direction, DriftDirection::Older);
}

// Pin parsing.

#[test]
fn the_repo_pin_parses_to_the_exact_version() {
    let pin = PinFile::from_toml("rust-toolchain.toml", PIN_TOML).unwrap();
    assert_eq!(pin.channel, v(1, 91, 0));
    assert_eq!(pin.path, "rust-toolchain.toml");
}

#[test]
fn a_top_level_channel_key_parses() {
    let pin = PinFile::from_toml("rust-toolchain.toml", "channel = \"1.91.0\"\n").unwrap();
    assert_eq!(pin.channel, v(1, 91, 0));
}

#[test]
fn a_triple_suffix_is_ignored() {
    let pin = PinFile::from_toml(
        "rust-toolchain.toml",
        "channel = \"stable-x86_64-unknown-linux-gnu\"",
    )
    .unwrap();
    assert_eq!(pin.channel, stable());
}

#[test]
fn a_version_with_a_triple_suffix_parses_to_the_version() {
    let channel = parse_channel("1.91.0-x86_64-unknown-linux-gnu").unwrap();
    assert_eq!(channel, v(1, 91, 0));
}

#[test]
fn a_two_component_version_defaults_the_patch_to_zero() {
    let channel = parse_channel("1.91").unwrap();
    assert_eq!(channel, v(1, 91, 0));
}

#[test]
fn comments_are_skipped_not_parsed() {
    let contents = "# channel = \"1.80.0\"\n\n[toolchain]\nchannel = \"1.91.0\"\n";
    let pin = PinFile::from_toml("rust-toolchain.toml", contents).unwrap();
    assert_eq!(pin.channel, v(1, 91, 0));
}

#[test]
fn a_duplicate_channel_key_is_rejected() {
    let contents = "channel = \"1.91.0\"\n[toolchain]\nchannel = \"1.92.0\"\n";
    let err = PinFile::from_toml("rust-toolchain.toml", contents).unwrap_err();
    assert!(err.contains("duplicate"), "err: {err}");
}

#[test]
fn a_missing_channel_key_is_rejected() {
    let err =
        PinFile::from_toml("rust-toolchain.toml", "components = [\"rustfmt\"]\n").unwrap_err();
    assert!(err.contains("no `channel` key"), "err: {err}");
}

#[test]
fn an_unquoted_channel_value_is_rejected() {
    let err = PinFile::from_toml("rust-toolchain.toml", "channel = 1.91.0\n").unwrap_err();
    assert!(err.contains("quoted string"), "err: {err}");
}

#[test]
fn unrecognized_channels_are_refused_not_guessed() {
    for bad in ["weird", "1.9.1.0", "1", "2026", "1.91.0-beta.2"] {
        let err = parse_channel(bad).unwrap_err();
        assert!(err.contains("refusing to guess"), "{bad}: {err}");
    }
    assert_eq!(
        parse_channel("nightly").unwrap(),
        ToolchainChannel::Floating(ChannelKind::Nightly)
    );
    assert_eq!(
        parse_channel("beta").unwrap(),
        ToolchainChannel::Floating(ChannelKind::Beta)
    );
}

// Rule 3: a check that has never passed is a defect report.

#[test]
fn a_gate_red_on_every_run_is_a_defect_report() {
    // The incident gate: red on every run of the default branch.
    let record = GateRecord {
        name: "clippy".to_string(),
        runs: vec![false; 47],
    };
    assert_eq!(
        gate_standing(&record),
        GateStanding::DefectReport { runs: 47 }
    );
    let line = standing_line(&record);
    assert!(line.contains("47"), "line: {line}");
    assert!(line.contains("defect report"), "line: {line}");
    assert!(line.contains("not a quality signal"), "line: {line}");
    assert!(line.contains("diagnosis nobody filed"), "line: {line}");
}

#[test]
fn a_gate_that_has_passed_is_a_quality_signal() {
    let record = GateRecord {
        name: "test".to_string(),
        runs: vec![
            true, false, true, true, false, false, true, false, true, false,
        ],
    };
    assert_eq!(
        gate_standing(&record),
        GateStanding::QualitySignal {
            greens: 5,
            runs: 10
        }
    );
    let line = standing_line(&record);
    assert!(line.contains("quality signal"), "line: {line}");
    assert!(line.contains("5 of 10"), "line: {line}");
}

#[test]
fn an_unobserved_gate_is_unobserved() {
    let record = GateRecord {
        name: "new-gate".to_string(),
        runs: vec![],
    };
    assert_eq!(gate_standing(&record), GateStanding::Unobserved);
}

// Rule 4: a pin upgrade is a decision with work attached.

#[test]
fn an_unrecorded_pin_bump_is_a_decision_made_by_accident() {
    let bump = PinChange {
        from: v(1, 91, 0),
        to: v(1, 93, 0),
        decision_recorded: false,
    };
    assert_eq!(
        pin_change_verdict(&bump),
        PinChangeVerdict::UnrecordedBump {
            direction: DriftDirection::Newer
        }
    );
}

#[test]
fn a_recorded_pin_bump_is_a_decision() {
    let bump = PinChange {
        from: v(1, 91, 0),
        to: v(1, 93, 0),
        decision_recorded: true,
    };
    assert_eq!(pin_change_verdict(&bump), PinChangeVerdict::RecordedBump);
}

#[test]
fn an_unchanged_pin_is_not_a_decision() {
    let same = PinChange {
        from: v(1, 91, 0),
        to: v(1, 91, 0),
        decision_recorded: false,
    };
    assert_eq!(pin_change_verdict(&same), PinChangeVerdict::Unchanged);
}

// Rule 5: a standing instruction to ignore a signal is a bug report.

#[test]
fn a_standing_ignore_with_no_bug_report_is_the_bug_report() {
    // The incident workaround: a note telling people not to trust the
    // gate, with no issue filed for it.
    let workaround = Workaround {
        gate: "clippy".to_string(),
        instruction: "clippy failures are expected; ignore".to_string(),
        bug_report: None,
    };
    assert_eq!(
        workaround_verdict(&workaround),
        WorkaroundVerdict::UnfiledBugReport
    );
    let line = workaround_line(&workaround);
    assert!(line.contains("is a bug report"), "line: {line}");
    assert!(line.contains("clippy"), "line: {line}");
}

#[test]
fn a_workaround_that_files_the_issue_is_not_a_finding() {
    let workaround = Workaround {
        gate: "clippy".to_string(),
        instruction: "ignore until fixed".to_string(),
        bug_report: Some("InferWeave/inferweave#323".to_string()),
    };
    assert_eq!(workaround_verdict(&workaround), WorkaroundVerdict::Filed);
    let line = workaround_line(&workaround);
    assert!(line.contains("InferWeave/inferweave#323"), "line: {line}");
}

// The audit.

#[test]
fn audit_names_every_drifting_step_with_its_path() {
    let pin = pin();
    let clean = Workflow {
        path: ".github/workflows/rust.yml".to_string(),
        steps: vec![
            WorkflowStep {
                id: "toolchain".to_string(),
                toolchain: StepToolchain::Unspecified,
            },
            WorkflowStep {
                id: "clippy".to_string(),
                toolchain: StepToolchain::Channel(v(1, 91, 0)),
            },
        ],
    };
    let drifting = Workflow {
        path: ".github/workflows/ci.yml".to_string(),
        steps: vec![
            WorkflowStep {
                id: "clippy".to_string(),
                toolchain: StepToolchain::Channel(stable()),
            },
            WorkflowStep {
                id: "test".to_string(),
                toolchain: StepToolchain::Unspecified,
            },
        ],
    };
    let findings = audit(&pin, &[clean, drifting]);
    assert_eq!(findings.len(), 1, "findings: {findings:?}");
    assert_eq!(findings[0].rule, "TOOLCHAIN_GATE_DRIFT");
    assert!(findings[0].detail.contains(".github/workflows/ci.yml"));
    assert!(findings[0].detail.contains("clippy"));
    assert!(findings[0].detail.contains("1.91.0"));
}

#[test]
fn audit_is_clean_when_all_steps_inherit_the_pin() {
    let pin = pin();
    let workflow = Workflow {
        path: ".github/workflows/rust.yml".to_string(),
        steps: vec![WorkflowStep {
            id: "toolchain".to_string(),
            toolchain: StepToolchain::Unspecified,
        }],
    };
    assert!(audit(&pin, &[workflow]).is_empty());
}

// The incident, end to end: pre-fix state, the diagnosis, and the fix.

#[test]
fn the_incident_end_to_end() {
    let pin = pin();

    // Pre-fix: the step names `stable` while the repository declares
    // 1.91.0. The gate is red on every input because `stable` carries
    // lints 1.91.0 does not (`manual_checked_division`,
    // `truncating_to_zero_length`), and nothing in the failure output
    // says which toolchain ran.
    let pre = Workflow {
        path: ".github/workflows/ci.yml".to_string(),
        steps: vec![
            WorkflowStep {
                id: "setup-rust".to_string(),
                toolchain: StepToolchain::Channel(stable()),
            },
            WorkflowStep {
                id: "clippy".to_string(),
                toolchain: StepToolchain::Unspecified,
            },
        ],
    };
    let findings = audit(&pin, std::slice::from_ref(&pre));
    assert_eq!(findings.len(), 1, "findings: {findings:?}");
    assert!(findings[0].detail.contains("setup-rust"));

    // The gate has never passed: every input red. That is a defect
    // report, not a quality signal.
    let gate = GateRecord {
        name: "clippy".to_string(),
        runs: vec![false; 12],
    };
    assert_eq!(
        gate_standing(&gate),
        GateStanding::DefectReport { runs: 12 }
    );

    // And the standing instruction to ignore it, with no filed report:
    // the workaround is the diagnosis nobody filed.
    let workaround = Workaround {
        gate: "clippy".to_string(),
        instruction: "clippy failures are expected; ignore".to_string(),
        bug_report: None,
    };
    assert_eq!(
        workaround_verdict(&workaround),
        WorkaroundVerdict::UnfiledBugReport
    );

    // Post-fix (inferweave#323): the override is removed; the pin holds;
    // the audit is clean.
    let post = Workflow {
        path: ".github/workflows/ci.yml".to_string(),
        steps: vec![
            WorkflowStep {
                id: "setup-rust".to_string(),
                toolchain: StepToolchain::Unspecified,
            },
            WorkflowStep {
                id: "clippy".to_string(),
                toolchain: StepToolchain::Unspecified,
            },
        ],
    };
    assert!(audit(&pin, &[post]).is_empty());
}
