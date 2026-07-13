# Rust validation cutover catalog baseline

Date: 2026-07-12
Source: `scripts/validate.sh` at `5fdc31f9`

## Frozen catalog

The version-1 catalog contains **149** uniquely named `check_*` gates. Its canonical
order is the declaration order in `scripts/validate.sh`, which is the only complete,
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
The legacy `main` invokes **142** top-level gates. Six definitions are internal
components reached through aggregating gates (`team_personality_*`,
`autospec_resume_structure`, `autospec_supervisor_structure`, and
`fab_container_dockerfile`), and `check_architecture_fitness_engine` is defined but
never invoked. The direct Rust catalog must keep all 149 symbols for ownership audit,
but its executable plan must select only the 142 top-level entries.

## Baseline verification

- RED: `cargo test -p autospec-core --test validation_catalog catalog_has_one_owner_slot_for_every_frozen_gate -- --exact` failed before implementation because the catalog type and fixture were absent.
- GREEN: `cargo test -p autospec-core --test validation_catalog -- --nocapture` verifies fixture parity and rejects empty or duplicate catalog IDs.

## Scope boundary

This baseline does not execute validation, invoke a shell, define tool commands, or
remove legacy validation code. The shell's runtime includes dynamic per-skill discovery;
future planning work must map that dynamic execution behavior onto these frozen IDs
and their recorded reachability without changing their catalog order.
