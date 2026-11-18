### Added

- Disk hygiene, ON by default (`scripts/disk-hygiene.sh`, docs in `docs/disk-hygiene.md`): guarded worktree cleanup on every exit path (clean and fully pushed trees only; dirty or unpushed work is kept and logged; prefers `herdr-eng cleanup --scope worktree`), `CARGO_INCREMENTAL=0` for Rust worktrees (each keeps its own `target/`, removed with it), `--rm --label herdr.ephemeral=1` on test containers, and a free-space pre-flight that parks work (exit 75, `code_health:disk_low_parked`) instead of starting a build that will fail (default floor: below both 5% and 20 GiB on the worktree, `$CARGO_TARGET_DIR` or `$HOME` volume). Removal only targets a registered, unlocked worktree root; "pushed" accepts a MERGED PR at HEAD or a patch-equivalent commit set. Opt-outs: `AUTOSPEC_DISK_HYGIENE=off`, `AUTOSPEC_DISK_CLEANUP=off`, `AUTOSPEC_DISK_PREFLIGHT=off`.

### Fixed

- The watchdog orphaned-worktree GC un-pushed-commits guard used `git log --not --remotes`, which lists nothing without a positive revision, so it never detected unpushed commits. It now uses `git log HEAD --not --remotes`.
