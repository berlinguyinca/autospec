# Autonomous Runtime Refresh and Accountability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every autonomous start execute the requested checkout's immutable runtime and require one durable, understandable GitHub accountability epic per conductor generation.

**Architecture:** A fail-closed shell preflight selects or builds an immutable digest-keyed runtime, then delegates to the Rust launcher. The launcher acquires its lifecycle lease, creates or adopts one run epic through a crash-safe local intent/journal, and spawns only after verifying the remote marker. Lifecycle events append locally before marker-bounded GitHub projection.

**Tech Stack:** Bash 3.2-compatible shell, Rust, GitHub CLI, JSON/JSONL, Bats, Mermaid Markdown.

**Spec:** `docs/specs/2026-08-13-autonomous-runtime-refresh-accountability-design.md`

## Global Constraints

- Accountability epic and local journal are mandatory core autonomous behavior with no bypass flag.
- Start/restart fail closed; read-only and stop commands remain available.
- Runtime generations are immutable and start executes the exact verified generation path.
- No new dependencies.
- Private state uses `0700` directories and `0600` files and rejects unsafe symlinks/ownership.
- GitHub text is typed, bounded, sanitized, and excludes secrets, absolute paths, raw argv, and raw logs.
- Use test-first red/green cycles and conventional Lore commits.

---

### Task 1: Immutable runtime identity and generation publisher

**Files:**
- Create: `scripts/autonomous-runtime-refresh.sh`
- Create: `scripts/autospec-runtime-install.sh`
- Modify: `install.sh`
- Test: `tests/autonomous/test_runtime_refresh.bats`
- Test: `tests/install/test_runtime_generations.bats`

**Interfaces:**
- Produces: `autonomous-runtime-refresh.sh check|ensure|identity --repo-dir DIR`; `autospec-runtime-install.sh --repo-dir DIR` prints the exact executable path.
- Produces: immutable `$HOME/.autospec/runtime-generations/<source-digest>/autospec` plus private receipt and atomic `current` pointer.

- [ ] Write failing tests for deterministic complete/batched identity, warm current fast path, immutable generation publication, moving-source rejection, concurrent repositories, signals/SIGKILL, lock identity, and exact-generation output.
- [ ] Run focused Bats and confirm failures are due to missing helpers/generation behavior.
- [ ] Port the reviewed strict receipt parser, then implement batched complete input hashing and pre/post-build identity equality.
- [ ] Implement private staged generation directories, verified sync, atomic pointer publication, process-identity lock, and narrow runtime-only installation.
- [ ] Run focused Bats, `bash -n`, ShellCheck, release build, and `git diff --check`.
- [ ] Commit with Lore trailers.

### Task 2: Fail-closed start/restart freshness boundary

**Files:**
- Modify: `scripts/lib/install-operator-wrappers.sh`
- Modify: `skills/autospec-autonomous/install.sh`
- Modify: `templates/skill-blocks/startup-self-update.md`
- Modify generated skill trio members/goldens through repository generators.
- Test: `tests/autonomous/test_runtime_start_preflight.bats`
- Test: `tests/install/test_autospec_bin_path.sh`

**Interfaces:**
- Consumes: Task 1 `ensure` exact executable path.
- Produces: start-family wrappers that preflight, gracefully drain stale live scopes, rebuild, and exec the exact runtime with original arguments.

- [ ] Write failing tests for current bypass, missing-helper fail-closed, stale stopped rebuild, stale live graceful drain, bounded timeout, argument preservation, concurrent starts, build failure retention, and read-only/stop bypass.
- [ ] Run focused tests and observe the missing preflight failures.
- [ ] Add one shared wrapper preflight and keep generated wrappers minimal/lock-step.
- [ ] Integrate existing stop/status contracts without force-killing or deleting ambiguous state.
- [ ] Regenerate mirrors/goldens and run wrapper/install/lock-step validation.
- [ ] Commit with Lore trailers.

