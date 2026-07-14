# Rust Runtime Consolidation Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish a deterministic Rust-owned inventory of remaining stateful runtime migration candidates and keep the Rust quality gate green.

**Architecture:** `autospec-core` continues to own the pure R0-R4 classifier. `autospec-cli` owns filesystem traversal and rendering for a new read-only `runtime audit` subcommand. The command never changes migration state; it only provides the exact inventory that later parity-gated cutovers consume.

**Tech Stack:** Rust 2021 standard library, existing `autospec_core::runtime_policy`, Cargo integration tests, repository shell validation.

## Global Constraints

- No new dependencies.
- Preserve `autospec runtime classify <PATH> [--json]` behavior exactly.
- Audit only `scripts/`, `skills/`, and `packages/`; skip `.git`, `target`, and `node_modules`.
- Sort paths before rendering so repeated audit runs have stable output.
- All production behavior starts with a failing CLI integration test.
- Shell wrappers remain compatibility entrypoints; this plan does not remove a fallback.

---

### Task 1: Restore the Rust quality baseline

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs:98-100`
- Test: `crates/autospec-cli/tests/cli_commands.rs`

**Interfaces:**
- Consumes: `parse_options(args: &[String]) -> Result<Options, String>`.
- Produces: the existing `Options` semantics with no clippy warning.

- [x] **Step 1: Confirm the quality-gate failure**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: FAIL with `clippy::field-reassign-with-default` at the `raw_args` assignment.

- [x] **Step 2: Replace default-plus-reassignment with the equivalent initializer**

```rust
let mut options = Options {
    raw_args: args.to_vec(),
    ..Options::default()
};
```

- [x] **Step 3: Verify behavior and quality**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

Expected: both commands pass; autonomous command tests remain green.

### Task 2: Add the failing runtime-audit CLI tests

**Files:**
- Modify: `crates/autospec-cli/tests/runtime_commands.rs`
- Create: `crates/autospec-cli/src/commands/runtime/audit.rs`
- Modify: `crates/autospec-cli/src/commands/runtime.rs`

**Interfaces:**
- Consumes: `autospec runtime audit [--root PATH] [--json]`.
- Produces: deterministic R0-R4 groups in text or JSON form, or a command error.

- [x] **Step 1: Write a JSON audit test against a static fixture root**

```rust
let output = autospec()
    .args(["runtime", "audit", "--root", fixture.to_str().unwrap(), "--json"])
    .output()
    .expect("runtime audit runs");
assert!(output.status.success());
let stdout = String::from_utf8_lossy(&output.stdout);
assert!(stdout.contains("\\\"R1\\\":[\\\"autospec validate\\\"]"));
assert!(!stdout.contains("target/ignored.rs"));
```

- [x] **Step 2: Write a missing-root error test**

```rust
let output = autospec()
    .args(["runtime", "audit", "--root", "/missing/runtime-audit-root"])
    .output()
    .expect("runtime audit starts");
assert!(!output.status.success());
assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
```

- [x] **Step 3: Verify the tests fail for the missing subcommand**

Run: `cargo test -p autospec-cli --test runtime_commands runtime_audit`

Expected: FAIL because `runtime audit` is not implemented.

### Task 3: Implement deterministic runtime audit

**Files:**
- Create: `crates/autospec-cli/src/commands/runtime/audit.rs`
- Modify: `crates/autospec-cli/src/commands/runtime.rs`

**Interfaces:**
- Consumes: `audit::run(args: &[String]) -> Result<(), String>` and `classify_path(path)`.
- Produces: text and JSON audit reports with all R0-R4 keys present.

- [x] **Step 1: Parse `--root` and `--json` without adding dependencies**

```rust
let mut root = std::env::current_dir().map_err(|error| error.to_string())?;
let mut json = false;
```

Accept only `--root <PATH>` and `--json`; return `unknown autospec runtime audit option: <arg>` for anything else.

- [x] **Step 2: Traverse known platform directories and classify files**

```rust
for relative_root in ["scripts", "skills", "packages"] {
    collect_files(&root.join(relative_root), &root, &mut paths)?;
}
paths.sort();
for path in paths {
    let verdict = classify_path(&path);
    groups.entry(verdict.class).or_default().push(verdict.path);
}
```

`collect_files` skips directories named `.git`, `target`, and `node_modules`, returns repository-relative slash-separated paths, and ignores missing optional platform roots.

- [x] **Step 3: Render deterministic text and JSON reports**

```rust
println!("{{\\\"command\\\":\\\"runtime audit\\\",\\\"root\\\":\\\"{}\\\",\\\"classes\\\":{{...}}}}", escape_json(root));
```

Use fixed class order `R0`, `R1`, `R2`, `R3`, `R4`. Text reports one heading and one path per line. JSON always includes every class as an array.

- [x] **Step 4: Run the focused test and full Rust checks**

Run: `cargo test -p autospec-cli --test runtime_commands runtime_audit && cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS.

### Task 4: Document and verify the compatibility boundary

**Files:**
- Modify: `docs/cli-reference.md`
- Test: `autospec validate`

**Interfaces:**
- Consumes: documented read-only `autospec runtime audit` command.
- Produces: an explicit statement that audit inventories candidates and does not migrate or execute them.

- [x] **Step 1: Add `autospec runtime audit --json` to the CLI reference**

```markdown
| `autospec runtime audit --json` | yes | read-only R0-R4 inventory of platform migration candidates |
```

- [x] **Step 2: Verify the full repository contract**

Run: `autospec validate --fast`

Expected: `validate: OK -- all validation checks passed.`

- [x] **Step 3: Commit the bounded slice**

```bash
git add crates/autospec-cli/src/commands crates/autospec-cli/tests/runtime_commands.rs docs/cli-reference.md docs/superpowers/specs/2026-07-12-rust-runtime-consolidation-completion-design.md docs/superpowers/plans/2026-07-12-rust-runtime-consolidation-completion.md
git commit -m "feat: inventory Rust runtime migration candidates"
```
