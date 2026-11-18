# Disk hygiene

Autospec worktrees, Rust `target/` dirs and test containers are the main disk
consumers on agent hosts. `scripts/disk-hygiene.sh` (installed to
`~/.autospec/scripts/`) is ON by default and works with or without
`herdr-eng` installed.

| Piece | Behaviour | Opt-out |
|---|---|---|
| Worktree cleanup | After a PR merges/closes, or a run fails/aborts, the worktree and its target dir are removed when the tree is clean and every commit is pushed. Otherwise it is kept and `disk_hygiene:worktree_kept` is logged. Prefers `herdr-eng cleanup --scope worktree --path <wt>` (detected with `herdr-eng cleanup --help`), falls back to `git worktree remove`. | `AUTOSPEC_DISK_CLEANUP=off` |
| Shared Rust target | `worktree-guard.sh create` writes a git-excluded `.autospec-env.sh` exporting `CARGO_TARGET_DIR=${AUTOSPEC_CARGO_TARGET_ROOT:-$HOME/.cache/autospec/targets}/<repo-slug>` and `CARGO_INCREMENTAL=0`. A caller-set `CARGO_TARGET_DIR`, or `herdr-eng cargo-host` managing targets, wins. | `AUTOSPEC_SHARED_TARGET=off` |
| Docker | Test containers use `--rm --label herdr.ephemeral=1`; implementer prompts say so. | n/a |
| Disk pre-flight | Phase 4 dispatch, the autonomous drain and each conductor cycle check free space. Below `AUTOSPEC_DISK_MIN_FREE_PCT` (10) or `AUTOSPEC_DISK_MIN_FREE_GIB` (100) it runs `herdr-eng cleanup`/`build-cleanup run` if available, re-checks, then exits 75 (`code_health:disk_low_parked`) so the issue is parked. | `AUTOSPEC_DISK_PREFLIGHT=off` |
| Everything | | `AUTOSPEC_DISK_HYGIENE=off` |

`AUTOSPEC_HERDR_ENG` overrides the `herdr-eng` binary name/path. Concurrent
builds from several worktrees share one target dir: cargo serialises on its
build lock, trading some parallelism for large disk savings.
