# Runtime manifest default-mode invariant

## Goal

Reject invalid typed runtime manifests during parsing when `default_mode` does not name a declared mode.

## Scope

- Move the existing typed manifest parser into `runtime_env/manifest.rs` so each Rust module remains below the implementation gate's file limit.
- Add one parse-time membership invariant in `RuntimeManifest::parse`.
- Add a Rust core regression for a missing declared default mode.
- Document the parse-time failure behavior.

## Non-goals

- Do not alter manifest lookup order, provisioning, shell commands, or compatibility wrappers.
- Do not introduce a second parser, dependency, shell/Python implementation, or Bats coverage.

## Verification

1. Add the regression test before implementation and observe the existing parse succeeds.
2. Run focused runtime-manifest tests, then formatting, full workspace tests, fast validation, Clippy, and diff checks.
