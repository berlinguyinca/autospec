# Rust Foreground Waterfall Traversal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Rust foreground conductor resume exactly one sealed discovery tier per empty-queue cycle through Tier 1.5–4, while retaining every produced, failed, blocked, pending, or disabled outcome and keeping live Tier 2–4 activation out of scope.

**Architecture:** A small `foreground_waterfall` dispatcher reads the cursor through a policy-aware store under the current resilience lease, invokes only the cursor's producer, and maps each tier-specific progress enum into one closed foreground result. Each receipt coordinator re-verifies and holds the lease immediately before acquiring the waterfall lock and writing evidence, receipt, or state. The existing approved design remains authoritative: no complete-pass/no-work write, ideation, queue mutation, executor dispatch, shell fallback, or legacy deletion occurs in this slice.

**Tech Stack:** Rust 2021, existing `autospec-core` and `autospec-cli` workspace crates, existing direct-argv `gh api --method GET` Tier 1.5 adapter, filesystem-backed sealed waterfall state, no new dependencies.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-16-rust-autonomous-waterfall-design.md` and `AGENTS.md`.
- Global lock order is resilience lease transaction first, then waterfall store lock.
- Run external/read-only producer collection outside locks; revalidate the lease before every local mutation.
- One foreground cycle executes at most one current tier; an `Advanced` result returns the next cursor for the next cycle.
- `Produced`, `Failed`, `Blocked`, `NotRun`, and lock contention retain the current cursor and never count as dry.
- Tier 2, Tier 3, and Tier 4 production adapters remain checked-in disabled-only.
- A Tier 4 config is replay trust context, not activation authority.
- Tier 1.5 uses a named `TIER15_OBSERVATION_BUDGET: usize = 5`; it must not reuse the lifetime issue budget or add a new config key.
- No GitHub mutation, issue admission, claim mutation, executor dispatch, `why-no-work.json`, ideation, shell fallback, or legacy deletion.
- Every new Rust source/test file stays at or below 450 lines.
- Use TDD, Conventional+Lore commits, no amend, no hook bypass, and no new dependencies.

---

### Task 1: Add trusted waterfall policy and fence every later-tier receipt write

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/waterfall_policy.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/waterfall_policy_tests.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall_coordinator.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/tier15_receipts.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/tier2_receipts.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/tier3_receipts.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/tier4_receipts.rs`

**Interfaces:**
- Produces: `WaterfallPolicy::from_config(&AutonomousConfig) -> Result<WaterfallPolicy, String>`.
- Produces: `WaterfallStore::acquire_with_policy(root, repo, &WaterfallPolicy) -> Result<StoreAcquisition, WaterfallStoreError>`.
- Produces: lease-fenced `record_tier15_with_lease`, `record_tier2_with_lease`, `record_tier3_with_lease`, and `record_tier4_with_lease` wrappers.
- Preserves: current unfenced recorders only as private/test-local helpers; production traversal cannot call them.

- [ ] **Step 1: Write policy replay and stale-lease regressions**

Add focused tests proving:

```rust
#[test]
fn configured_tier4_rollover_replays_before_next_tier_one_scan() {
    let mut fixture = WaterfallPolicyFixture::completed_tier4();
    let result = fixture.record_empty_tier_one_with_config(fixture.config().clone());
    assert_eq!(result, Ok(Tier1Progress::Advanced));
    assert_eq!(fixture.cursor(), NoWorkTier::Tier1_5);
    assert!(fixture.receipt(2, NoWorkTier::Tier1).is_some());
}

#[test]
fn mismatched_configured_tier4_policy_writes_no_cursor_or_evidence() {
    let mut fixture = WaterfallPolicyFixture::completed_tier4();
    let before = fixture.snapshot();
    let result = fixture.record_empty_tier_one_with_config(fixture.changed_config());
    assert!(result.unwrap_err().contains("trusted source policy"));
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn replaced_lease_cannot_record_any_later_tier() {
    for tier in [NoWorkTier::Tier1_5, NoWorkTier::Tier2, NoWorkTier::Tier3, NoWorkTier::Tier4] {
        let mut fixture = WaterfallPolicyFixture::at(tier);
        fixture.replace_lease_generation();
        let before = fixture.snapshot();
        assert!(fixture.record_current_disabled_or_empty().is_err());
        assert_eq!(fixture.snapshot(), before);
    }
}
```

The disabled/default policy must replay disabled Tier 4 history without source authority. A nonempty config must derive a stable schema-1 `Tier4SourcePolicy` identity and exact descriptor set, but must not enable retrieval.

- [ ] **Step 2: Run the focused tests and observe RED**

Run:

