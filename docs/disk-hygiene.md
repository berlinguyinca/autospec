# Disk hygiene

Autospec worktrees, Rust `target/` dirs and test containers are the main disk
consumers on agent hosts. `scripts/disk-hygiene.sh` (installed to
`~/.autospec/scripts/`) is ON by default and works with or without
`herdr-eng` installed.

| Piece | Behaviour | Opt-out |
|---|---|---|
| Worktree cleanup | After a PR merges/closes, or a run fails/aborts, the worktree and its `target/` dir are removed when the tree is clean and its work is pushed. Otherwise it is kept and `disk_hygiene:worktree_kept` is logged. The path must be exactly a registered, unlocked linked-worktree root (never a subdirectory, the primary checkout or a `git worktree lock`ed tree). Prefers `herdr-eng cleanup --scope worktree --path <wt>` (detected with `herdr-eng cleanup --help`), falls back to `git worktree remove`; leftovers are swept only after git removal succeeded, never as a fallback after a failure. | `AUTOSPEC_DISK_CLEANUP=off` |
| "Pushed" | After a best-effort `git fetch --prune`, HEAD is pushed when it is reachable from its upstream (or, without an upstream, any remote), OR its PR is MERGED at exactly HEAD (needs `gh`), OR it is patch-equivalent to the default branch (`git cherry`). So squash-merged-and-deleted branches are reclaimed and stale remote refs do not hide lost work. | n/a |
| Ownership | `worktree-guard.sh create` writes its PID to `$WORKTREE_GUARD_CREATED_FILE` only when it really created the worktree. `dispatch-implementer.sh` and `explore-qa-gate.sh` clean up on abort only worktrees they created, never a reused or refused one. | n/a |
| Rust builds | `worktree-guard.sh create` (create and reuse) writes a git-excluded `.autospec-env.sh` exporting `CARGO_INCREMENTAL=0`. `CARGO_TARGET_DIR` is left alone: each worktree builds into its own `target/`, removed with it, so concurrent branches never overwrite each other's test binaries. | `AUTOSPEC_DISK_HYGIENE=off` |
| Docker | Test containers use `--rm --label herdr.ephemeral=1`; implementer prompts say so. | n/a |
| Disk pre-flight | Phase 4 dispatch, the autonomous drain and each conductor cycle check free space on the worktree, `$CARGO_TARGET_DIR` (if set) and `$HOME`. A path is low only when free space is below BOTH `AUTOSPEC_DISK_MIN_FREE_PCT` (5) AND `AUTOSPEC_DISK_MIN_FREE_GIB` (20), so big disks are not parked by the percentage nor small hosts by the floor. It runs `herdr-eng cleanup`/`build-cleanup run` if available, re-checks, then exits 75 (`code_health:disk_low_parked`) so the issue is parked. A parked conductor cycle still writes its heartbeat, processes the control channel and counts toward `CONDUCTOR_MAX_CYCLES`, and sleeps at least `AUTOSPEC_DISK_PARK_SLEEP_SECS` (60). | `AUTOSPEC_DISK_PREFLIGHT=off` |
| Watchdog GC | An orphaned `/tmp/wt-*` worktree is reclaimed when its heartbeat is stale and the issue is closed or no longer labelled in-flight. For an open, in-flight issue only a PR at exactly the worktree HEAD releases it: MERGED, or CLOSED plus `AUTOSPEC_WATCHDOG_GC_PR_CLOSED_GRACE_SECS` (3600) of worktree idleness (a failed run closes its PR and retries on the same branch). The dirty/unpushed check runs before any runtime teardown. | `AUTOSPEC_DISK_HYGIENE=off` |
| Everything | | `AUTOSPEC_DISK_HYGIENE=off` |

`AUTOSPEC_HERDR_ENG` overrides the `herdr-eng` binary name/path.
