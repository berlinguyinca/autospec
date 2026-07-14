# Rust Ready-Queue Control Plane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `autospec queue ready` the sole authority for selecting safe GitHub issues, then delete the legacy ready-queue and queue-only safety scripts.

**Architecture:** A pure Rust planner will consume typed GitHub issue, dependency, pull-request, and active-claim snapshots and return the existing `ready`, `blocked`, `claimed`, `conflicts`, `worker_cap`, and `batch` JSON contract. The CLI will obtain those snapshots with direct `gh` argument vectors, perform typed active-claim reconciliation/recovery before planning, and serialize the compatibility result. The issue-intent safety decision stays in the Rust core, while the old command-shaped shell linters disappear only after direct Rust tests cover their fixtures and callers.

**Tech Stack:** Rust 2021 standard library, existing `autospec_core::state::json` parser, existing `gh` CLI adapter style, Cargo tests, Bats regression tests; no new dependencies.

## Global Constraints

- Work only in `/private/tmp/wt-rust-control-plane-conductor`; the primary checkout remains read-only.
- Add no third-party dependency; use typed Rust models and the existing JSON parser.
- Preserve schema-1 claim comments, current queue JSON fields, sort order, and fail-closed GitHub evidence behavior.
- Preserve `AUTOSPEC_RUN_ONLY_ISSUES`, `AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS`, and configured `autonomous.concurrency.max_concurrent_repo_workers` precedence.
- A ready candidate must not bypass reviewed-safety markers, current-body safety evaluation, linked-PR checks, dependency gating, path-conflict detection, or serialization labels.
- Rust owns all GitHub label/comment lifecycle transitions; no live shell caller may mutate a claim label or clear a claim record.
- Keep `SKILL.md`, `codex/prompt.md`, and `opencode/agent.md` bodies lock-step; regenerate their goldens after edits.
- Retain historical docs/specs as historical evidence; remove only active authority paths and live references.

---

## File Structure

- Create: `crates/autospec-core/src/coordination/ready_queue.rs` — typed issue/PR snapshots, section-scoped parsing, dependency/cycle/path/serialization planning, and compatibility output model.
- Create: `crates/autospec-core/src/coordination/mod.rs` — public, narrow ready-queue exports.
- Create: `crates/autospec-core/tests/ready_queue.rs` — pure planner regressions for every non-network queue policy branch.
- Create: `crates/autospec-cli/src/commands/queue.rs` — `autospec queue ready` options, `gh`/GitHub adapters, typed recovery/reconciliation calls, JSON rendering, and configuration/environment resolution.
- Create: `crates/autospec-cli/tests/queue_commands.rs` — mocked-`gh` CLI compatibility and failure-mode tests.
- Modify: `crates/autospec-core/src/lib.rs` — export `coordination`.
- Modify: `crates/autospec-core/src/claim/mod.rs` — expose the safety verdict detail needed by queue output and add a typed stale-startup recovery decision (not a shell label swap).
- Modify: `crates/autospec-cli/src/commands/claim.rs` — add `claim state recover-stale-startup` with atomic label/state rollback semantics.
- Modify: `crates/autospec-cli/src/commands/mod.rs` — route the `queue` command and document it in root help.
- Modify: `crates/autospec-cli/tests/claim_commands.rs` — prove stale recovery’s preserve/release/rollback behavior.
- Modify: `skills/autospec-run/SKILL.md`, `skills/autospec-run/codex/prompt.md`, `skills/autospec-run/opencode/agent.md` — invoke `autospec queue ready`, never `list-ready-issues.sh`.
- Modify: `skills/autospec-run/scripts/autospec-run-status.sh`, `skills/autospec-fleet/scripts/fleet-status.sh`, `skills/autospec-fleet/scripts/fleet-run.sh`, `scripts/lib/autospec-loop.sh`, and `scripts/autonomous-waterfall.sh` — use the Rust command or accept an `AUTOSPEC_QUEUE_BIN` override, preserving their read-only fallback semantics.
- Modify: `skills/autospec-run/install.sh`, `install.sh`, and affected installer/smoke tests — stop installing deleted queue-only shell authorities.
- Delete: `skills/autospec-run/scripts/list-ready-issues.sh` and `skills/autospec-run/scripts/issue-safety-gate.sh` only after every live caller and its contract test reaches Rust.
- Modify: `scripts/lint-issue-safety.sh`, `scripts/apply-safety-review.sh`, `skills/autospec*/{SKILL.md,codex/prompt.md,opencode/agent.md}`, their generated goldens, and safety Bats tests to call `autospec lint issue safety` once the CLI has fixture parity.