```bash
cargo test -p autospec-cli --bin autospec waterfall_policy -- --nocapture
cargo test -p autospec-cli --bin autospec replaced_lease_cannot_record_any_later_tier -- --nocapture
```

Expected: FAIL because normal Tier 1 replay uses a bare store and later-tier recorders do not verify the lease.

- [ ] **Step 3: Implement policy-aware acquisition and fenced wrappers**

Use this shape:

```rust
pub(super) struct WaterfallPolicy {
    tier4_source: Option<Tier4SourcePolicy>,
}

pub(super) fn record_tier2_with_lease(
    state_root: &Path,
    repo: &str,
    lease: &ConductorLease,
    scan: Tier2Scan,
) -> Result<Tier2Progress, String> {
    with_current_lifecycle_lease(lease, || record_tier2_fenced(state_root, repo, scan))
}
```

Apply the same wrapper pattern to Tier 1.5, Tier 3, and Tier 4. Tier 4 receives the exact optional policy derived once from `AutonomousConfig`. Never acquire `WaterfallStore` before `with_current_lifecycle_lease`.

- [ ] **Step 4: Run focused and existing receipt recovery tests**

Run:

```bash
cargo test -p autospec-cli --bin autospec waterfall_policy -- --nocapture
cargo test -p autospec-cli --bin autospec tier15_receipts -- --nocapture
cargo test -p autospec-cli --bin autospec tier2_receipts -- --nocapture
cargo test -p autospec-cli --bin autospec tier3_receipts -- --nocapture
cargo test -p autospec-cli --bin autospec tier4_receipts -- --nocapture
```

Expected: PASS; policy drift and stale leases create no new artifact, receipt, or cursor write.

- [ ] **Step 5: Commit**

```bash
git add crates/autospec-cli/src/commands/autonomous.rs \
  crates/autospec-cli/src/commands/autonomous/waterfall.rs \
  crates/autospec-cli/src/commands/autonomous/waterfall_coordinator.rs \
  crates/autospec-cli/src/commands/autonomous/waterfall_policy.rs \
  crates/autospec-cli/src/commands/autonomous/waterfall_policy_tests.rs \
  crates/autospec-cli/src/commands/autonomous/tier15_receipts.rs \
  crates/autospec-cli/src/commands/autonomous/tier2_receipts.rs \
  crates/autospec-cli/src/commands/autonomous/tier3_receipts.rs \
  crates/autospec-cli/src/commands/autonomous/tier4_receipts.rs
git commit -m "fix: fence trusted waterfall replay"
```

### Task 2: Add the closed one-tier foreground dispatcher

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/foreground_waterfall.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/foreground_waterfall_tests.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`

**Interfaces:**
- Consumes: `WaterfallPolicy` and the lease-fenced receipt entry points from Task 1.
- Produces:

```rust
pub(super) enum ForegroundWaterfallProgress {
    Pending { tier: NoWorkTier },
    Produced { tier: NoWorkTier, count: u64 },
    Failed { tier: NoWorkTier, reason: String },
    Blocked { tier: NoWorkTier, reason: String },
    NotRun { tier: NoWorkTier, reason: String },
}

pub(super) fn run_one_tier(
    state_root: &Path,
    repo: &str,
    lease: &ConductorLease,
    config: &AutonomousConfig,
    tier1_evidence: Tier1QueueEvidence<'_>,
) -> Result<ForegroundWaterfallProgress, String>;
```

- [ ] **Step 1: Write closed-order and retention tests**

Use an injected private runner seam to prove one invocation per cycle:

```rust
#[test]
fn driver_runs_only_the_current_tier_and_returns_the_next_cursor() {
    let fixture = DriverFixture::at(NoWorkTier::Tier1);
    let (progress, calls) = fixture.run_with_outcomes([(NoWorkTier::Tier1, InjectedProgress::Advanced)]);
    assert_eq!(progress, ForegroundWaterfallProgress::Pending { tier: NoWorkTier::Tier1_5 });
    assert_eq!(calls, vec![NoWorkTier::Tier1]);
}

#[test]
fn driver_retains_pending_produced_failed_blocked_and_not_run() {
    for outcome in DriverFixture::retained_outcomes() {
        let fixture = DriverFixture::at(NoWorkTier::Tier2);
        let (progress, calls) = fixture.run_with_outcomes([(NoWorkTier::Tier2, outcome.clone())]);
        assert_eq!(progress, outcome.expected_progress(NoWorkTier::Tier2));
        assert_eq!(calls, vec![NoWorkTier::Tier2]);
        assert_eq!(fixture.cursor(), NoWorkTier::Tier2);
    }
}

