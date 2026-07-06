# Rust Core Workspace Recovery

## Version

V62

## Objective

Create an idiomatic Rust workspace for the AutoSpec core without deleting validated shell/Python/JS behavior.

## Scope

- Add `Cargo.toml` workspace.
- Add `crates/autospec-core` library.
- Add `crates/autospec-cli` binary.
- Add initial shared error type.
- Add smoke tests and CI-compatible cargo commands.

## Non-Goals

- Do not reimplement all existing shell workflows.
- Do not remove current `scripts/` or `skills/` surfaces.

## Dependencies

- `v61-recovery-public-launch-validation`

## Files To Create/Modify

- Create: `Cargo.toml`
- Create: `crates/autospec-core/Cargo.toml`
- Create: `crates/autospec-core/src/lib.rs`
- Create: `crates/autospec-core/src/error.rs`
- Create: `crates/autospec-cli/Cargo.toml`
- Create: `crates/autospec-cli/src/main.rs`
- Modify: `README.md`
- Modify: `docs/architecture.md`

## Implementation Steps

1. Add a Cargo workspace with resolver `2`.
2. Add `autospec-core` with modules `error`, `spec`, `graph`, `state`, `validation`, `evidence`.
3. Add `autospec-cli` using `clap` with only `--help` and `doctor` initially.
4. Define `AutospecError` using `thiserror`.
5. Add a smoke test that `autospec doctor --json` returns valid JSON.
6. Document Rust workspace boundaries.

## Acceptance Criteria

- [ ] `Cargo.toml` exists at repo root.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets --all-features` passes.
- [ ] `cargo test --all` passes.
- [ ] Existing `bash scripts/validate.sh --fast` still passes.

## Validation Commands

```bash
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test --all
bash scripts/validate.sh --fast
```

## Expected Outputs

- `target/debug/autospec --help` shows a CLI entry point.
- `target/debug/autospec doctor --json` emits JSON.

## Rollback/Handoff Notes

If Rust setup conflicts with existing packaging, keep Cargo workspace isolated under `crates/` and document the conflict in `.autospec/handoff/v62-rust-core-workspace-blocker.md`.
