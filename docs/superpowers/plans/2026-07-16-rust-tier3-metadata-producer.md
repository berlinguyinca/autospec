# Rust Tier 3 Metadata Producer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the Rust-owned, receipt-backed Tier 3 metadata foundation without reusing shell workstreams or treating unavailable metadata as dry.

**Architecture:** A pure core validates injected architecture, coverage, and debt evidence, then exposes only evaluator-sealed canonical documents. The CLI records the checked-in disabled policy in production and tests injected complete or failed scans through sealed Tier 3 receipt/replay recovery.

**Tech Stack:** Rust standard library, existing strict JSON parser, existing waterfall receipts/store, no new dependencies.

## Global Constraints

- Every new Tier 3 production or test source file stays at 450 lines or fewer.
- Pure core has no filesystem, environment, process, network, GitHub, queue,
  claim, label, branch, worktree, PR, foreground, or `WaterfallStore` authority.
- Do not call shell scripts, `autospec-explore`, the legacy fitness/debt/coverage
  workstreams, `gh`, `curl`, `omx`, or a model child.
- Production constructs only `Tier3Input::DisabledByCheckedInPolicy` with exact
  reason `tier3_metadata_disabled_by_checked_in_policy`.
- Persist evidence before a receipt and a receipt before the cursor; only
  `Exhausted(NoMetadataFindings)` advances Tier 3 to Tier 4.
- `NoMetadataFindings` is a new closed `DryReason` variant; update the
  no-work codec's fixed reason-count shape and exact-key tests with the core
  contract rather than aliasing it to a proposal-stage dry reason.
- Produced evidence is planning-only and never mutates GitHub or dispatches work.

---

### Task 1: Add the pure Tier 3 metadata funnel

**Files:**

- Modify: `crates/autospec-core/src/lib.rs`
- Modify: `crates/autospec-core/src/autonomous/no_work.rs`
- Modify: `crates/autospec-core/src/autonomous/no_work/codec.rs`
- Create: `crates/autospec-core/src/autonomous/tier3.rs`
- Create: `crates/autospec-core/src/autonomous/tier3/model.rs`
- Create: `crates/autospec-core/src/autonomous/tier3/evaluate.rs`
- Create: `crates/autospec-core/src/autonomous/tier3/evidence.rs`
- Create: `crates/autospec-core/tests/autonomous_tier3.rs`
- Modify: `crates/autospec-core/tests/autonomous_no_work.rs`

**Consumes:** `FunnelCounts` and strict JSON conventions.

**Produces:** `evaluate_tier3(Tier3Input)`, opaque `Tier3EvidenceDocuments`,
closed outcomes, and canonical in-memory documents.

- [ ] **Step 1: Write failing core tests**

Cover exact disabled policy, architecture→coverage→debt failure/missing
precedence, invalid paths/fields/order, conflicting duplicates, empty evidence,
rank order/cap, count invariants, partial failure prefixes, canonical JSON,
and production-source authority.

```rust
let result = evaluate_tier3(Tier3Input::Enabled {
    architecture: Tier3StageResult::Complete(architecture()),
    coverage: Tier3StageResult::Complete(coverage()),
    debt: Tier3StageResult::Complete(debt()),
})?;
assert_eq!(result.observation().expect("complete").funnel.ranked, 1);
```

- [ ] **Step 2: Run the red proof**

Run: `cargo test -p autospec-core --test autonomous_tier3`

Expected: FAIL because `autonomous::tier3` does not exist.

- [ ] **Step 3: Define the closed model and evaluator**

Add `pub mod tier3;` to the inline autonomous module. Define closed stages
`Architecture`, `Coverage`, `Debt`, and `Ranking`; kinds, severities, failure
codes, stage results, adapter evidence, findings,
partial evidence, evaluation, and opaque document view. Validate sorted unique
stage records and rank by `(severity, rule_id, path, line, message)`, capped at
ten. Bind injected failure stages to their enclosing slot and preserve only
validated predecessors.

```rust
pub fn evaluate_tier3(input: Tier3Input) -> Result<Tier3Evaluation, Tier3Failure>;
pub const DISABLED_REASON: &str = "tier3_metadata_disabled_by_checked_in_policy";
pub const TIER3_RANK_LIMIT: u64 = 10;
```

Extend the closed no-work `DryReason` set with
`NoMetadataFindings` / `"no_metadata_findings"`, including codec count arrays,
strict exact-key parsing, and its focused state round-trip test. This is the
only dry outcome a complete empty Tier 3 metadata scan may produce.

- [ ] **Step 4: Render opaque canonical documents**

Expose documents only from an evaluated observation or sealed failure. Render
architecture, coverage, debt, findings, and failure JSON with exact schema/kind
and predecessor digest fields. Raw adapter structs must have no public renderer.

- [ ] **Step 5: Run green core proof**

Run: `cargo test -p autospec-core --test autonomous_tier3`

Expected: PASS.

- [ ] **Step 6: Commit the pure funnel**

```bash
git add crates/autospec-core/src/lib.rs \
  crates/autospec-core/src/autonomous/no_work.rs \
  crates/autospec-core/src/autonomous/no_work/codec.rs \
  crates/autospec-core/src/autonomous/tier3.rs \
  crates/autospec-core/src/autonomous/tier3 \
  crates/autospec-core/tests/autonomous_tier3.rs \
  crates/autospec-core/tests/autonomous_no_work.rs
git commit -m "feat: model native Tier 3 metadata evidence"
```

Use Lore trailers recording the no-shell/no-prose-evidence constraint and the
focused core proof.

### Task 2: Seal Tier 3 receipts and replay

**Files:**