#[test]
fn nonempty_tier4_config_remains_disabled_production_data() {
    let fixture = DriverFixture::at(NoWorkTier::Tier4).with_nonempty_tier4_config();
    let progress = fixture.run_production();
    assert!(matches!(progress, ForegroundWaterfallProgress::NotRun { tier: NoWorkTier::Tier4, .. }));
    assert_eq!(fixture.source_fetch_count(), 0);
}
```

Also prove the exact tier order `tier1`, `tier1_5`, `tier2`, `tier3`, `tier4`, and that the dispatcher has no no-work or ideation call.

- [ ] **Step 2: Run and observe RED**

Run:

```bash
cargo test -p autospec-cli --bin autospec foreground_waterfall -- --nocapture
```

Expected: FAIL because the dispatcher and closed progress type do not exist.

- [ ] **Step 3: Implement one-tier dispatch**

Define `const TIER15_OBSERVATION_BUDGET: usize = 5`. Dispatch by the policy-aware current cursor:

```rust
match tier {
    NoWorkTier::Tier1 => record_tier_one(state_root, repo, lease, tier1_evidence),
    NoWorkTier::Tier1_5 => {
        let scan = tier15::scan(repo, TIER15_OBSERVATION_BUDGET);
        record_tier15_with_lease(state_root, repo, lease, scan)
    }
    NoWorkTier::Tier2 => record_tier2_with_lease(
        state_root,
        repo,
        lease,
        tier2::disabled_by_checked_in_policy(),
    ),
    NoWorkTier::Tier3 => record_tier3_with_lease(
        state_root,
        repo,
        lease,
        tier3::disabled_by_checked_in_policy(),
    ),
    NoWorkTier::Tier4 => record_tier4_with_lease(
        state_root,
        repo,
        lease,
        tier4::disabled_by_checked_in_policy(&config.tier4),
        policy.tier4_source().cloned(),
    ),
}
```

Map `Advanced` to `Pending { tier: reloaded_cursor }`. Lock contention is `Pending`, never `Blocked`. Do not loop and do not fabricate a blocked receipt when no producer supports one yet.

- [ ] **Step 4: Run focused tests and line cap**

Run:

```bash
cargo test -p autospec-cli --bin autospec foreground_waterfall -- --nocapture
test "$(wc -l < crates/autospec-cli/src/commands/autonomous/foreground_waterfall.rs)" -le 450
test "$(wc -l < crates/autospec-cli/src/commands/autonomous/foreground_waterfall_tests.rs)" -le 450
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/autospec-cli/src/commands/autonomous.rs \
  crates/autospec-cli/src/commands/autonomous/foreground_waterfall.rs \
  crates/autospec-cli/src/commands/autonomous/foreground_waterfall_tests.rs
git commit -m "feat: resume one native discovery tier"
```

### Task 3: Wire empty foreground cycles through the dispatcher

**Files:**
- Create: `crates/autospec-cli/tests/autonomous_foreground_waterfall_commands.rs`
- Create: `crates/autospec-cli/tests/support/foreground_waterfall_fixture.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`

**Interfaces:**
- Consumes: `run_one_tier` from Task 2.
- Preserves: the existing `ConductorState` at `Scan`; no produced Tier 1.5 candidate is admitted in this gate.

- [ ] **Step 1: Write end-to-end foreground regressions**

Add isolated fake-`gh` fixtures with exact tests:

```rust
#[test]
fn repeated_empty_foreground_cycles_reach_and_retain_disabled_tier2() {
    let fixture = ForegroundWaterfallFixture::empty_repository();
    fixture.run_foreground_three_times().assert_success();
    assert_eq!(fixture.cursor(), NoWorkTier::Tier2);
    assert_eq!(fixture.receipt_status(NoWorkTier::Tier2), "not_run");
    assert!(!fixture.tier_directory_exists(NoWorkTier::Tier3));
}

#[test]
fn tier15_produced_retains_cursor_without_claim_or_executor() {
    let fixture = ForegroundWaterfallFixture::with_clear_tier15_candidate();
    fixture.run_until_tier15().assert_success();
    assert_eq!(fixture.cursor(), NoWorkTier::Tier1_5);
    assert_eq!(fixture.claim_mutations(), 0);
    assert_eq!(fixture.executor_launches(), 0);
}

#[test]
fn tier15_read_failure_is_sealed_and_never_dry() {
    let fixture = ForegroundWaterfallFixture::with_tier15_page_failure();
    fixture.run_until_tier15().assert_success();
    assert_eq!(fixture.cursor(), NoWorkTier::Tier1_5);
    assert_eq!(fixture.receipt_status(NoWorkTier::Tier1_5), "failed");
    assert!(!fixture.why_no_work_exists());
}

