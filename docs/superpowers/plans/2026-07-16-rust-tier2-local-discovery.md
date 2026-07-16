# Rust Tier 2 Local Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the Rust-owned, evidence-backed Tier 2 local discovery foundation without retaining the legacy shell explorer or mistaking an unavailable generator or verifier for a dry discovery pass.

**Architecture:** A strict local collector returns only deterministic domain evidence. A pure `autospec_core::autonomous::tier2` funnel validates injected typed proposal and verifier results, then deduplicates, verifies, ranks, and renders canonical evidence. The CLI persists Tier 2 artifacts and receipts under the waterfall lock; its checked-in production policy is initially `NotRun`, while tests use injected complete and failed inputs to prove every receipt path.

**Tech Stack:** Rust standard library, existing `autospec_core` strict JSON and receipt primitives, existing local waterfall store; no new dependencies and no external process execution.

## Global Constraints

- Every new Task 5 production or test source file stays at 450 lines or fewer.
- Core Tier 2 and the strict collector have no filesystem writes, environment reads, process/network, GitHub, queue/claim, branch/worktree/PR, or `WaterfallStore` authority.
- Do not call `scan_specialists`, `autospec explore specialists`, the cache, an environment proposal stub, a shell, legacy explorer scripts, `gh`, `curl`, or `omx`.
- A disabled runner is exactly `NotRun { reason: "tier2_local_discovery_disabled_by_policy" }`; only a sealed `Exhausted` receipt advances the cursor.
- Persist every Tier 2 artifact before its receipt, persist the receipt before the cursor, and re-verify every reference during replay.
- A produced candidate is planning evidence only: no issue, label, queue, claim, branch, PR, or implementation action.

---

### Task 1: Extract strict no-cache local evidence collection

**Files:**

- Modify: `crates/autospec-core/src/explore/specialists.rs`
- Modify: `crates/autospec-core/src/explore/specialists/scan.rs`
- Create: `crates/autospec-core/src/explore/specialists/lexicon.rs`
- Create: `crates/autospec-core/src/explore/specialists/strict.rs`
- Modify: `crates/autospec-core/tests/explore_specialists.rs`

**Consumes:** existing `DetectedDomain` and `FileLineEvidence`.

**Produces:** a public, read-only `collect_strict_domains` API with deterministic local evidence or a typed Tier 2 collector failure. Cache-backed `ScanOptions` and `scan_specialists` retain their current behavior.

- [ ] **Step 1: Write failing strict-collector tests**

Add temporary-root tests for deterministic domain/evidence ordering, zero-domain validity, invalid UTF-8 selected manifests, unreadable selected inputs, root-escaping symlinks, evidence cap, and no cache/environment/write authority.

```rust
let evidence = collect_strict_domains(&StrictCollectorOptions::new(root.path()))?;
assert_eq!(evidence.collector_version, "strict-local-v1");
assert_eq!(evidence.domains[0].name, "trading");
assert_eq!(evidence.domains[0].evidence.len(), 8);
```

- [ ] **Step 2: Run the failing collector test**

Run: `cargo test -p autospec-core --test explore_specialists strict`

Expected: FAIL because `StrictCollectorOptions` and `collect_strict_domains` do not exist.

- [ ] **Step 3: Extract shared scanner logic**

Move the checked-in lexicon, manifest/document policy, skip policy, normalization, token matching, bounded record logic, and domain sorting from `scan.rs` to `lexicon.rs`. Keep `scan.rs` as the legacy compatibility adapter importing that logic; do not copy the lexicon.

```rust
pub(super) fn scan_line(relative: &str, line: usize, text: &str, hits: &mut [Vec<FileLineEvidence>]);
pub(super) fn ranked_domains(hits: Vec<Vec<FileLineEvidence>>) -> Vec<DetectedDomain>;
```

- [ ] **Step 4: Implement the strict collector**

Expose this exact API from `specialists.rs`:

```rust
pub struct StrictCollectorOptions { pub repo_dir: PathBuf, pub max_depth: usize }
pub fn collect_strict_domains(
    options: &StrictCollectorOptions,
) -> Result<Tier2CollectorEvidence, Tier2Failure>;
```

Canonicalize the root, require a directory, reject symlinks, canonicalize selected files/directories before use, retain only root-relative evidence paths, and map read/UTF-8/containment failures to closed collector codes. Use depth three, the existing eight-evidence and 120-character caps, and score-descending/name-ascending sorting. Do not derive `SuggestedSpecialist`.

- [ ] **Step 5: Run the focused collector proof**

Run: `cargo test -p autospec-core --test explore_specialists strict`

Expected: PASS, including existing cache-backed scanner tests.

- [ ] **Step 6: Commit the collector slice**

```bash
git add crates/autospec-core/src/explore/specialists.rs \
  crates/autospec-core/src/explore/specialists/scan.rs \
  crates/autospec-core/src/explore/specialists/lexicon.rs \
  crates/autospec-core/src/explore/specialists/strict.rs \
  crates/autospec-core/tests/explore_specialists.rs
git commit -m "feat: collect strict local Tier 2 evidence"
```

