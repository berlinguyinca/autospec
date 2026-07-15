# Runtime Manifest Companion Fixtures Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add parseable, non-executing companion runtime-manifest fixtures and document their Rust-owned isolation contract.

**Architecture:** Two v1 YAML fixture files are parsed by the existing Rust `RuntimeManifest` test surface. A focused runbook makes generated broker values and the non-executing remote-isolation boundary explicit.

**Tech Stack:** Rust integration tests, constrained runtime-manifest YAML, Markdown documentation.

## Global Constraints

- The Rust control plane and `autospec runtime env` are the only runtime-manifest implementation authority.
- Do not add, restore, or invoke `scripts/agent-env.sh`, Bats, Python, a shell YAML parser, or a second manifest model.
- Fixtures contain no credentials, hosts, connection strings, provisioning, migration, deletion, deployment, or shell execution.
- Do not place `AGENT_FRONTEND_PORT`, `AGENT_BACKEND_PORT`, `AGENT_PUBLIC_URL`, `AUTOSPEC_PUBLIC_URL`, or `COMPOSE_PROJECT_NAME` in `modes.*.env`.

---

### Task 1: Add fixtures, Rust coverage, and the runbook

**Files:**
- Create: `tests/fixtures/runtime-manifests/lc-binbase-scheduler.yml`
- Create: `tests/fixtures/runtime-manifests/companion-stack.yml`
- Modify: `crates/autospec-core/tests/runtime_env.rs`
- Create: `docs/runbooks/agent-runtime-companion-stacks.md`

**Interfaces:**
- Consumes: `RuntimeManifest::parse(source: &str)` and `RuntimeManifest::selected_mode("auto")`.
- Produces: two v1 fixtures that parse and a user-facing Rust runtime contract.

- [ ] **Step 1: Write the failing fixture-parsing test**

Add this test before the fixture files exist:

```rust
#[test]
fn companion_runtime_manifest_fixtures_parse_and_select_defaults() {
    for (source, expected_mode) in [
        (
            include_str!("../../../tests/fixtures/runtime-manifests/lc-binbase-scheduler.yml"),
            "playwright-local",
        ),
        (
            include_str!("../../../tests/fixtures/runtime-manifests/companion-stack.yml"),
            "go-modules",
        ),
    ] {
        let manifest = RuntimeManifest::parse(source).expect("fixture parses");
        assert_eq!(manifest.selected_mode("auto").expect("default mode").name(), expected_mode);
    }
}
```

- [ ] **Step 2: Verify the test fails**

Run `cargo test -p autospec-core --test runtime_env companion_runtime_manifest_fixtures_parse_and_select_defaults`.

Expected: compilation fails because the first included fixture file is absent.

- [ ] **Step 3: Add exact data-only fixtures**

`tests/fixtures/runtime-manifests/lc-binbase-scheduler.yml`:

```yaml
version: 1
name: lc-binbase-scheduler
default_mode: playwright-local
modes:
  playwright-local:
    command: npm run dev
    env:
      E2E_USE_HARNESS: "1"
      RUNTIME_PROFILE: scheduler-playwright
```

`tests/fixtures/runtime-manifests/companion-stack.yml`:

```yaml
version: 1
name: companion-stacks
default_mode: go-modules
modes:
  go-modules:
    command: npm run dev
    env:
      RUNTIME_PROFILE: go-modules
  flasheic:
    command: npm run dev
    env:
      RUNTIME_PROFILE: flasheic
```

- [ ] **Step 4: Add the runbook contract**

Create `docs/runbooks/agent-runtime-companion-stacks.md`. It must name `lc-binbase-scheduler`, `go-modules`, and `flasheic`; direct users to `.autospec/runtime.yml` and `autospec runtime env`; state that the Rust broker generates the five reserved variables; and describe `agent_<environment-id>` only as a non-executing naming convention. It must prohibit credentials, hosts, connection strings, provisioning, migration, deletion, and deployment commands.

- [ ] **Step 5: Verify focused and scoped behavior**

Run `cargo test -p autospec-core --test runtime_env companion_runtime_manifest_fixtures_parse_and_select_defaults`, then `cargo test -p autospec-core --test runtime_env && cargo run -q -p autospec-cli -- validate --fast && cargo fmt --all --check && cargo clippy --workspace -- -D warnings && git diff --check`.

Expected: every command exits `0`.

### Task 2: Publish a gate-compliant change

**Files:**
- Modify: GitHub issue `#1996`
- Modify: pull-request description for the implementation branch

**Interfaces:**
- Consumes: issue-quality, issue-safety, implementation-lint, and claim-lease contracts.
- Produces: a safety-reviewed Rust-only issue and a PR Closeout report with reproducible proof.

- [ ] **Step 1: Rewrite and validate issue `#1996`**

List both fixtures, the Rust test, the runbook, and the design/plan artifacts in `## Implementation scope` and `## Implementation outline`. Place `### Primary smoke test (inner loop)` under `## Verification` before `## Tests required`, so the test-tier detector sees only the Rust test requirement. Run `cargo run -q -p autospec-cli -- lint issue <issue-body-path>` and `cargo run -q -p autospec-cli -- lint issue safety --actor berlinguyinca --title '<issue-title>' <issue-body-path>`.

Expected: quality exits `0` and safety prints `SAFETY_PASS`.

- [ ] **Step 2: Classify, claim, and gate the staged implementation**

Run `cargo run -q -p autospec-cli -- classify issue --repo berlinguyinca/autospec --issue 1996`; acquire the lease; then run `cargo run -q -p autospec-cli -- lint implementation --pre-commit --staged --issue 1996`.

Expected: deterministic model-fit labels, a safety review, an acquired lease, and zero staged findings.
