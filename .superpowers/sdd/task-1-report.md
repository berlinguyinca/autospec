# Task 1 report: Restore the supported macOS build boundary

## Result

The autospec CLI now builds on `aarch64-apple-darwin`. Native executor process ownership,
forked supervision, direct execution, and draft creation are coherently Linux-only. The public
executor entry and narrow internal seams fail before side effects on non-Linux with:

`executor supervision requires Linux pidfd ownership`

Common receipt/state/recovery code remains portable. Harness alias parsing, safe executable
resolution, default OpenCode adapter resolution, and primary supervisor executable resolution
remain available on macOS. Heartbeat predecessor retirement now fails closed on non-Linux rather
than silently succeeding.

## Files changed

- `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
  - Added the side-effect-free Linux admission boundary and non-Linux executor entry stub.
  - Gated Linux pidfd/subreaper/fork/direct/draft ownership seams as one implementation unit.
  - Added non-Linux admission, harness parser, and supervisor resolver tests.
  - Kept the full process-ownership test module enabled on Linux, where its semantics are valid.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/post_fork.rs`
  - Narrowed all post-fork syscall helpers from Unix to Linux.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/harness.rs`
  - Removed Linux gates from `safe_executable` and `default_opencode_adapter`.
- `crates/autospec-cli/src/commands/claim.rs`
  - Made non-Linux startup heartbeat retirement fail closed.
  - Kept Linux-specific claim heartbeat unit tests on Linux.
- `crates/autospec-cli/src/commands/claim/heartbeat_predecessor.rs`
  - Added a non-Linux fail-closed predecessor retirement implementation.
- `crates/autospec-cli/src/commands/runtime/env/state.rs`
  - Cast `PRIVATE_FILE_MODE` to `nix::libc::mode_t` only at the `Mode` boundary.

`trusted_git.rs` required no edit after the parent Linux admission boundary and portable
`safe_executable` repair resolved its macOS compilation path.

## TDD evidence

### RED

Command:

```text
cargo test -p autospec-cli executor_bridge_fails_closed_before_state_mutation_without_linux_pidfds --no-run
```

Result before production edits: exit 101. The macOS compiler reported the expected boundary leaks,
including unresolved `nix::unistd::pipe2`, missing `OwnedProcess` / `OwnedProcessSet` /
`ForkedChild` and post-fork helpers, gated `safe_executable` / `default_opencode_adapter`, missing
Linux-only predecessor retirement, and Darwin's `Mode` type mismatch. The bin target reported 96
errors and the test target reported 203 errors.

### GREEN

Fresh final verification command:

```text
cargo check -p autospec-cli && \
cargo test -p autospec-cli executor_bridge_fails_closed_before_state_mutation_without_linux_pidfds && \
cargo test -p autospec-cli executor_bridge_keeps && \
cargo build --release -p autospec-cli && \
cargo test --workspace --no-run
```

Result: exit 0.

- `cargo check -p autospec-cli`: PASS.
- Unsupported-platform admission: 1 passed, 0 failed.
- Portable parser/resolver coverage: 2 passed, 0 failed.
- `cargo build --release -p autospec-cli`: PASS.
- `cargo test --workspace --no-run`: PASS; all workspace test executables compiled.

`git diff --check` over the Task 1 files also passed with no whitespace errors.

## Self-review

- The non-Linux public entry invokes admission before reading the terminal receipt, canonicalizing
  the repository, creating directories, or writing durable state.
- Every non-Linux internal stub returns the same executor supervision contract; none attempts a
  weaker Darwin emulation.
- Linux code bodies and process ownership semantics were not rewritten; only their compilation
  boundary changed.
- Portable harness path checks were explicitly ungated and exercised on macOS.
- No dependency, CI, public configuration, or unrelated runtime behavior was added.
- No changes were made outside the assigned Task 1 source files and this report.

## Concerns / pending verification

- Linux runtime executor supervision tests were not run locally because this host is macOS.
  `cargo test -p autospec-cli executor_bridge` remains pending in Linux CI/container verification.
- The repository currently has pre-existing `cargo fmt --all -- --check` findings across many
  unrelated files. Task 1 files pass `git diff --check`; broad formatting was intentionally not
  applied because it would overwrite or absorb unrelated concurrent work.
- macOS compilation emits existing/expected unused-code warnings after excluding Linux ownership
  paths; there are zero compiler errors and no warning is promoted to an error by current gates.

## Review-fix report (2026-08-13)

### Result

- Non-Linux predecessor retirement again returns `Ok(())` when a fresh acquisition has no
  released predecessor, while a released predecessor still fails closed because pidfd ownership
  is unavailable.
- The executor-bridge and claim parent test trees are restored as `#[cfg(test)]`; Linux process
  ownership is gated at the child module or individual case boundary.
- The macOS `autospec` unit-test binary now discovers 517 tests instead of the 227 tests visible
  after the original Task 1 change. The required structured receipt and paginated parser cases
  execute rather than reporting zero tests.
- Linux production code bodies were not changed. Linux runtime verification remains pending CI.

### RED

Command:

```text
cargo test -p autospec-cli fresh_acquisition_without_predecessor_needs_no_linux_retirement
```

Exact failing result before the production fix:

```text
running 1 test
test commands::claim::heartbeat_predecessor::tests::fresh_acquisition_without_predecessor_needs_no_linux_retirement ... FAILED

fresh acquisition has nothing to retire: CommandFailure { message: "predecessor heartbeat retirement requires Linux pidfd ownership", exit_code: 2, kind: Diagnostic }

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 226 filtered out; finished in 0.00s
```

### GREEN focused tests

Commands:

```text
cargo test -q -p autospec-cli --bin autospec fresh_acquisition_without_predecessor_needs_no_linux_retirement
cargo test -q -p autospec-cli --bin autospec released_predecessor_requires_linux_pidfd_retirement
cargo test -q -p autospec-cli --bin autospec structured_review_receipt
cargo test -q -p autospec-cli --bin autospec paginated_comments_parser_flattens_two_raw_pages
```

Exact results:

```text
running 1 test
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 516 filtered out; finished in 0.00s

running 1 test
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 516 filtered out; finished in 0.00s

running 6 tests
......
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 511 filtered out; finished in 0.88s

running 1 test
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 516 filtered out; finished in 0.00s
```

Test discovery command and exact result:

```text
cargo test -q -p autospec-cli --bin autospec -- --list 2>/dev/null | rg ': test$' | wc -l
517
```

### Required build and workspace verification

```text
cargo build --release -p autospec-cli
Finished `release` profile [optimized] target(s) in 0.01s

cargo test --workspace --no-run
Finished `test` profile [unoptimized + debuginfo] target(s) in 0.03s
```

`cargo test --workspace` executed the restored macOS test tree and exited 101. The Task 1
regressions and representative portable tests passed. The final `autospec` unit-test summary was:

```text
test result: FAILED. 404 passed; 113 failed; 0 ignored; 0 measured; 0 filtered out; finished in 35.45s
```

These are unrelated portability gaps exposed by restoring the parent test trees, not Task 1
regressions. The dominant exact signatures were 32 failures from the literal Linux executor root
being rejected on macOS (`executor path contains a symlink: /tmp`) and 31 failures from fixtures
that do not create the expected executor Git hook directory (`canonicalize executor Git hook
directory: No such file or directory (os error 2)`). Other cases depend on Linux direct-process /
sandbox behavior or absent external tools such as `semgrep`. They were deliberately not hidden by
broad test gates because they are generic fixture portability defects outside the Task 1 ownership
boundary. Two deterministic Tier-2 runner failures are also unchanged files outside Task 1 and
expose pre-existing macOS executable assumptions:

```text
bounded_child_output_is_private:
child can inspect its output descriptor: "child exited 1"

bounded_child_preserves_a_nonzero_status:
unexpected child error: cannot spawn /bin/false: No such file or directory (os error 2)
```

`target/release/autospec validate` was runnable but produced no output and did not complete after
more than two minutes; it was interrupted with exit 130. No shell or skill files changed in Task 1,
so install, lock-step, and `bash -n` gates belong to later tasks and are not claimed here.

`cargo fmt --all -- --check` remains blocked by the repository-wide pre-existing formatting diff
documented above. `git diff --check` is used for this review-fix diff.

## Second review-fix report (2026-08-13)

### Result

- Restored the mixed `json_identity`, `heartbeat_prior`, and `heartbeat_classify` modules on
  macOS. Linux-only helpers and test cases remain gated at their individual imports/functions.
- The portable `startup_heartbeat_portable_unix` test uses a width-neutral device-number
  comparison because Darwin's `stat.st_dev` and Rust's `MetadataExt::dev()` expose different
  integer types.
- The macOS `autospec` test binary now discovers 538 tests, up from 517 before this pass.

### RED

Before removing the three whole-module gates, each required filter returned the same exact result:

```text
cargo test -q -p autospec-cli --bin autospec startup_heartbeat_portable_unix
cargo test -q -p autospec-cli --bin autospec unsupported_platform_stale_recovery_is_fail_closed
cargo test -q -p autospec-cli --bin autospec json_identity

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 517 filtered out; finished in 0.00s
```

### GREEN focused tests

The portable heartbeat, unsupported-platform classification, and three representative neutral
JSON identity cases each produced:

```text
running 1 test
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 537 filtered out; finished in 0.00s
```

The exact JSON identity filters were:

```text
autonomous_executor_bridge_persisted_json_rejects_unknown_fields
autonomous_executor_bridge_persisted_json_requires_supported_schema
autonomous_executor_bridge_process_identity_requires_every_component
```

Previously green regressions remained green:

```text
fresh_acquisition_without_predecessor_needs_no_linux_retirement: 1 passed
released_predecessor_requires_linux_pidfd_retirement: 1 passed
structured_review_receipt: 6 passed
paginated_comments_parser_flattens_two_raw_pages: 1 passed
```

Test discovery:

```text
cargo test -q -p autospec-cli --bin autospec -- --list 2>/dev/null | rg ': test$' | wc -l
538
```

### Required build verification

```text
cargo build --release -p autospec-cli
Finished `release` profile [optimized] target(s) in 0.04s

cargo test --workspace --no-run
Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
```

The known 113 broader macOS fixture/tool failures were not rerun or suppressed in this pass.
Linux pidfd, `/proc`, fork, subreaper, direct-process ownership, and Linux executable semantics
remain gated for Linux CI.