Use Lore trailers recording the no-cache/no-environment constraint and focused test command.

### Task 2: Add the pure typed Tier 2 funnel

**Files:**

- Modify: `crates/autospec-core/src/lib.rs`
- Create: `crates/autospec-core/src/autonomous/tier2.rs`
- Create: `crates/autospec-core/src/autonomous/tier2/model.rs`
- Create: `crates/autospec-core/src/autonomous/tier2/funnel.rs`
- Create: `crates/autospec-core/src/autonomous/tier2/evidence.rs`
- Create: `crates/autospec-core/tests/autonomous_tier2.rs`

**Consumes:** strict collector evidence, `FunnelCounts`, and `IDEATION_CANDIDATE_LIMIT`.

**Produces:** `evaluate_tier2(Tier2Input) -> Result<Tier2Evaluation, Tier2Failure>` and canonical in-memory documents.

- [ ] **Step 1: Write failing model/funnel tests**

Use complete typed fixtures instead of a process. Cover disabled policy, bounded-field violations, evidence outside the collector, zero proposals, normalization/dedup winner, duplicate conflict, duplicate/missing/unknown verdicts, all-refuted, rank order, cap, monotonic counts, and canonical JSON.

```rust
let result = evaluate_tier2(Tier2Input::Enabled {
    collector: Tier2StageResult::Complete(collector()),
    generator: Tier2StageResult::Complete(generated(vec![proposal("a"), proposal("b")])),
    verifier: Tier2StageResult::Complete(verdicts_survive(["a", "b"])),
})?;
assert_eq!(result.observation().funnel.ranked, 2);
```

- [ ] **Step 2: Run the failing funnel target**

Run: `cargo test -p autospec-core --test autonomous_tier2`

Expected: FAIL because the Tier 2 module does not exist.

- [ ] **Step 3: Define closed core types**

Add `pub mod tier2;` to the inline autonomous module in `lib.rs`. In `model.rs`, define `Tier2Input`, `Tier2StageResult`, collector/generated/verifier records, proposal/source/severity/complexity, verification, closed stage/failure code, observation, deduplication, ranked proposal, and evaluation.

```rust
pub const TIER2_SCHEMA: u64 = 1;
pub const TIER2_RANK_LIMIT: u64 = IDEATION_CANDIDATE_LIMIT;
pub const DISABLED_REASON: &str = "tier2_local_discovery_disabled_by_policy";
```

Validate nonempty bounded text, `confidence_millis <= 1000`, closed enum mappings, sorted unique collector evidence, exact collector-backed proposal evidence, and overflow-safe `FunnelCounts`.

- [ ] **Step 4: Implement deterministic evaluation and renderers**

Use a `BTreeMap`, reject conflicting candidate payloads, select higher integer score then lower severity rank then lower stable key, require one verdict per winner, and rank by severity ascending, score descending, stable key ascending.

```rust
let group_key = format!("{}\\0{}", proposal.source.as_str(), normalize_title(&proposal.title));
let score_quotient = proposal.confidence_millis as u64 / proposal.complexity.units();
```

`DisabledByCheckedInPolicy` returns only the exact `NotRun` reason. A stage failure stays typed; never replace it with an empty proposal list. Render schema-one one-line JSON from typed validated data only.

- [ ] **Step 5: Run the pure funnel proof**

Run: `cargo test -p autospec-core --test autonomous_tier2`

Expected: PASS.

- [ ] **Step 6: Commit the core funnel slice**

```bash
git add crates/autospec-core/src/lib.rs \
  crates/autospec-core/src/autonomous/tier2.rs \
  crates/autospec-core/src/autonomous/tier2 \
  crates/autospec-core/tests/autonomous_tier2.rs
git commit -m "feat: model the native Tier 2 discovery funnel"
```

Use Lore trailers prohibiting a lexicon hit from becoming defect proof.

### Task 3: Seal Tier 2 evidence and receipt recovery

**Files:**

- Modify: `crates/autospec-core/src/autonomous/waterfall.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/tier2.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/tier2_receipts.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall/evidence.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/waterfall_tests.rs`

**Consumes:** the pure evaluation, `TierReceipt`, `WaterfallState`, and the Tier 1/Tier 1.5 lock pattern.

**Produces:** an injected `Tier2Scan` seam, sealed artifact persistence/replay verification, and `Tier2Progress` which advances only an exhausted receipt.

- [ ] **Step 1: Write failing receipt/recovery tests**

Seed a state cursor at `NoWorkTier::Tier2`. Cover disabled policy, exhausted zero-proposal and all-refuted paths advancing Tier 3, produced/failed retaining Tier 2, identical replay, pre-cursor restart, and tampered/missing/extra/misordered artifact recovery failure.

```rust
assert_eq!(record_tier2(root.path(), REPO, Tier2Scan::NotRun),
    Ok(Tier2Progress::NotRun(DISABLED_REASON.to_string())));
assert_eq!(load_state(&root)?.current_tier(), NoWorkTier::Tier2);
```

