# Rust Executor Result Ingestion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a strict Rust executor-result receipt protocol that can record
owner-verified outcomes without reviving the shell autonomous waterfall.

**Architecture:** Keep the bare two-flag child call as the successful deferred
compatibility protocol. Route explicit receipt flags through a dedicated parser
and a typed claim recorder that verifies a fresh nonterminal run-state lease
and, for success, a branch-bound linked Closeout PR before appending immutable
evidence. The command prints JSON
before returning its stable process status so automation can distinguish a
recorded retryable or blocked result from malformed or stolen ownership.

**Tech Stack:** Rust workspace; `autospec_core::coordination::ConductorOutcome`;
existing strict claim JSON codecs; fake `gh` CLI integration fixture.

## Global Constraints

- No new dependency and no `bash`, `sh`, `omx`, `autospec-run`, or script invocation.
- A successful result requires a positive `--pr` whose open PR head equals the claimed branch; blocked and retryable require a nonempty `--reason`.
- Preserve `--repo OWNER/REPO --issue N` deferred output and exit `0` exactly.
- Never release, merge, or requeue a claim from this command.
- Explicit result exits: accepted `0`, retryable `10`, blocked `20`, malformed `2`, ownership-lost `3`.

---

### Task 1: Implement strict executor-result evidence and regression coverage

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs:87-115,1047-1119`
- Modify: `crates/autospec-cli/src/commands/mod.rs:79-90`
- Modify: `crates/autospec-cli/src/commands/claim.rs:533-602`
- Modify: `crates/autospec-core/src/claim/mod.rs:985-1000`
- Modify: `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

**Interfaces:**
- Consumes: compiled `autospec` binary, the existing `ForegroundFixture` fake `gh` transport, and the core conductor outcome type.
- Produces: strict receipt parser/output, owner-and-PR-verified claim result recording, and executable regression assertions.

- [ ] **Step 1: Add failing protocol tests**

Add helpers that invoke:

```rust
fixture.configured_command().args([
    "autonomous", "executor-result", "--repo", "test/repo", "--issue", "42",
    "--worker-id", "rust-foreground-conductor-1", "--branch", "autonomous/issue-42",
    "--outcome", "blocked", "--reason", "waiting-for-review",
])
```

Assert exact JSON `status`, `outcome`, and exit `20`; then read the fake
run-state comment and assert it is still `state=claimed`. Add negative tests
for a missing repo/issue, duplicated/unknown options, mixed `--pr`/`--reason`,
foreign worker, foreign branch, and a success PR without one Closeout report.
Add a success fixture PR that closes `#42` and has one `## Closeout report`,
then assert accepted JSON, exit `0`, and PR `17` in the run-state. Keep the
current deferred receipt assertion unchanged.

- [ ] **Step 2: Run the new test target and confirm failure**

Run: `cargo test -p autospec-cli --test autonomous_conductor_commands`

Expected: FAIL because `executor-result` does not recognize `--outcome` and
does not write or verify explicit result evidence.