### Task 3: Durable run-accountability domain and renderer

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/accountability.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/accountability/render.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/accountability/store.rs`
- Test: `crates/autospec-cli/tests/autonomous_accountability.rs`

**Interfaces:**
- Produces: `AccountabilityStore::open`, `begin_launch`, `append_event`, `render`, `ack_projection`, `status`.
- Produces: typed run identity, monotonic events, partial-tail recovery, projection revision/digest/high-watermark, and sanitized two-diagram Markdown.

- [ ] Write failing Rust tests for run identity transitions, private atomic state, monotonic event IDs, partial-tail recovery, crash boundaries, 48 KiB compaction, 25-node cap, sanitizer attacks, and comprehension snapshot.
- [ ] Run exact test target and confirm domain types/modules are missing.
- [ ] Implement minimal typed store and renderer in focused modules under file-size limits.
- [ ] Run exact tests, targeted rustfmt, Clippy for touched targets, and `git diff --check`.
- [ ] Commit with Lore trailers.

### Task 4: Exactly-once GitHub epic projection

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/accountability/github.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/supervisor.rs`
- Test: `crates/autospec-cli/tests/autonomous_accountability_github.rs`

**Interfaces:**
- Consumes: Task 3 launch intent/store/renderer.
- Produces: paginated marker reconciliation, single create attempt, `create_unknown`, lease renewal/loss handling, verified binding, optional Project assignment, and epic closure/succession.

- [ ] Write failing GH-stub tests for zero/one/multiple marker matches, delayed visibility/page boundary, ambiguous response, lease loss, crash after binding, follow/supervisor adoption, explicit restart succession, removed/closed epic, and optional Project failure.
- [ ] Run tests and verify they fail before integration.
- [ ] Implement GitHub adapter and launcher ordering: freshness → lease → verified epic → launch metadata → spawn.
- [ ] Add mandatory labels and exclude `autospec:run-accountability` from every autonomous/grooming queue.
- [ ] Run focused tests, queue tests, workspace check, and `git diff --check`.
- [ ] Commit with Lore trailers.

### Task 5: Lifecycle event wiring and operator visibility

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs` or a focused sibling module.
- Modify: `scripts/lib/autospec-loop.sh`
- Modify: `scripts/autospec-autonomous.sh`
- Test: `tests/autospec/test_conductor_wiring.bats`
- Test: `tests/autonomous/test_operator_conductor_list.bats`
- Test: `crates/autospec-cli/tests/autonomous_accountability_events.rs`

**Interfaces:**
- Consumes: Task 3 append/project APIs and Task 4 verified binding.
- Produces: event updates for selection, claim, PR, review, merged, failed, quarantined, parked, stopped, and completed; status/list accountability fields.

- [ ] Write failing tests proving local-first event ordering, merged-only completion wording, projection retry/degradation, mandatory journal failure blocking, and local-only status/list fields.
- [ ] Run tests and observe missing lifecycle wiring.
- [ ] Wire typed events at stable lifecycle boundaries, coalesce projections, and preserve existing daily digest separately.
- [ ] Add status/list JSON fields and human summaries without network calls.
- [ ] Run focused shell/Rust tests and `git diff --check`.
- [ ] Commit with Lore trailers.

### Task 6: Lock-step documentation, full verification, and real launch

**Files:**
- Modify: `AGENTS.md`
- Modify: `skills/autospec-autonomous/SKILL.md`
- Modify: `skills/autospec-autonomous/opencode/agent.md`
- Modify: `skills/autospec-autonomous/codex/prompt.md`
- Modify: `docs/cli-reference.md`
- Modify generated goldens.

**Interfaces:**
- Documents the non-optional invariant, epic schema/visuals, failure recovery, status fields, and Project option.

- [ ] Add failing structural/lock-step checks for the mandatory run-epic contract and no-bypass rule.
- [ ] Update canonical docs and generate mirrors/goldens.
- [ ] Run `cargo test --workspace`, Clippy, release build, `autospec validate`, all focused Bats, ShellCheck, `bash -n`, workflow lint, and `git diff --check`.
- [ ] Dispatch independent code/security reviewers; fix every important finding with new tests/commits.
- [ ] Push a ready PR, wait for all required CI, admin-merge under repository authority, and verify issue #3135 final projection.
- [ ] Reinstall merged Autospec and launch one fresh conductor; verify exactly one new epic exists before the conductor PID and its body contains both Mermaid diagrams and concise What/Why/Evidence entries.