- [ ] **Step 2: Run the failing CLI tests**

Run: `cargo test -p autospec-cli --bin autospec tier2`

Expected: FAIL because Tier 2 receipt modules and artifacts do not exist.

- [ ] **Step 3: Add multi-artifact integrity checks**

Add this core accessor:

```rust
pub fn evidence(&self) -> &[SealedEvidence] { &self.evidence }
```

Extend the CLI evidence enum with `Tier2(Policy|Collector|Generated|Dedup|Verification|RoiRank|Failure)`, deriving paths below `waterfall/<pass>/tier2/`. Add `verify_tier2_evidence` that demands the exact ordered reference list per status, recomputes every digest, rejects duplicate/unexpected paths, and validates stage dependency links.

- [ ] **Step 4: Implement the disabled-policy adapter and coordinator**

Declare `tier2` and `tier2_receipts` as private autonomous modules with `#[allow(dead_code)]` until foreground wiring is specified.

```rust
pub(super) enum Tier2Scan { NotRun, Complete(Tier2Observation), Failed(Tier2Failure) }
pub(super) enum Tier2Progress { Pending, Advanced, Produced(u64), Failed(String), NotRun(String) }
pub(super) fn record_tier2(state_root: &Path, repo: &str, scan: Tier2Scan)
    -> Result<Tier2Progress, String>;
```

The only production construction is `Tier2Input::DisabledByCheckedInPolicy`; tests inject `Complete` and `Failed`. Persist artifacts in dependency order, then receipt, then call `WaterfallState::record_receipt` only for `Exhausted`. Replay verifies artifacts before returning the original result.

- [ ] **Step 5: Run the focused CLI recovery proof**

Run: `cargo test -p autospec-cli --bin autospec tier2`

Expected: PASS.

- [ ] **Step 6: Commit the receipt slice**

```bash
git add crates/autospec-core/src/autonomous/waterfall.rs \
  crates/autospec-cli/src/commands/autonomous.rs \
  crates/autospec-cli/src/commands/autonomous/tier2.rs \
  crates/autospec-cli/src/commands/autonomous/tier2_receipts.rs \
  crates/autospec-cli/src/commands/autonomous/waterfall.rs \
  crates/autospec-cli/src/commands/autonomous/waterfall/evidence.rs \
  crates/autospec-cli/src/commands/autonomous/waterfall_tests.rs
git commit -m "feat: seal native Tier 2 waterfall receipts"
```

Use Lore trailers documenting evidence-before-receipt-before-cursor ordering.

### Task 4: Lock authority boundaries and publish the cutover state

**Files:**

- Modify: `crates/autospec-core/tests/autonomous_tier2.rs`
- Modify: `crates/autospec-core/tests/explore_specialists.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/tier2.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/tier2_receipts.rs`
- Modify: `docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md`

**Consumes:** Tasks 1–3.

**Produces:** static evidence that Tier 2 is Rust-only/local-only and a plan record that the model child is separately gated.

- [ ] **Step 1: Write failing authority tests**

Inspect production-only source and reject `bash`, `sh`, `zsh`, `autospec-explore`, `std::env`, `AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT`, cache APIs, `gh`, `curl`, queue, claim, label, `auto-implement`, branch, worktree, PR, `Command`, and `WaterfallStore` in pure core/strict collector modules. Also reject foreground dispatch and mutation routes in the receipt coordinator.

- [ ] **Step 2: Run the failing authority tests**

Run: `cargo test -p autospec-core --test autonomous_tier2 && cargo test -p autospec-cli --bin autospec tier2`

Expected: FAIL until the guards exist.

- [ ] **Step 3: Add guards and publish the status**

Keep the guards narrow enough to avoid unrelated command code. Mark Task 5 complete only for strict collection, pure funnel, sealed receipt/replay, and disabled policy. Record a distinct activation gate for a live model child: fixed executable/version, direct argv, deadline, capped output, schema compatibility, read-only denial, and network-policy proof.

- [ ] **Step 4: Run the full Task 5 verification set**

```bash
cargo test -p autospec-core --test explore_specialists
cargo test -p autospec-core --test autonomous_tier2
cargo test -p autospec-cli --bin autospec
cargo fmt --check
cargo clippy -p autospec-core --all-targets -- -D warnings
cargo clippy -p autospec-cli --bin autospec -- -D warnings
cargo run -q -p autospec-cli -- validate --fast --json
git diff --check
```

Expected: every command exits zero. Report any unrelated workspace-wide diagnostics rather than suppressing them.

- [ ] **Step 5: Commit the authority/cutover slice**

```bash
git add crates/autospec-core/tests/autonomous_tier2.rs \
  crates/autospec-core/tests/explore_specialists.rs \
  crates/autospec-cli/src/commands/autonomous/tier2.rs \
  crates/autospec-cli/src/commands/autonomous/tier2_receipts.rs \
  docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md
git commit -m "test: guard native Tier 2 discovery authority"
```

Use Lore trailers stating that live model activation remains an explicit safety gate, never an unimplemented fallback.