#### Task 1 continuation: Parse and emit strict executor-result status

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs:87-115,1047-1119`

**Interfaces:**
- Consumes: explicit CLI flags from Task 1 and `claim::record_executor_result` from Task 3.
- Produces: `ExecutorResultInput`, `ExecutorResultStatus`, JSON output, and status-bearing `CommandFailure` results.

- [ ] **Step 1: Make autonomous command errors status-aware**

Change the autonomous public entrypoint to return `Result<(), CommandFailure>`
and remove the dispatcher's diagnostic re-wrapping for this command family.
Map all existing autonomous subcommands through `CommandFailure::diagnostic`;
dispatch `executor-result` directly so it can return its protocol exit code
without changing unrelated autonomous command diagnostics.

- [ ] **Step 2: Add a dedicated strict parser**

Define `ExecutorResultInput { repo, issue, worker_id, branch, outcome, pr,
reason }` and parse only `--repo`, `--issue`, `--worker-id`, `--branch`,
`--outcome`, `--pr`, and `--reason`. Detect repeated flags during parsing and
validate the exact combinations in the design. If the input consists only of
repo and issue, call the existing `executor_receipt_json` compatibility path.

- [ ] **Step 3: Emit one JSON result before its process status**

Use a compact formatter equivalent to:

```rust
println!("{{\"status\":\"blocked\",\"repo\":\"{}\",\"issue\":42,\"outcome\":\"blocked\",\"reason\":\"{}\"}}", repo, reason);
return Err(CommandFailure::status("", 20));
```

Use exit `0` for accepted success and compatibility deferred, `10` for a
recorded retryable result, `20` for a recorded blocked or unverified success,
`2` for malformed input, and `3` for ownership loss. Escape all JSON strings
with the existing `json_escape` helper.

- [ ] **Step 4: Run the focused tests**

Run: `cargo test -p autospec-cli --test autonomous_conductor_commands`

Expected: protocol syntax tests proceed to the claim-recording assertion.

#### Task 1 continuation: Add owner- and PR-verified claim result recording

**Files:**
- Modify: `crates/autospec-cli/src/commands/claim.rs:533-602`
- Modify: `crates/autospec-core/src/claim/mod.rs:985-1000`

**Interfaces:**
- Consumes: `ConductorOutcome`, repository/issue/worker/branch identity, and optional PR evidence.
- Produces: `record_executor_result` with typed accepted, evidence-blocked, and ownership-lost outcomes.

- [ ] **Step 1: Expose one reusable linked-PR predicate**

Add `is_reconcilable_pull_request(pull_request, issue)` in the core claim
module. Make `find_reconcilable_pull_request` call it so direct PR-number
verification and existing reconciliation use exactly the same closing-reference
and single-Closeout policy.

- [ ] **Step 2: Replace string-only executor recording with typed evidence**

Introduce a crate-visible result enum that distinguishes `Recorded`,
`EvidenceUnavailable`, and `OwnershipLost`. Require a fresh, nonterminal
owner-matched run-state first. For success, fetch open PRs and require the
supplied number to close the issue, have one Closeout, and have a matching
`headRefName`; return `EvidenceUnavailable` without mutation otherwise.
Append an immutable receipt with a generated ID, then re-read the receipt and
run-state so a takeover cannot be overwritten or accepted.

- [ ] **Step 3: Preserve the foreground compatibility caller**

Keep `record_executor_outcome` as a thin blocked deferred wrapper, or update
the foreground call to the typed API with `Blocked(DEFERRED_EXECUTOR_REASON)`.
The current bare child must still record `executor_deferred` and stay paused.

- [ ] **Step 4: Run the focused tests and core conductor tests**

Run: `cargo test -p autospec-cli --test autonomous_conductor_commands && cargo test -p autospec-core --test autonomous_conductor`

Expected: PASS; the core suite continues proving that `Succeeded` reaches scan
only through `DispatchRecorded` followed by `Reconciled`.

### Task 2: Document the Rust-only protocol and stale boundary

**Files:**
- Modify: `docs/cli-reference.md:20-100`
- Modify: `docs/workflows.md:70-90`
- Modify: `docs/runbooks/mainline-health-admission.md:1-12`

**Interfaces:**
- Consumes: finalized flag grammar and exit/status mapping from Tasks 2-3.
- Produces: operator documentation that has no legacy-shell foreground claim.

- [ ] **Step 1: Document the command table and protocol semantics**

Add `autospec autonomous executor-result` to the CLI table. State its required
explicit identity flags, outcome-specific evidence, JSON status values, stable
exit codes, and the special two-flag deferred compatibility receipt.

- [ ] **Step 2: Correct the workflow and health runbook boundary**

State that foreground mainline admission feeds the Rust foreground conductor,
not a legacy shell conductor. Explain that recorded success is evidence only
and does not merge/release a PR, while blocked/retryable outcomes retain the
lease.

- [ ] **Step 3: Run the documentation assertions and format check**

Run: `cargo fmt --all --check && git diff --check`

Expected: PASS with no whitespace or Rust formatting findings.

### Task 3: Run the release-quality verification set

**Files:**
- Modify: no additional files

**Interfaces:**
- Consumes: completed implementation and documentation.
- Produces: reproducible completion evidence.

- [ ] **Step 1: Run focused and workspace tests**

Run: `cargo test -p autospec-cli --test autonomous_conductor_commands && cargo test --workspace --quiet`

Expected: PASS.

- [ ] **Step 2: Run static validation**

Run: `cargo fmt --all --check && cargo clippy --workspace -- -D warnings && cargo run -q -p autospec-cli -- validate --fast && git diff --check`

Expected: PASS with no warnings or diff errors.

- [ ] **Step 3: Inspect the final scope**

Run: `git status --short && git diff --stat origin/main...HEAD`

Expected: only the files named in this plan and issue #2077 are changed.
