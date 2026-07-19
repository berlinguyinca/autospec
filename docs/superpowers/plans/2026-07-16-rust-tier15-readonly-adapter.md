# Rust Tier 1.5 Read-only Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan task-by-task.

**Goal:** Read every open and closed GitHub issue through a Rust-owned, read-only boundary, turn the sealed Tier 1.5 observation into a durable waterfall receipt, and never promote or mutate an issue.

**Architecture:** `autospec_core::autonomous::tier15` remains the pure selector. A CLI-local adapter uses direct `gh api --method GET` REST page reads, projects only the strict `RemoteIssuePage` shape, and fails closed on every command, parse, pagination, or evidence error. A coordinator persists immutable observation/failure evidence and its receipt before advancing only an exhausted Tier 1.5 cursor.

**Tech Stack:** Rust standard library, existing `RemoteIssuePage` parser, existing SHA-256-sealed waterfall store. No dependency additions.

## Global Constraints

- Never reuse `queue::list_issues`, queue planning, safety review, claim, legacy promoter/classifier, shell snippets, or write-capable GitHub commands.
- Use `gh api --method GET` only; project REST data to `RemoteIssuePage`, filter pull requests, retrieve every page for each state, and fail closed on a repeated/overflowed/invalid page.
- Do not use `gh issue edit`, `gh issue comment`, `gh label`, `gh api --method` other than `GET`, GraphQL mutations, process shells, label/body/template writes, or foreground integration.
- Incomplete, malformed, or unavailable open/closed evidence yields a sealed `TierStatus::Failed` receipt, never an exhausted decision.
- Persist evidence before its immutable receipt; persist the receipt before cursor advancement; an exhausted receipt advances to Tier 2, while produced or failed receipts retain the Tier 1.5 cursor and replay idempotently.
- Keep all source files below 500 lines, use TDD, add no dependencies, and preserve exact Tier 1.5 decision evidence.

---

### Task 1: Read-only paginated Tier 1.5 snapshot

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/tier15.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Test: module tests in `crates/autospec-cli/src/commands/autonomous/tier15.rs`

**Interfaces:**
- Consumes: `autospec_core::coordination::parse_remote_issue_page_json` and `RemoteIssue`.
- Produces: an internal `Tier15Scan::{Complete(Tier15Observation), Failed(String)}` based on `observe_tier15(Tier15Input)`.

- [ ] Write failing tests for a two-page open scan plus closed scan, pull-request filtering, malformed projected data, a failed later page, and a repeated/overflowed page cursor.
- [ ] Add an injected page-fetch helper that receives `(state, page)` and returns one strict projected page. It must call the pure observer only after both complete snapshots are available; any page error returns `Tier15Scan::Failed` with no partial dry result.
- [ ] Implement the production fetcher with exact argument vectors equivalent to `gh api --method GET repos/{repo}/issues?state={open|closed}&per_page=100&page={N} --jq <RemoteIssuePage projection>`. The projection must retain `raw_count`, exclude `.pull_request != null`, and include only number/title/body/labels/author/state.
- [ ] Reject an invalid repository identifier, page-number overflow, parse failure, command failure, and pages that cannot demonstrate termination. Do not import the queue module or run a shell.
- [ ] Run `cargo test -p autospec-cli --lib autonomous::tier15`, then format and clippy the CLI crate.

### Task 2: Sealed Tier 1.5 receipts and restart semantics

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall_coordinator.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall_tests.rs`
- Test: module tests in `tier15.rs` and coordinator/store tests

**Interfaces:**
- Consumes: `Tier15Scan`, `WaterfallStore`, `WaterfallState`, `TierReceipt`, and `TierStatus`.
- Produces: `Tier15Progress::{Pending, Advanced, Produced(u64), Failed(String)}`.

- [ ] Write failing tests for open/closed observation evidence persisted at `waterfall/{pass}/tier1_5/observation.json`, failure evidence at `waterfall/{pass}/tier1_5/read-failure.json`, digest tampering, receipt-before-cursor recovery, exhausted advancement to Tier 2, and produced/failed receipt replay without advancement.
- [ ] Generalize the existing Tier 1 evidence helper into a closed evidence-artifact boundary shared by Tier 1 and Tier 1.5. It must atomically persist immutable bytes, calculate `sha256_hex`, and verify each Tier 1/Tier 1.5 receipt's named evidence bytes during load/replay/state validation. Split the store into a submodule if retaining it in one file would exceed 500 lines.
- [ ] Map the full observation's deterministic JSON to one sealed evidence artifact. Use `TierStatus::Produced { count }` when the observation has candidates; otherwise use `TierStatus::Exhausted { reason: DryReason::NoProposalsGenerated }`. Map every scan error to `TierStatus::Failed { reason }` with sealed failure evidence.
- [ ] `record_tier15` may act only when the state cursor is `tier1_5`; its replay validates existing evidence before returning. It records and persists an exhausted receipt before updating state. A produced or failed receipt returns the typed outcome and leaves the state cursor intact. It may not write why-no-work, admit a candidate, or call a queue/claim operation.
- [ ] Add source-authority tests prohibiting the legacy promoter/classifier, queue/claim imports, shell/process use outside the narrow direct `gh api --method GET` fetcher, and every GitHub write verb. Run focused CLI/core tests, formatting, clippy, and `git diff --check`.

## Review checklist

- [ ] No partial page result can become `Exhausted`.
- [ ] Every receipt records either all decision evidence or a failure reason.
- [ ] Evidence digest, receipt digest, and cursor references are all reload-verified.
- [ ] The diff contains no promotion, label/body/comment, queue, claim, foreground, or legacy shell authority.
