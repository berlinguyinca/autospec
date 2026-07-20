# Rust Autonomous Policy Digest Completion Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete issue #1602 by binding every persisted Rust main-health cycle receipt to the exact effective repository policy and proving policy reload across foreground invocations.

**Architecture:** `MainHealthConfig` produces a canonical SHA-256 identity from the resolved health branch plus its sorted exact advisory-check set. The CLI computes that identity after branch resolution and appends it to `main-health-observations.jsonl` on every foreground invocation, including retained-state invocations, without changing the conductor-state schema.

**Tech Stack:** Rust 2021 standard library, the existing `autonomous::waterfall::sha256_hex` helper, fake-`gh` CLI integration tests, Cargo.

## Global Constraints

- Preserve the existing `.autospec/autonomous.yml` parser and precedence `--branch` > `main_health.branch` > GitHub default branch.
- Hash the effective resolved branch and sorted exact `ignore_checks`; do not hash raw YAML, comments, field order, or unrelated policy blocks.
- Persist `effective_policy_digest` in each native `main-health-observations.jsonl` receipt on every foreground invocation after successful config parsing and health evaluation.
- Malformed or unreadable repository configuration must fail before a health decision or receipt write.
- Do not change the strict `ConductorState` schema, add a dependency, read a global health environment variable, or invoke a shell/helper script.

---

### Task 1: Canonical effective main-health policy identity

**Files:**
- Modify: `crates/autospec-core/src/autonomous/config.rs`
- Modify: `crates/autospec-core/tests/autonomous_config.rs`

**Interfaces:**
- Produces: `MainHealthConfig::effective_policy_digest(&self, resolved_branch: &str) -> Result<String, String>`.
- Consumes: `autospec_core::autonomous::waterfall::sha256_hex` and the existing sorted `BTreeSet<String>`.

- [ ] **Step 1: Write failing digest and isolation tests**

Add tests that require:

```rust
let first = AutonomousConfig::parse(
    "main_health:\n  branch: main\n  ignore_checks:\n    - Unit Tests\n",
)?;
let reordered = AutonomousConfig::parse(
    "main_health:\n  ignore_checks:\n    - Unit Tests\n  branch: main\n",
)?;
let second = AutonomousConfig::parse(
    "main_health:\n  branch: release\n  ignore_checks:\n    - E2E Tests\n",
)?;

assert_eq!(
    first.main_health.effective_policy_digest("main")?,
    reordered.main_health.effective_policy_digest("main")?,
);
assert_ne!(
    first.main_health.effective_policy_digest("main")?,
    second.main_health.effective_policy_digest("release")?,
);
```

Also require an empty resolved branch to return an error and two independently parsed repository policies to retain distinct values and digests in one process.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p autospec-core --test autonomous_config
```

Expected: compilation fails because `effective_policy_digest` does not exist.

- [ ] **Step 3: Implement the canonical digest**

Use an unambiguous length-prefixed identity document:

```text
autospec-main-health-policy-v1
branch:<byte-length>:<resolved-branch>
ignore_check:<byte-length>:<check-name>
```

Append one `ignore_check` row per `BTreeSet` entry and return
`autospec-main-health-policy-v1:<sha256-hex>`. Reject an empty resolved branch.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run the Task 1 command again. Expected: all `autonomous_config` tests pass.

- [ ] **Step 5: Commit Task 1**

Stage the two Task 1 files and commit `feat: bind autonomous health policy identity` with Lore trailers.

### Task 2: Persist and reload the effective policy receipt

**Files:**
- Modify: `crates/autospec-core/src/autonomous/mainline_health.rs`
- Modify: `crates/autospec-core/tests/mainline_health.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/tests/autonomous_conductor_commands.rs`
- Modify: `docs/CONFIG_REFERENCE.md`
- Modify: `docs/runbooks/mainline-health-admission.md`

**Interfaces:**
- Consumes: `MainHealthConfig::effective_policy_digest(&resolved_branch)`.
- Produces: `MainlineHealth::to_json_with_policy_digest(repo, digest)` with an exact `effective_policy_digest` field.
- Persists: `<autonomous-state-scope>/main-health-observations.jsonl` for every evaluated foreground invocation.

- [ ] **Step 1: Write failing receipt and reload tests**

Core JSON coverage must require:

```rust
let json = health.to_json_with_policy_digest(
    "owner/repo",
    "autospec-main-health-policy-v1:abc123",
);
assert!(json.contains(
    "\"effective_policy_digest\":\"autospec-main-health-policy-v1:abc123\""
));
```

The CLI fixture must run foreground with one valid repository config, read `main-health-observations.jsonl`, rewrite the same repository's config to a second valid branch/check policy, run foreground again with retained conductor state, and require a second appended observation whose digest changes. It must also assert that malformed config leaves the prior observation bytes unchanged.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test -p autospec-core --test mainline_health
cargo test -p autospec-cli --test autonomous_conductor_commands
```