### Task 1: Freeze the policy in pure Rust models

**Files:**
- Create: `crates/autospec-core/src/coordination/mod.rs`
- Create: `crates/autospec-core/src/coordination/ready_queue.rs`
- Create: `crates/autospec-core/tests/ready_queue.rs`
- Modify: `crates/autospec-core/src/lib.rs`
- Modify: `crates/autospec-core/src/claim/mod.rs`

**Interfaces:**
- Consumes: `ClaimSafetyInput` and `evaluate_claim_safety` from `autospec_core::claim`.
- Produces: `ReadyQueueInput`, `RemoteIssue`, `RemotePullRequest`, `ReadyQueuePlan`, and `plan_ready_queue(&ReadyQueueInput) -> ReadyQueuePlan`.

- [x] **Step 1: Write the failing pure planner tests**

```rust
#[test]
fn scopes_dependency_edges_to_the_dependencies_heading() {
    let plan = plan_ready_queue(&ReadyQueueInput::new(
        vec![issue(100, "## Shared contracts\n#100 depends on #101\n\n## Implementation outline\n- `src/a.rs`")],
        vec![], BTreeMap::new(), vec![], 3, 0,
    ));
    assert_eq!(plan.ready_numbers(), vec![100]);
}

#[test]
fn blocks_a_candidate_when_linked_pr_evidence_is_unavailable() {
    let input = ReadyQueueInput::with_pull_request_error(vec![safe_issue(1859)], "gh pr list failed");
    assert_eq!(plan_ready_queue(&input).blocked[0].reason, "linked_pr_evidence_unavailable");
}
```

- [x] **Step 2: Run the new tests to verify they fail**

Run: `cargo test -p autospec-core --test ready_queue`

Expected: FAIL because the `coordination` module and planner types do not exist.

- [x] **Step 3: Define the typed planner boundary**

```rust
pub struct ReadyQueueInput {
    pub candidates: Vec<RemoteIssue>,
    pub active: Vec<RemoteIssue>,
    pub dependencies: BTreeMap<u64, RemoteIssue>,
    pub pull_requests: Result<Vec<RemotePullRequest>, String>,
    pub batch_size: usize,
    pub max_repo_workers: usize,
}

pub fn plan_ready_queue(input: &ReadyQueueInput) -> ReadyQueuePlan;
```

Implement these exact policies in numeric issue order: `autospec:needs-human`; `evaluate_claim_safety`; nonterminal linked PR; `## Dependencies` parsing; epic/umbrella and child-tracker back-edge exemptions; recursive dependency-cycle detection; paths quoted in `## Implementation outline`; active and same-batch path conflicts; labels `reasoning:deep`, `priority:high`, `regression`, `audit`, and `release`; worker cap; and the existing first-serial-item batch rule.

- [x] **Step 4: Make planner output retain compatibility data**

```rust
pub struct QueueIssueView {
    pub issue: RemoteIssue,
    pub reason: Option<&'static str>,
    pub unmet_dependencies: Vec<u64>,
    pub non_blocking_refs: Vec<NonBlockingReference>,
    pub paths: Vec<String>,
    pub parallel_safe: Option<bool>,
}
```

Render labels as `{\"name\": ...}` and `author` as `{\"login\": ...}` in the CLI instead of discarding remote fields; this keeps existing `jq` consumers stable.

- [x] **Step 5: Run focused core tests**

Run: `cargo test -p autospec-core --test ready_queue --test claim_safety`

Expected: PASS, including dependency heading scoping, epics, tracker back-edges, cycles, linked PR error/open cases, safety failures, active/batch conflicts, worker cap, serialized first candidate, and issue-number filtering.

- [x] **Step 6: Commit the pure planner**

```bash
git add crates/autospec-core/src/coordination crates/autospec-core/src/lib.rs crates/autospec-core/src/claim/mod.rs crates/autospec-core/tests/ready_queue.rs crates/autospec-core/tests/claim_safety.rs
git commit -m "feat: model ready queue policy in Rust"
```

### Task 2: Add CLI adapters and typed stale-startup recovery

