# Ready Queue Draft Admission Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop Rust ready-queue admission for unclassified or non-implementation issues.

**Architecture:** Add two early deterministic label gates in `plan_ready_queue_with_trusted_actors`. Reuse `QueueIssueView` blocked metadata so JSON output explains why a candidate was excluded.

**Tech Stack:** Rust standard library and existing queue tests.

## Global Constraints

- Do not add shell/Python behavior, Bats tests, dependencies, or a label-only workaround.
- `needs-classify` blocks before every downstream queue calculation.
- `auto-implement` is mandatory for ready admission.

---

### Task 1: Gate classification drafts in the Rust queue

**Files:**
- Modify: `crates/autospec-core/src/coordination/ready_queue.rs`
- Modify: `crates/autospec-core/tests/ready_queue.rs`
- Modify: `crates/autospec-cli/tests/queue_commands.rs`

**Interfaces:**
- Consumes: `RemoteIssue::has_label`, `QueueIssueView`, and `plan_ready_queue`.
- Produces: blocked reasons `needs_classify` and `missing_auto_implement`.

- [ ] **Step 1: Write failing core and CLI tests**

Add a core case with a safety-reviewed `needs-classify` candidate and a safety-reviewed candidate lacking `auto-implement`; assert both are blocked, their reasons are stable, and `ready_numbers()` is empty. Add a promoted case whose labels are only `auto-implement` and `safety:reviewed`; assert it is ready. Add CLI JSON coverage for the draft blocked reason.

- [ ] **Step 2: Run the focused tests to verify RED**

Run `cargo test -p autospec-core --test ready_queue` and `cargo test -p autospec-cli --test queue_commands`.

Expected: the new draft test fails because the queue currently admits the candidate.

- [ ] **Step 3: Add early label gates**

Before the existing `autospec:needs-human` and safety checks, add:

```rust
if view.issue.has_label("needs-classify") {
    view.reason = Some("needs_classify".to_string());
    view.blocked_label = Some("needs-classify".to_string());
    blocked.push(view);
    continue;
}
if !view.issue.has_label("auto-implement") {
    view.reason = Some("missing_auto_implement".to_string());
    blocked.push(view);
    continue;
}
```

- [ ] **Step 4: Verify GREEN and commit**

Run:

```bash
cargo fmt --all --check
cargo test --workspace
cargo run -p autospec-cli -- validate --fast
git diff --check
```

Expected: all commands exit 0 and the current draft issue is absent from `autospec queue ready` output. Commit a conventional Lore message.
