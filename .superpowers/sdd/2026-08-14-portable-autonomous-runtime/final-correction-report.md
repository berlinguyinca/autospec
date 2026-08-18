# Final heartbeat correction report

Date: 2026-08-14
Branch: `feat/portable-autonomous-runtime`

## Outcome

Portable heartbeat publication no longer depends on Apple-only `renameatx_np` on FreeBSD, and Unix child-directory opens no longer perform a pathname probe before their descriptor-relative open. macOS retains `renameatx_np(RENAME_EXCL)`, Linux production publication is unchanged, and the existing Windows handle-relative branch is unchanged.

On FreeBSD, the deterministic private generation stage is published with descriptor-relative `linkat`. The destination link is the atomic no-replace decision point. The retained directory is synced, then the private stage link is removed and the directory is synced again. If the process stops after the destination link but before stage cleanup, the next exact-generation publication verifies both names are private mode-0600 links to the same inode with link count two, verifies the staged heartbeat generation, removes only that deterministic stage, syncs, then validates the final destination has the same inode, the expected generation, and link count one. A pre-existing destination with a different inode or generation is rejected without removing either name.

## Official FreeBSD version rationale

The FreeBSD 15.1 `rename(2)` history states that `renameat2` appeared in FreeBSD 16.0, so requiring `renameat2(RENAME_NOREPLACE)` would exclude supported FreeBSD 14/15 hosts. The FreeBSD 14.2 `link(2)` page documents `linkat` and states that link creation is atomic; it also records `linkat` as available since FreeBSD 8.0. Therefore retained-directory, descriptor-relative `linkat` is the compatible atomic no-replace publication primitive for FreeBSD 14/15.

- https://man.freebsd.org/cgi/man.cgi?apropos=0&manpath=FreeBSD+15.1-RELEASE+and+Ports.quarterly&query=rename&sektion=2
- https://man.freebsd.org/cgi/man.cgi?apropos=0&manpath=FreeBSD+14.2-RELEASE+and+Ports&query=linkat&sektion=2

## TDD RED evidence

### Retained-parent/session race

Command:

```text
cargo test -p autospec-cli --bin autospec 'commands::claim::heartbeat_portable::tests::publication_remains_bound_to_open_repository_after_parent_swap' -- --exact --nocapture
```

Before the production correction, the extended regression used `session_id: Some("session-a")` and failed deterministically after the repository path swap:

```text
thread 'commands::claim::heartbeat_portable::tests::publication_remains_bound_to_open_repository_after_parent_swap' panicked ...
handle-bound publication: CommandFailure { message: "heartbeat child directory disappeared after creation", exit_code: 2, kind: Diagnostic }
test ...publication_remains_bound_to_open_repository_after_parent_swap ... FAILED
test result: FAILED. 0 passed; 1 failed
```

This failure was caused by `mkdirat` creating `sessions` beneath the retained repository descriptor, followed by `symlink_metadata(parent.path.join("sessions"))` probing the replacement repository pathname.

### FreeBSD workflow coverage

Command:

```text
bats tests/cli/test_rust_workflow.bats
```

Before the workflow correction:

```text
1..1
not ok 1 rust workflow behavior-tests autospec-cli on all supported platforms
# AssertionError
```

The new Bats assertion failed because the FreeBSD workflow did not run the crash-after-link recovery regression.

### FreeBSD test surface

Command after adding the target-gated crash test but before its failpoint/implementation:

```text
cargo check -p autospec-cli --tests --target x86_64-unknown-freebsd
```

Observed RED compiler evidence:

```text
error[E0425]: cannot find value `FREEBSD_CRASH_AFTER_LINK` in this scope
error: could not compile `autospec-cli` (bin "autospec" test) due to 1 previous error
```

The executable FreeBSD regressions are target-gated because they exercise FreeBSD kernel filesystem behavior. Native execution is delegated to the FreeBSD VM workflow; cross-target compilation on the macOS host proves the test and production surfaces compile.

## Files changed