**Files:**
- Create: `crates/autospec-cli/src/commands/queue.rs`
- Create: `crates/autospec-cli/tests/queue_commands.rs`
- Modify: `crates/autospec-cli/src/commands/mod.rs`
- Modify: `crates/autospec-cli/src/commands/claim.rs`
- Modify: `crates/autospec-cli/tests/claim_commands.rs`

**Interfaces:**
- Consumes: `plan_ready_queue`, `autospec claim state reconcile-linked-pr`, and `autospec claim state recover-stale-startup`.
- Produces: `autospec queue ready [--repo OWNER/REPO] [--batch-size N] [--only-issues N ...]` with a single JSON document and exit `0`; option/remote errors exit `2`.

- [x] **Step 1: Write failing mocked-`gh` command tests**

```rust
#[test]
fn queue_ready_emits_the_legacy_top_level_shape() {
    let output = fixture.command(["queue", "ready", "--repo", "test/repo", "--batch-size", "2"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"ready\":"));
    assert!(stdout.contains("\"worker_cap\":"));
    assert!(stdout.contains("\"batch\":"));
}
```

- [x] **Step 2: Run the new tests to verify they fail**

Run: `cargo test -p autospec-cli --test queue_commands`

Expected: FAIL because `autospec queue` is not routed.

- [x] **Step 3: Implement direct `gh` adapters without shell interpolation**

```rust
Command::new("gh")
    .args(["issue", "list", "--repo", &repo, "--state", "open", "--label", "auto-implement", "--limit", "200", "--json", "number,title,body,labels,author"])
    .output()
```

Parse remote JSON strictly, query only dependency targets, and make a failed or malformed `gh pr list` become the per-candidate reason `linked_pr_evidence_unavailable` rather than selecting a candidate optimistically.

- [x] **Step 4: Implement `claim state recover-stale-startup` before the queue planner runs**

```text
autospec claim state recover-stale-startup --issue <N> --repo OWNER/REPO [--timeout-seconds 300]
```

This command must retain the active claim when there is a heartbeat, a local/remote branch, a PR, a fresh server timestamp, unreadable state, or failed GitHub mutation. It may remove `in-progress-by-bot`, add `auto-implement`, and clear the state only after the stale/no-evidence predicate passes; if clear fails, restore the active label and return a non-zero status. The queue command reconciles linked PRs first, then invokes recovery once per active issue, then re-reads active issues before applying the worker cap.

- [x] **Step 5: Run focused CLI tests**

Run: `cargo test -p autospec-cli --test queue_commands --test claim_commands`

Expected: PASS, including root/queue help, malformed options, repo inference, JSON shape, stale recovery preserve/release/rollback, PR reconciliation before worker cap, and direct argument-vector safety.

- [x] **Step 6: Commit the CLI boundary**

```bash
git add crates/autospec-cli/src/commands crates/autospec-cli/tests
git commit -m "feat: add Rust ready queue and stale claim recovery"
```

### Task 3: Replace live queue callers and delete the queue authority

**Files:**
- Modify: `skills/autospec-run/SKILL.md`, `skills/autospec-run/codex/prompt.md`, `skills/autospec-run/opencode/agent.md`
- Modify: `skills/autospec-run/scripts/autospec-run-status.sh`
- Modify: `skills/autospec-fleet/scripts/fleet-status.sh`, `skills/autospec-fleet/scripts/fleet-run.sh`
- Modify: `scripts/lib/autospec-loop.sh`, `scripts/autonomous-waterfall.sh`
- Modify: `skills/autospec-run/install.sh`, `install.sh`
- Modify: `tests/autospec-run/test_list_ready_issues.bats`, `tests/unit/test_autospec_coordination_queue.bats`, `tests/unit/test_autospec_linked_pr_reconcile.bats`, `tests/unit/test_autospec_fleet_scheduler.bats`, `tests/unit/test_autospec_fleet_status_stop.bats`, `tests/autospec/test_conductor_wiring.bats`, `tests/autonomous/test_waterfall.bats`, `tests/smoke/test_install_all_skills.bats`
- Delete: `skills/autospec-run/scripts/list-ready-issues.sh`, `skills/autospec-run/scripts/issue-safety-gate.sh`

**Interfaces:**
- Consumes: `autospec queue ready --repo "$REPO" --batch-size "$BATCH"`.
- Produces: all old readiness consumers receive byte-valid JSON with the same fields and retain an injectable command path for isolated Bats fixtures.

- [ ] **Step 1: Convert one direct Bats contract to fail against the absent Rust queue**

