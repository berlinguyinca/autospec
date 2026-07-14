# Rust validation cutover catalog baseline

Date: 2026-07-12
Source: retired shell dispatcher at `5fdc31f9`

## Frozen catalog

The version-1 catalog contains **149** uniquely named `check_*` gates. Its canonical
order is the declaration order in the retired shell dispatcher, which is the complete,
stable ordering that includes every named gate. The ordered IDs are checked in at
`crates/autospec-cli/tests/fixtures/validation-cutover/catalog-v1.json`.

The initial inventory omitted the live `check_flag_sentinel_docs` gate. A complete
definition-versus-catalog audit on 2026-07-12 restored it immediately after
`check_required_files`, matching its declaration order in the legacy executor.

`ValidationCatalog::standard()` has one deliberately non-executable catalog slot per
frozen ID. Every entry is currently required and non-independent; mode selection,
parallelism, direct Rust structural implementations, and explicit external commands are
reserved for later cutover tasks.

## Execution reachability

The catalog is intentionally broader than the shell's top-level execution sequence.
The legacy `main` invokes **133** unique top-level gates in **138** ordered call
occurrences. Fifteen definitions are
internal components: nine dynamic per-skill helpers (`check_lockstep`,
`check_frontmatter`, required-file, self-update, and metadata helpers) plus
six aggregating components (`team_personality_*`, `autospec_resume_structure`,
`autospec_supervisor_structure`, and `fab_container_dockerfile`).
`check_architecture_fitness_engine` is defined but never invoked. The direct Rust
catalog must keep all 149 symbols for ownership audit, but its executable plan must
select only the 138 top-level call occurrences.

## Baseline verification

- RED: `cargo test -p autospec-core --test validation_catalog catalog_has_one_owner_slot_for_every_frozen_gate -- --exact` failed before implementation because the catalog type and fixture were absent.
- GREEN: `cargo test -p autospec-core --test validation_catalog -- --nocapture` verifies fixture parity and rejects empty or duplicate catalog IDs.

## Completed cutover evidence

The cutover is complete. `autospec validate` builds a direct Rust plan from the
frozen catalog, executes it with typed process definitions, and emits schema-2 results.
The direct full plan contains 138 ordered occurrences of 133 unique top-level IDs;
the fast plan contains 130 occurrences and omits Bats, Python, and install-test
batches while retaining structural checks.

- Catalog parity: `crates/autospec-cli/tests/validation_parity.rs` compares all 149
  frozen IDs, full/fast counts, scoped metadata, and fixed parallelism.
- Failure parity: `crates/autospec-core/tests/validation_runner.rs` covers required,
  optional, missing-tool, signal, ordering, and bounded-scheduler outcomes.
- Scoped execution: `--changed` and `--since` query Git with the requested base. The
  current catalog has no safe narrow global owners, so scoped plans retain all global
  top-level checks rather than silently skipping coverage.
- Removal audit: the direct CLI test rejects tracked references to the deleted shell
  dispatcher and every recursion environment variable; runtime audit fixtures no
  longer contain the retired script.
- Legacy cleanup: the dispatcher, affected-set helper, wrapper-only fixtures, and
  shell-validator suites are deleted. All live callers now invoke `autospec validate`.

Final commands and results are recorded with the implementation commit:

```text
cargo fmt --all --check                                  # pass
cargo test --workspace                                    # pass
cargo clippy --workspace --all-targets -- -D warnings     # pass
cargo run -q -p autospec-cli -- validate --fast --json    # 130 passed
cargo run -q -p autospec-cli -- validate --json           # 138 passed
git diff --check                                          # pass
```