#[test]
fn newly_ready_work_preempts_a_retained_waterfall_cursor() {
    let fixture = ForegroundWaterfallFixture::retained_at_tier2_with_ready_issue();
    fixture.run_foreground_once().assert_success();
    assert_eq!(fixture.safety_reviews(), vec![42]);
    assert_eq!(fixture.tier2_record_attempts(), 0);
}
```

Assert exact receipts and cursor, absence of Tier 3/4 artifacts, `why-no-work.json`, issue edits/comments, claim labels, executor markers, and shell invocation.

- [ ] **Step 2: Run and observe RED**

Run:

```bash
cargo test -p autospec-cli --test autonomous_foreground_waterfall_commands -- --nocapture
```

Expected: FAIL because foreground stops after Tier 1 and never calls the dispatcher for later cursors.

- [ ] **Step 3: Replace the Tier-1-only empty branch**

Keep the change in `autonomous.rs` small: on a genuinely empty repository queue, call `run_one_tier`; translate every progress variant into `(state, false)` while preserving `Scan`. A slice-empty observation still uses `ConductorEvent::ScanEmpty` and does not start/resume a repository waterfall. Ready work continues to transition through `ScanFoundWork` before any later-tier producer runs.

- [ ] **Step 4: Run integration and prior foreground suites**

Run:

```bash
cargo test -p autospec-cli --test autonomous_foreground_waterfall_commands -- --nocapture
cargo test -p autospec-cli --test autonomous_conductor_commands -- --nocapture
cargo test -p autospec-cli --test autonomous_resilience_commands -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/autospec-cli/src/commands/autonomous.rs \
  crates/autospec-cli/tests/autonomous_foreground_waterfall_commands.rs \
  crates/autospec-cli/tests/support/foreground_waterfall_fixture.rs
git commit -m "feat: traverse native discovery from foreground"
```

### Task 4: Seal authority, documentation, and release proof

**Files:**
- Create: `crates/autospec-core/tests/autonomous_foreground_waterfall_authority.rs`
- Modify: `docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md`
- Modify: `docs/cli-reference.md` only if observable foreground status text changes.
- Test: existing Tier 2, Tier 3, Tier 4 authority and CLI guard suites.

**Interfaces:**
- Consumes: completed traversal and policy seams.
- Produces: a static no-authority gate and an accurate cutover checklist; source activation remains unchecked.

- [ ] **Step 1: Write authority tests before documentation changes**

Prove recursively that the dispatcher and foreground wiring contain no `Command`, shell, legacy waterfall/drain, write-capable GitHub operation, queue admission, claim mutation, executor dispatch, `NoWorkState::record`, `why-no-work.json`, or ideation authority. Prove Tier 4 config reaches only policy trust and the disabled adapter.

- [ ] **Step 2: Run and observe RED**

Run:

```bash
cargo test -p autospec-core --test autonomous_foreground_waterfall_authority -- --nocapture
```

Expected: FAIL until the new dispatcher is inventoried and its closed operation counts are encoded.

- [ ] **Step 3: Update the governing checklist accurately**

Mark foreground cursor traversal complete, but keep Tier 1.5 mutation/admission, Tier 2/3/4 activation, complete-pass recording, ideation, executor/premerge parity, installer migration, final audit, and legacy deletion unchecked. State explicitly that current production traversal stops at Tier 2 `NotRun`.

- [ ] **Step 4: Run the complete gate set**

Run:

```bash
cargo fmt --all -- --check
cargo test -p autospec-core --test autonomous_foreground_waterfall_authority
cargo test -p autospec-core --test autonomous_tier2_authority
cargo test -p autospec-core --test autonomous_tier3_authority
cargo test -p autospec-core --test autonomous_tier4_authority
cargo test -p autospec-cli
cargo clippy -p autospec-core --all-targets -- -D warnings
cargo clippy -p autospec-cli --bin autospec -- -D warnings
cargo run -q -p autospec-cli -- validate --fast --json
git diff --check origin/main..HEAD
```

Expected: every command passes; native fast validation reports 132/132 or the updated deterministic catalog total with zero failures.

- [ ] **Step 5: Commit**

```bash
git add crates/autospec-core/tests/autonomous_foreground_waterfall_authority.rs \
  docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md \
  docs/cli-reference.md
git commit -m "test: seal foreground waterfall authority"
```

## Self-Review

- Spec coverage: lease fencing, policy-aware replay, cursor order, retained non-dry outcomes, foreground precedence, and authority are covered. Complete-pass/no-work and activation remain explicitly excluded.
- Placeholder scan: no placeholder token or abbreviated call remains; every test step names the fixture inputs and exact assertions.
- Type consistency: Task 2 consumes Task 1's `WaterfallPolicy` and fenced recorders; Task 3 consumes Task 2's `run_one_tier`; Task 4 audits those exact surfaces.
- Scope: four independently reviewable tasks, each with a red/green cycle and a Lore commit.
