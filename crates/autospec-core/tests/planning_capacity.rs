//! Integration tests for `autospec_core::planning::capacity` (issue #3821).
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! §6 (Fleet Capacity Model) and §7.1 (Target Initial Width).
//!
//! Precedence (§6.2): flag > project config > orchestrator > env > default 32.
//! Config files are real files on a temp dir — no mocks.

use std::fs;
use std::path::PathBuf;

use autospec_core::planning::capacity::{
    clamp_capacity, config_target_agents, resolve_capacity, target_initial_width, CapacityInputs,
    DEFAULT_TARGET_AGENTS, MAX_SUPPORTED_AGENTS, MIN_SUPPORTED_AGENTS,
};

fn temp_dir(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "autospec-planning-capacity-{name}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

// ── Precedence levels ────────────────────────────────────────────────────────

#[test]
fn resolve_returns_default_32_when_no_source_present() {
    let inputs = CapacityInputs::default();
    assert_eq!(inputs.default_agents, 32);
    assert_eq!(resolve_capacity(&inputs), 32);
    assert_eq!(DEFAULT_TARGET_AGENTS, 32);
}

#[test]
fn flag_wins_over_config_orchestrator_and_env() {
    let inputs = CapacityInputs {
        agents_flag: Some(48),
        config_target_agents: Some(20),
        orchestrator_capacity: Some(90),
        agent_capacity_env: Some(12),
        ..CapacityInputs::default()
    };
    assert_eq!(resolve_capacity(&inputs), 48);
}

#[test]
fn env_loses_to_explicit_flag() {
    let inputs = CapacityInputs {
        agents_flag: Some(15),
        agent_capacity_env: Some(64),
        ..CapacityInputs::default()
    };
    assert_eq!(resolve_capacity(&inputs), 15);
}

#[test]
fn config_wins_over_orchestrator_and_env() {
    let inputs = CapacityInputs {
        config_target_agents: Some(20),
        orchestrator_capacity: Some(90),
        agent_capacity_env: Some(12),
        ..CapacityInputs::default()
    };
    assert_eq!(resolve_capacity(&inputs), 20);
}

#[test]
fn orchestrator_wins_over_env() {
    let inputs = CapacityInputs {
        orchestrator_capacity: Some(47),
        agent_capacity_env: Some(12),
        ..CapacityInputs::default()
    };
    assert_eq!(resolve_capacity(&inputs), 47);
}

#[test]
fn env_wins_over_default() {
    let inputs = CapacityInputs {
        agent_capacity_env: Some(64),
        ..CapacityInputs::default()
    };
    assert_eq!(resolve_capacity(&inputs), 64);
}

// ── Clamp boundaries ─────────────────────────────────────────────────────────

#[test]
fn clamp_maps_below_range_to_min() {
    assert_eq!(clamp_capacity(9), 10);
    assert_eq!(clamp_capacity(0), 10);
    assert_eq!(clamp_capacity(-5), 10);
    assert_eq!(clamp_capacity(i64::MIN), 10);
    assert_eq!(MIN_SUPPORTED_AGENTS, 10);
}

#[test]
fn clamp_maps_above_range_to_max() {
    assert_eq!(clamp_capacity(101), 100);
    assert_eq!(clamp_capacity(1000), 100);
    assert_eq!(clamp_capacity(i64::MAX), 100);
    assert_eq!(MAX_SUPPORTED_AGENTS, 100);
}

#[test]
fn clamp_keeps_values_inside_closed_range() {
    assert_eq!(clamp_capacity(10), 10);
    assert_eq!(clamp_capacity(32), 32);
    assert_eq!(clamp_capacity(64), 64);
    assert_eq!(clamp_capacity(100), 100);
}

#[test]
fn resolve_clamps_out_of_range_sources() {
    let low = CapacityInputs {
        agents_flag: Some(2),
        ..CapacityInputs::default()
    };
    assert_eq!(resolve_capacity(&low), 10);

    let high = CapacityInputs {
        orchestrator_capacity: Some(500),
        ..CapacityInputs::default()
    };
    assert_eq!(resolve_capacity(&high), 100);
}

// ── Target initial width (§7.1) ──────────────────────────────────────────────

#[test]
fn target_width_is_smaller_issue_pool() {
    assert_eq!(target_initial_width(20, 32), 12);
}

#[test]
fn target_width_is_capped_by_capacity() {
    assert_eq!(target_initial_width(200, 32), 32);
    assert_eq!(target_initial_width(200, 10), 10);
}

#[test]
fn target_width_zero_issues_is_zero() {
    assert_eq!(target_initial_width(0, 32), 0);
}

#[test]
fn target_width_uses_ceil_of_sixty_percent() {
    assert_eq!(target_initial_width(1, 32), 1);
    assert_eq!(target_initial_width(5, 32), 3);
    assert_eq!(target_initial_width(11, 32), 7);
    assert_eq!(target_initial_width(100, 32), 32);
}

// ── Config file handling (real files on a temp dir) ──────────────────────────

#[test]
fn config_loader_reads_target_agents_from_real_file() {
    let dir = temp_dir("read");
    let path = dir.join("autospec.yml");
    fs::write(
        &path,
        "version: 1\ngit:\n  tracked: true\nplanning:\n  parallelism:\n    target_agents: 48\n",
    )
    .expect("write config");
    assert_eq!(config_target_agents(&path).expect("load config"), Some(48));
    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn config_loader_absent_file_returns_none_not_error() {
    let dir = temp_dir("absent");
    let path = dir.join("does-not-exist.yml");
    assert_eq!(
        config_target_agents(&path).expect("absent file is not an error"),
        None
    );
    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn config_loader_without_planning_block_returns_none() {
    let dir = temp_dir("no-planning");
    let path = dir.join("autospec.yml");
    fs::write(&path, "version: 1\ngit:\n  tracked: true\n").expect("write config");
    assert_eq!(config_target_agents(&path).expect("load config"), None);
    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn config_loader_rejects_malformed_structure() {
    let dir = temp_dir("malformed");

    let planning_not_mapping = dir.join("planning-string.yml");
    fs::write(&planning_not_mapping, "planning: 48\n").expect("write config");
    assert!(config_target_agents(&planning_not_mapping).is_err());

    let agents_not_integer = dir.join("agents-string.yml");
    fs::write(
        &agents_not_integer,
        "planning:\n  parallelism:\n    target_agents: many\n",
    )
    .expect("write config");
    assert!(config_target_agents(&agents_not_integer).is_err());

    let bad_yaml = dir.join("bad.yml");
    fs::write(&bad_yaml, "planning: [\n  broken\n").expect("write config");
    assert!(config_target_agents(&bad_yaml).is_err());

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn end_to_end_file_config_beats_env_and_default() {
    let dir = temp_dir("e2e");
    let path = dir.join("autospec.yml");
    fs::write(&path, "planning:\n  parallelism:\n    target_agents: 90\n").expect("write config");
    let inputs = CapacityInputs {
        config_target_agents: config_target_agents(&path).expect("load config"),
        agent_capacity_env: Some(12),
        ..CapacityInputs::default()
    };
    assert_eq!(resolve_capacity(&inputs), 90);
    fs::remove_dir_all(&dir).expect("cleanup");
}
