// executor_bridge tests.
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

mod adoption_cleanup;
mod attempt_generation;
mod attempt_retirement;
mod base_merge;
mod branch_predecessor;
mod cleanup_reap;
mod cleanup_restart;
mod closeout_harness;
mod closeout_remote;
mod closeout_repairs;
mod codex_permission;
mod codex_sandbox;
mod commit_rust;
mod continuation_event;
mod descendant_spawn;
mod dispatcher_temporary;
mod draft_release;
mod explore_pin_staleness;
mod full_suite;
mod generation_input;
mod harness_death;
mod harness_supervisor;
mod identity_reviewer;
mod integration_smoke;
mod json_identity;
mod license_checker;
mod merged_reconciliation;
mod npm_inputs;
mod ordered_publication;
mod production_entry;
mod proxy_direct;
mod prunable_zero;
mod pull_mutation;
mod quarantine_nested;
mod ready_harness;
mod remote_base;
mod repair_implementation;
mod restart_direct;
mod result_reviewer;
mod review_provider;
mod reviewer_automatic;
mod reviewer_runtime;
mod runtime_close;
mod runtime_fixture;
mod rust_commit;
mod scanner_command;
mod scope_root;
mod sidecar_launch;
mod snapshot_identity;
mod structured_review;
mod structured_review_receipt;
mod support_base;
mod support_harness_env;
mod support_invocation;
mod support_launch;
mod support_review;
mod sync_integration;
mod terminal_label;
mod worktree_post;