Expected: the core test cannot find `to_json_with_policy_digest`, and the integration test cannot find or cannot observe a changing `effective_policy_digest`.

- [ ] **Step 3: Implement receipt persistence**

Keep `MainlineHealth::to_json(repo)` unchanged for compatibility and add the digest-bearing serializer. In `run_foreground_with_lease`, compute the effective digest through `effective_main_health_policy_digest` after health evaluation. That helper hashes the resolved `health.branch` normally, but maps typed `default-branch-missing` evidence to the reserved invalid-ref identity `autospec:unresolved-default-branch` so an unresolved branch remains distinct from every valid Git branch without weakening `MainHealthConfig::effective_policy_digest`'s empty-branch rejection. Persist the digest-bearing main-health receipt before any retained conductor-state return. Pass the same digest through later admission persistence rather than recomputing it.

- [ ] **Step 4: Document the receipt contract**

Document that `effective_policy_digest` is canonical over the resolved branch and sorted exact ignored-check names, changes when either effective value changes, ignores YAML formatting/unrelated blocks, and is refreshed for each foreground invocation.

- [ ] **Step 5: Run focused tests and verify GREEN**

Run both Task 2 commands again. Expected: all tests pass, including retained-state reload and malformed-config no-write coverage.

- [ ] **Step 6: Commit Task 2**

Stage the six Task 2 files and commit `feat: persist effective autonomous policy receipts` with Lore trailers.

### Task 3: Authority and completion verification

**Files:**
- Modify only if a verified defect requires repair.

**Interfaces:**
- Verifies issue #1602 acceptance criteria and the #2076 Rust-only authority boundary.

- [ ] **Step 1: Run static authority checks**

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p autospec-core --all-targets -- -D warnings
cargo clippy -p autospec-cli --bin autospec -- -D warnings
```

Expected: all commands exit `0` without warnings.

- [ ] **Step 2: Run Rust integration verification**

Run:

```bash
cargo test -p autospec-core --test autonomous_config
cargo test -p autospec-core --test mainline_health
cargo test -p autospec-cli --test autonomous_conductor_commands -- --test-threads=1
```

Expected: every focused test passes.

- [ ] **Step 3: Run repository validation**

Run:

```bash
cargo run -q -p autospec-cli -- validate --fast --json
```

Expected: JSON status `passed`, with zero required failures.

- [ ] **Step 4: Review and close the issue through the PR**

Require a clean task review and whole-branch review. Create a PR whose body contains `Closes #1602`, the exact focused/full verification evidence, and the remaining typed-executor dependency under #2076.

## Plan self-review

- Spec coverage: the plan covers per-process isolation, repository reload, malformed-config fail-closed behavior, the missing policy digest, and shell-free authority.
- Placeholder scan: no deferred implementation steps or ambiguous test commands remain.
- Type consistency: both tasks use `MainHealthConfig::effective_policy_digest(&str)` and `MainlineHealth::to_json_with_policy_digest(&str, &str)` consistently.
- Scope: the plan does not add executor configuration, activate discovery tiers, or modify legacy scripts.