```bash
run "$AUTOSPEC" queue ready --repo testorg/testrepo --batch-size 3
[ "$status" -eq 0 ]
run jq -r '.batch | map(.number) | join(",")' <<<"$output"
[ "$output" = "30,32" ]
```

- [ ] **Step 2: Update all live consumers to resolve one command**

```bash
AUTOSPEC_QUEUE_BIN="${AUTOSPEC_QUEUE_BIN:-${AUTOSPEC_BIN:-autospec}}"
queue_json="$("$AUTOSPEC_QUEUE_BIN" queue ready --repo "$repo" --batch-size "$batch_size")"
```

Never reintroduce `gh issue edit`, `claim state clear`, `issue-safety-gate.sh`, or `list-ready-issues.sh` in a caller. Keep `--list-ready-bin` as a backward-compatible flag only if it maps to a full queue-command argv; otherwise replace it with `--queue-bin` and update each fixture.

- [ ] **Step 3: Regenerate lock-step prompt mirrors and goldens**

Run:
```bash
bash scripts/derive-trio.sh --in-place skills/autospec-run
bash scripts/gen-skill-goldens.sh autospec-run
```

Expected: all three bodies name `autospec queue ready` and have identical generated content.

- [ ] **Step 4: Delete queue-only shell authorities and enforce their absence**

```bash
git rm skills/autospec-run/scripts/list-ready-issues.sh skills/autospec-run/scripts/issue-safety-gate.sh
rg -n 'list-ready-issues\.sh|issue-safety-gate\.sh' --glob '!docs/**' --glob '!docs/memory/**' --glob '!llms*.txt'
```

Expected: remaining occurrences are explicit negative installer assertions or history only; add a Rust/runtime validation assertion that rejects live references.

- [ ] **Step 5: Run the migrated queue regression suite**

Run:
```bash
bats --print-output-on-failure tests/autospec-run/test_list_ready_issues.bats tests/unit/test_autospec_coordination_queue.bats tests/unit/test_autospec_linked_pr_reconcile.bats tests/unit/test_autospec_fleet_scheduler.bats tests/unit/test_autospec_fleet_status_stop.bats tests/autospec/test_conductor_wiring.bats tests/autonomous/test_waterfall.bats tests/smoke/test_install_all_skills.bats
```

Expected: PASS; live GitHub E2E remains opt-in and is not run unless its environment flag is explicitly set.

- [ ] **Step 6: Commit caller cutover**

```bash
git add skills/autospec-run skills/autospec-fleet scripts tests crates/autospec-core/src/validation
git commit -m "refactor: remove shell ready queue authority"
```

### Task 4: Move the remaining issue-intent lint command to Rust

**Files:**
- Modify: `crates/autospec-core/src/claim/mod.rs`
- Modify: `crates/autospec-cli/src/commands/lint.rs`
- Modify: `crates/autospec-core/tests/claim_safety.rs`
- Modify: `crates/autospec-cli/tests/queue_commands.rs`
- Modify: `tests/unit/test_lint_issue_safety.bats`, `tests/autospec/apply-safety-review.bats`, `tests/unit/test_phase3_lint_integration.bats`
- Modify: `scripts/apply-safety-review.sh`, `skills/autospec/SKILL.md`, `skills/autospec/codex/prompt.md`, `skills/autospec/opencode/agent.md`, `skills/autospec-define/SKILL.md`, `skills/autospec-define/codex/prompt.md`, `skills/autospec-define/opencode/agent.md`
- Delete: `scripts/lint-issue-safety.sh`

**Interfaces:**
- Produces: `autospec lint issue safety [--json] [--actor LOGIN] [--title TITLE] <body-file>`.
- Compatibility: print `SAFETY_PASS`, `SAFETY_AMBIGUOUS`, or `SAFETY_BLOCK` plus deterministic `RULE_ID:` lines; return `0`, `1`, `2`, or `64` for usage errors.

- [ ] **Step 1: Convert every current safety fixture into direct Rust expectations**

```rust
#[test]
fn issue_safety_reports_ambiguous_with_machine_readable_rule_id() {
    let verdict = evaluate_issue_intent_policy("Clean old data", include_str!("../../../tests/fixtures/issue-safety/ambiguous-clean-data.md"), "");
    assert_eq!(verdict.decision, IssueSafetyDecision::Ambiguous);
    assert_eq!(verdict.findings[0].rule_id, "vague-data-cleanup");
}
```

