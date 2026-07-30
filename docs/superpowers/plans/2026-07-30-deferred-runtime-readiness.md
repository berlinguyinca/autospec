# Deferred Runtime Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an explicit deferred-readiness runtime mode while preserving fail-closed `CleanupFailed` state.

**Architecture:** `RuntimeMode` owns a typed readiness policy parsed identically by manifest versions 1 and 2. Direct provisioning branches on that policy, while reconciliation retains the existing fail-closed boundary for every `CleanupFailed` state because legacy mode commands can own untracked processes.

**Tech Stack:** Rust, `yaml-edit`, process-level CLI integration tests, Markdown runbook.

## Global Constraints

- Omitted readiness must preserve the current strict frontend-bind check.
- Deferred readiness must be explicit manifest data, never inferred from command text.
- `CleanupFailed` must continue to require explicit authenticated teardown.
- No new dependency is permitted.

---

### Task 1: Typed readiness and fail-closed recovery contract

**Files:**
- Modify: `crates/autospec-core/src/runtime_env/manifest.rs`
- Modify: `crates/autospec-core/src/runtime_env/manifest_v2.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env/lifecycle.rs`
- Test: `crates/autospec-cli/tests/runtime_resources.rs`
- Test: `crates/autospec-cli/tests/runtime_state_reconciliation.rs`
- Modify: `docs/runbooks/agent-runtime-manifest.md`

**Interfaces:**
- Consumes: manifest mode mappings and `RuntimeContext.mode`.
- Produces: internal `RuntimeReadiness::{Bound, Deferred}` and `RuntimeMode::requires_frontend_bind() -> bool`.
- Preserves: `PORT_BIND_HEALTH_RETRIES_EXHAUSTED` for bound modes and all existing teardown safety checks.

- [ ] **Step 1: Write failing parser tests inside `manifest.rs`**

Add a `#[cfg(test)]` module that parses both manifest versions and asserts hand-written expectations:

```rust
#[test]
fn readiness_defaults_to_bound_and_accepts_deferred_in_both_versions() {
    let legacy = RuntimeManifest::parse(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n",
    )
    .unwrap();
    assert!(legacy
        .selected_mode("local")
        .unwrap()
        .requires_frontend_bind());

    for version in ["1", "2"] {
        let source = format!(
            "version: {version}\nmodes:\n  local:\n    command: sh -c 'true'\n    readiness: deferred\n"
        );
        let manifest = RuntimeManifest::parse(&source).unwrap();
        assert!(!manifest
            .selected_mode("local")
            .unwrap()
            .requires_frontend_bind());
    }
}
```

Add a second test requiring `unsupported runtime readiness: eventual` for both versions.

- [ ] **Step 2: Run parser tests and verify RED**

Run:

```bash
cargo test -p autospec-core runtime_env::manifest::tests
```

Expected: compilation fails because `RuntimeMode::requires_frontend_bind` does not exist.

- [ ] **Step 3: Add failing deferred-activation integration coverage**

In `runtime_resources.rs`, write a manifest with:

```yaml
version: 1
modes:
  local:
    command: sh -c 'printf x >> attempts'
    readiness: deferred
```

Run `autospec runtime env up`, require exit code `0`, require `attempts` to equal exactly `x`, require owner lifecycle `Active`, and require both port claims to remain owned by the environment.

- [ ] **Step 4: Add fail-closed cleanup regression coverage**

In `runtime_state_reconciliation.rs`, start a deferred fixture, mutate its owner lifecycle to `CleanupFailed`, and run `up` again. Require exit `2`, `RUNTIME_LIFECYCLE_MISMATCH`, exactly one setup-command execution, no down-command execution, and preserved authoritative state.

- [ ] **Step 5: Run CLI tests and verify RED**

Run:

```bash
cargo test -p autospec-cli --test runtime_resources --test runtime_state_reconciliation
```

Expected: the deferred fixture exhausts frontend-bind retries; the cleanup regression already passes against the existing fail-closed behavior.

- [ ] **Step 6: Implement the typed parser contract**

Add:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RuntimeReadiness {
    #[default]
    Bound,
    Deferred,
}
```

Store it on `RuntimeMode`, expose an immutable accessor, initialize it to `Bound`, and parse only `bound` or `deferred`. Add `readiness` to the version-2 allowed mode keys.

- [ ] **Step 7: Implement deferred activation**

In direct provisioning, after the setup command succeeds:

```rust
if !context.mode.requires_frontend_bind() {
    return activate_direct(layout, &mut owner, ports, state);
}
```

Run this branch before the bind wait and before any retry. Do not change `reconcile_provisioning`; `CleanupFailed` remains an explicit-teardown state.

- [ ] **Step 8: Run focused tests and verify GREEN**

Run:

```bash
cargo test -p autospec-core runtime_env::manifest::tests
cargo test -p autospec-cli --test runtime_resources --test runtime_state_reconciliation
```

Expected: all focused tests pass with zero ignored tests.

- [ ] **Step 9: Document the public manifest field**

Update `docs/runbooks/agent-runtime-manifest.md` with:

```yaml
modes:
  playwright:
    command: sh -c 'true'
    readiness: deferred
```

State that `bound` is the default, `deferred` does not prove a listener exists, and deferred mode is for child-owned startup after the environment is activated.

- [ ] **Step 10: Run full verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
autospec validate
git diff --check
```

Expected: every command exits `0`; no test or validation check is skipped.

- [ ] **Step 11: Commit the implementation**

Stage only the eight issue-scoped files and commit:

```bash
git commit -m "fix: support deferred runtime readiness"
```

Include Lore trailers recording the strict default, the fail-closed cleanup boundary, focused and full verification, issue `#2808`, and the required OmX co-author.