- `crates/autospec-cli/src/commands/claim/heartbeat_portable.rs`
  - Removed Unix `symlink_metadata(parent.path.join(name))` child probing.
  - Opens a single validated child directly beneath `parent.file` with `openat(O_DIRECTORY|O_NOFOLLOW)` and maps `ENOENT` from that descriptor-relative operation only.
  - Split macOS `renameatx_np(RENAME_EXCL)` from FreeBSD.
  - Added FreeBSD `linkat` no-replace publication, directory syncs, deterministic stage cleanup, and exact crash recovery with inode/link-count/generation validation.
  - Extended the parent-swap test to publish and verify both issue and session heartbeats in the retained repository while leaving the replacement untouched.
  - Added FreeBSD-only real-filesystem collision and crash-after-link recovery tests.
- `.github/workflows/rust.yml`
  - Runs both FreeBSD-specific regressions with `run_exact`; crash recovery appears exactly once and the helper positively requires one `... ok` line.
- `tests/cli/test_rust_workflow.bats`
  - Requires each FreeBSD-specific exact test exactly once.
- `.superpowers/sdd/2026-08-14-portable-autonomous-runtime/final-correction-report.md`
  - This evidence and rationale report.

## GREEN verification evidence

Final verification command sequence exited 0:

```text
run_exact publication_is_idempotent_but_rejects_another_generation
run_exact publication_remains_bound_to_open_repository_after_parent_swap
run_exact publication_retry_cleans_crash_staging_aliases
cargo test -p autospec-cli --bin autospec 'commands::claim::heartbeat_portable::tests::' -- --nocapture
bats tests/cli/test_rust_workflow.bats
cargo clippy -p autospec-cli --tests -- -D warnings
cargo check -p autospec-cli --tests --target x86_64-unknown-freebsd
cargo check -p autospec-cli --tests --target x86_64-pc-windows-msvc
cargo check -p autospec-cli --tests --target x86_64-unknown-linux-gnu
cargo fmt --all -- --check
git diff --check
```

Results:

- Three required macOS-host exact heartbeat tests each passed exactly once.
- The complete macOS-host `heartbeat_portable::tests` filter passed: 12 passed, 0 failed.
- Workflow Bats passed: 1 passed, 0 failed.
- Strict autospec-cli test Clippy passed with `-D warnings`.
- FreeBSD, Windows MSVC, and Linux GNU test-target checks exited 0.
- Rustfmt check and `git diff --check` exited 0.
- Windows cross-target output contains existing cfg-specific warnings outside this correction's owned file; the required check still exited 0. The strict native-host Clippy surface was warning-free.

An attempted `cargo test --no-run --target x86_64-unknown-freebsd` compiled all Rust sources but could not link a FreeBSD test binary with the macOS host linker (`ld: unknown options: --as-needed ...`). This is a host cross-linker limitation, not a Rust diagnostic; native execution remains covered by the FreeBSD VM job.

The removed legacy shell validator is unavailable in this worktree. The canonical `cargo run -q -p autospec-cli -- validate` command and all task-specific validation commands listed above were run directly.

## Self-review and remaining risk

- Confirmed Linux `atomic_rename_exclusive` implementation is byte-for-byte unchanged.
- Confirmed the Windows `open_existing_private_child` and Windows publication implementation are unchanged.
- Confirmed macOS still uses `renameatx_np(RENAME_EXCL)` and no FreeBSD build references that symbol.
- Confirmed FreeBSD recovery rejects wrong inode, wrong link count, non-private permissions, malformed content, and wrong generation before removing the deterministic stage.
- Confirmed the crash failpoint is test-only, path-scoped, and consumed before panicking, so parallel FreeBSD tests cannot steal it or leave it armed.
- Confirmed the replacement repository remains untouched in the parent-swap regression, including absence of a `sessions` directory.

Remaining risk: native FreeBSD filesystem execution cannot run on this macOS host. The FreeBSD VM workflow now runs the collision and crash-recovery regressions exactly once with positive pass-count enforcement; CI is the authoritative native execution evidence.