- Modify: `crates/autospec-core/src/autonomous/waterfall.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/tier3.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/tier3_receipts.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall/evidence.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier2/canonical.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/waterfall/evidence/canonical.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier3.rs`
- Create: focused Tier 3 receipt/recovery test modules as needed

**Consumes:** Task 1 opaque documents, `TierReceipt`, `WaterfallState`, the
Tier 2 lock/replay pattern, and shared canonical lexical JSON validation.

**Produces:** private `Tier3Scan`/`Tier3Progress`, sealed artifacts, strict
replay validation, and Tier 4 advancement only for exhausted evidence.

- [ ] **Step 1: Write failing receipt/recovery tests**

Seed a valid Tier 3 cursor after Tier 1, Tier 1.5, and Tier 2 exhausted
receipts. Cover policy-only NotRun, empty complete evidence advancing Tier 4,
produced and failed outcomes retaining Tier 3, every exact failure prefix,
replay before cursor write, and tampered/missing/extra/misordered evidence.

```rust
assert_eq!(record_tier3(root.path(), REPO, Tier3Scan::NotRun)?,
    Tier3Progress::NotRun(DISABLED_REASON.to_string()));
assert_eq!(load_state(&root)?.current_tier(), NoWorkTier::Tier3);
```

- [ ] **Step 2: Run the red proof**

Run: `cargo test -p autospec-cli --bin autospec tier3`

Expected: FAIL because Tier 3 modules and artifacts do not exist.

- [ ] **Step 3: Add sealed artifact and replay verification**

Add `Tier3EvidenceArtifact::{Policy, Architecture, Coverage, Debt, Findings,
Failure}` with derived paths under `waterfall/<pass>/tier3/`. Reuse shared
canonical lexical JSON checks. Require exact ordered references, strict keys,
digest/link validation, bounded failure detail, stage-appropriate funnels, and
terminal status/funnel consistency. Extend state receipt verification for
completed Tier 3 receipts.

- [ ] **Step 4: Implement the disabled adapter and coordinator**

Keep Tier 3 modules private and `#[allow(dead_code)]` until foreground wiring
is separately specified. The only production input is disabled policy; tests
inject complete or sealed failed results. Persist documents in dependency order,
then the receipt, then call `WaterfallState::record_receipt` only for
`Exhausted(NoMetadataFindings)`.

```rust
pub(super) enum Tier3Scan { NotRun, Complete(Tier3Observation), Failed(Tier3Failure) }
pub(super) enum Tier3Progress { Pending, Advanced, Produced(u64), Failed(String), NotRun(String) }
```

- [ ] **Step 5: Run green receipt proof**

Run: `cargo test -p autospec-cli --bin autospec tier3`

Expected: PASS.

- [ ] **Step 6: Commit sealed receipts**

```bash
git add crates/autospec-core/src/autonomous/waterfall.rs \
  crates/autospec-cli/src/commands/autonomous.rs \
  crates/autospec-cli/src/commands/autonomous/tier3.rs \
  crates/autospec-cli/src/commands/autonomous/tier3_receipts.rs \
  crates/autospec-cli/src/commands/autonomous/waterfall.rs \
  crates/autospec-cli/src/commands/autonomous/waterfall/evidence.rs \
  crates/autospec-cli/src/commands/autonomous/waterfall/evidence \
  crates/autospec-cli/src/commands/autonomous/tier3_receipts*
git commit -m "feat: seal native Tier 3 metadata receipts"
```

Use Lore trailers recording evidence-before-receipt-before-cursor ordering.

### Task 3: Guard authority and publish Tier 3 foundation state

**Files:**

- Create: `crates/autospec-core/tests/autonomous_tier3_authority.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/tier3.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/tier3_receipts.rs`
- Modify: `docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md`

**Consumes:** Tasks 1–2.

**Produces:** mechanical no-shell/no-mutation proof and an accurate parent-plan
record of the disabled Tier 3 foundation.

- [ ] **Step 1: Write failing production-source guards**

Recursively scan Tier 3 pure modules using the established comment/literal-aware
matcher. Reject filesystem, environment, process, network, shell, legacy
workstreams, queue/claim/GitHub/branch/worktree/PR, and waterfall authority.
The CLI receipt coordinator may use only local `WaterfallStore` persistence;
reject direct I/O, foreground dispatch, remote, and mutation authority.

- [ ] **Step 2: Run the red guard proof**

Run: `cargo test -p autospec-core --test autonomous_tier3_authority`

Expected: FAIL until the Tier 3 production-source guard exists.

- [ ] **Step 3: Publish the cutover state**

Record that the typed Tier 3 metadata foundation and disabled receipt policy are
complete, while metadata-source activation, foreground wiring, Tier 4,
ideation, and legacy deletion remain separately gated.

- [ ] **Step 4: Run final verification**

```bash
cargo test -p autospec-core --test autonomous_tier3
cargo test -p autospec-core --test autonomous_tier3_authority
cargo test -p autospec-cli --bin autospec tier3
cargo test -p autospec-cli --bin autospec
cargo fmt --check
cargo clippy -p autospec-core --all-targets -- -D warnings
cargo clippy -p autospec-cli --bin autospec -- -D warnings
cargo run -q -p autospec-cli -- validate --fast --json
git diff --check
```

Expected: every command exits zero. Report unrelated workspace-wide diagnostics
instead of suppressing them.

- [ ] **Step 5: Commit authority and status**

```bash
git add crates/autospec-core/tests/autonomous_tier3_authority.rs \
  crates/autospec-cli/src/commands/autonomous/tier3.rs \
  crates/autospec-cli/src/commands/autonomous/tier3_receipts.rs \
  docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md
git commit -m "test: guard native Tier 3 metadata authority"
```

Use Lore trailers stating that metadata activation and legacy deletion are not
implicit fallbacks.