- [ ] **Step 2: Run tests to establish the missing command behavior**

Run: `cargo test -p autospec-core --test claim_safety && cargo test -p autospec-cli --test queue_commands`

Expected: FAIL until `lint issue safety` parses its options and emits the compatibility payload.

- [ ] **Step 3: Reuse one Rust policy result for claim and lint surfaces**

```rust
pub struct IssueSafetyVerdict {
    pub decision: IssueSafetyDecision,
    pub findings: Vec<IssueSafetyFinding>,
    pub trusted: bool,
}

pub fn evaluate_issue_intent_policy(title: &str, body: &str, actor: &str) -> IssueSafetyVerdict;
```

`evaluate_claim_safety` consumes this result after removing the reviewed marker block. The lint CLI reads the body with `fs::read_to_string`, produces the legacy plain/JSON fields, and never runs a policy shell subprocess. If a configured custom expression cannot be represented by the dependency-free evaluator, return a fail-closed `invalid-policy-regex` finding instead of ignoring it.

- [ ] **Step 4: Replace safety-linter callers, regenerate both lock-step trios, and delete the shell file**

Run:
```bash
bash scripts/derive-trio.sh --in-place skills/autospec
bash scripts/derive-trio.sh --in-place skills/autospec-define
bash scripts/gen-skill-goldens.sh autospec autospec-define
git rm scripts/lint-issue-safety.sh
```

Expected: the safety review helper and planning skills invoke `autospec lint issue safety`, while no live source invokes `bash ...lint-issue-safety.sh`.

- [ ] **Step 5: Run safety parity checks**

Run: `bats --print-output-on-failure tests/unit/test_lint_issue_safety.bats tests/autospec/apply-safety-review.bats tests/unit/test_phase3_lint_integration.bats`

Expected: PASS, including safe, block, ambiguous, trusted-reset, stale-review, invalid policy, and JSON-mode cases.

- [ ] **Step 6: Commit lint authority removal**

```bash
git add crates scripts skills tests
git commit -m "refactor: move issue safety linting into Rust"
```

### Task 5: Final integration audit and verification

**Files:**
- Modify: `docs/cli-reference.md`, `docs/workflows.md`, and relevant installer/runtime validation files.
- Modify: `docs/superpowers/plans/2026-07-14-rust-ready-queue-control-plane.md`

- [ ] **Step 1: Document the new command and deleted script surface**

```markdown
autospec queue ready --repo OWNER/REPO --batch-size N
autospec lint issue safety --json --title TITLE BODY_FILE
```

Describe the output arrays, worker-cap object, fail-closed evidence behavior, and the active `AUTOSPEC_QUEUE_BIN` injection point.

- [ ] **Step 2: Add static reachability tests**

```rust
assert_no_live_reference("skills/autospec-run/scripts/list-ready-issues.sh");
assert_no_live_reference("skills/autospec-run/scripts/issue-safety-gate.sh");
assert_no_live_reference("scripts/lint-issue-safety.sh");
```

- [ ] **Step 3: Run repository-wide verification**

Run:
```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
target/debug/autospec validate
bash scripts/validate.sh
bash skills/autospec-run/validate.sh
bash scripts/derive-trio.sh skills/autospec-run --check
bash scripts/derive-trio.sh skills/autospec --check
bash scripts/derive-trio.sh skills/autospec-define --check
git diff --check
```

Expected: all commands exit `0`.

- [ ] **Step 4: Review the plan completion state and commit documentation/audit**

```bash
git add docs crates tests scripts skills
git commit -m "docs: record Rust ready queue cutover"
```

Record Lore trailers for each commit: `Constraint`, `Rejected`, `Confidence`, `Scope-risk`, `Directive`, `Tested`, and `Not-tested`.

## Plan self-review

- Spec coverage: Tasks 1–3 cover queue selection, stale recovery, GitHub adapters, all live callers, and deletion; Task 4 removes the shared issue-safety shell authority; Task 5 proves runtime reachability and documents the resulting interface.
- Placeholder scan: no task leaves an unbounded implementation instruction; each code-changing task names its files, exports, commands, and tests.
- Type consistency: `ReadyQueueInput`/`ReadyQueuePlan` are defined in Task 1 and consumed in Task 2; `recover-stale-startup` is defined in Task 2 before Task 3 calls it; `IssueSafetyVerdict` is defined in Task 4 before both the lint and claim surfaces use it.
