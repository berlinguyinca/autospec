// executor_bridge tests.
#![allow(dead_code, unused_imports)]
//
// This was one 23,719-line file. The size gate had been telling us to split it for a long
// time and finally made it unavoidable: an oversized file may not grow, so a verified
// eight-line fix to a test race could not land at all. Splitting is the remedy the rule
// prescribes.
//
// The cut follows the order the tests were already written in, so each module is a
// contiguous slice of the original and every case keeps its name — `cargo test` selects
// the same set it always did, one module path deeper. Fixtures used by a single module
// moved with it; the ones shared across modules are in the support_* files.

#[cfg(target_os = "linux")]
mod adoption_cleanup;
mod attempt_generation;
#[cfg(target_os = "linux")]
mod attempt_retirement;
mod base_merge;
#[cfg(target_os = "linux")]
mod branch_predecessor;
#[cfg(target_os = "linux")]
mod cleanup_reap;
#[cfg(target_os = "linux")]
mod cleanup_restart;
mod closeout_harness;
mod closeout_remote;
mod closeout_repairs;
mod codex_permission;
#[cfg(unix)]
mod codex_root_policy;
#[cfg(target_os = "linux")]
mod codex_sandbox;
mod commit_rust;
mod continuation_event;
#[cfg(target_os = "linux")]
mod descendant_spawn;
mod dispatcher_temporary;
#[cfg(unix)]
mod draft_release;
mod explore_pin_staleness;
mod full_suite;
mod generation_input;
#[cfg(target_os = "linux")]
mod harness_death;
#[cfg(target_os = "linux")]
mod harness_supervisor;
#[cfg(target_os = "linux")]
mod identity_reviewer;
mod integration_smoke;
mod json_identity;
mod license_checker;
#[cfg(target_os = "linux")]
mod merged_reconciliation;
mod npm_inputs;
mod ordered_publication;
mod production_entry;
mod proxy_direct;
mod prunable_zero;
#[cfg(target_os = "linux")]
mod pull_mutation;
#[cfg(target_os = "linux")]
mod quarantine_nested;
mod ready_harness;
mod remote_base;
mod repair_implementation;
#[cfg(target_os = "linux")]
mod restart_direct;
mod result_reviewer;
mod review_provider;
mod reviewer_automatic;
#[cfg(target_os = "linux")]
mod reviewer_runtime;
mod runtime_close;
#[cfg(target_os = "linux")]
mod runtime_fixture;
#[cfg(target_os = "linux")]
mod rust_commit;
mod scanner_command;
mod scope_root;
#[cfg(target_os = "linux")]
mod sidecar_launch;
#[cfg(target_os = "linux")]
mod snapshot_identity;
mod structural_repository_failure;
mod structured_review;
mod structured_review_receipt;
mod support_base;
#[cfg(unix)]
mod trusted_codex_launch;
#[cfg(windows)]
pub(super) use support_base::TEST_ENVIRONMENT;
mod support_harness_env;
pub(super) mod support_invocation;
mod support_launch;
mod support_review;
mod support_shim;
mod sync_integration;
mod terminal_label;
mod worktree_post;
