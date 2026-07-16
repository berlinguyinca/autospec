# Rust Tier 4 External Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Add a Rust-owned, receipt-backed Tier 4 external-discovery foundation that fail-closes untrusted input and cannot activate external retrieval or remote work.

**Architecture:** The core parser interprets only checked-in source descriptors. A pure evaluator consumes typed injected source envelopes, candidate references, verifier verdicts and ROI policy and emits canonical documents. The CLI persists and replays those documents; the production adapter records only an exact disabled-policy receipt.

**Tech Stack:** Rust standard library, existing autospec-core JSON and waterfall primitives, Cargo tests, Cargo fmt, Clippy, native fast validation.

## Global Constraints

- No new dependency and no HTTP/TLS/URL client: live retrieval is outside this plan.
- Production Tier 4 may not use shell, process, network, GitHub, model, queue, claim, branch, worktree, foreground, executor, or remote-mutation APIs.
- Tier 4 config is descriptor data only: it has no enabled flag, protocol, URL, transport, or command.
- Present config has 1 through 4 unique sources. IDs are lower-kebab ASCII up to 64 bytes. Hosts are lowercase ASCII DNS. Paths are absolute and up to 256 bytes with no query, fragment, backslash, empty, dot, or dot-dot segment.
- max_bytes is 1 through 1_048_576, deadline_millis is 100 through 30_000, source facts are at most 128 per source, generated references at most 512, ROI threshold is 500 of 1000, and rank is capped at ten.
- Evidence never contains raw response bytes. Funnel counts remain monotonic: observed >= deduplicated >= verified >= roi_approved >= ranked.
- V1 disabled policy is exactly tier4_external_discovery_disabled_by_checked_in_policy, producer rust-tier4-disabled-policy-v1, one policy artifact, zero funnel.
- Only Exhausted(NoProposalsGenerated | VerificationRejected | RoiFiltered) advances Tier 4. NotRun, Produced, Failed, Blocked, and every other dry reason retain Tier 4.
- Preserve completed Tier 4 receipt references through Tier4-to-Tier1 rollover, so load/persist replay can reject a forged completed state.
- Every new Tier 4 source/test stays at or below 450 lines. Do not touch .superpowers/sdd/task-1-report.md.
- Use Conventional commits with Lore trailers. Never amend or bypass hooks.

---

## File Structure

| Path | Responsibility |
| --- | --- |
| crates/autospec-core/src/autonomous/config.rs | Top-level config composition and public re-exports. |
| crates/autospec-core/src/autonomous/config/tier4/{mod,validation}.rs | Strict Tier 4 grammar and scalar validation. |
| crates/autospec-core/src/autonomous/tier4/{model,candidate,failure,evaluate,evidence}.rs | Pure typed model, closed failure grammar, evaluator and documents. |
| crates/autospec-core/src/autonomous/waterfall.rs | Retained rollover-chain semantics. |
| crates/autospec-cli/src/commands/autonomous/{tier4,tier4_receipts}.rs | Disabled-only adapter and receipt coordinator. |
| crates/autospec-cli/src/commands/autonomous/waterfall/tier_evidence.rs | Tier 2/3 method extraction plus Tier 4 store methods. |
| crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier4*.rs | Canonical replay, lexical shape and consistency checks. |
| Tier 4 core/CLI test files | Contract, authority, tamper, recovery, failure-prefix and state tests. |

### Task 1: Parse strict checked-in Tier 4 descriptor policy

**Files:**
- Modify: crates/autospec-core/src/autonomous/config.rs
- Create: crates/autospec-core/src/autonomous/config/tier4/mod.rs
- Create: crates/autospec-core/src/autonomous/config/tier4/validation.rs
- Create: crates/autospec-core/tests/autonomous_tier4_config.rs
- Modify: crates/autospec-core/tests/autonomous_config.rs

**Interfaces:**
```rust
pub struct AutonomousConfig {
    pub main_health: MainHealthConfig,
    pub tier4: Tier4Config,
}
pub struct Tier4Config { pub sources: Vec<Tier4SourceDescriptor> }
pub struct Tier4SourceDescriptor {
    pub id: String, pub host: String, pub path: String,
    pub max_bytes: u32, pub deadline_millis: u32,
}
```

- [ ] **Step 1: Write the failing tests**

Add valid one/two-source fixtures with exact descriptor assertions; table-test absent/mixed main_health policy, nested unrelated tier4, duplicate blocks/fields/IDs/hosts, zero/five sources, tabs/indentation, inline collections, invalid IDs, uppercase/IP/scheme/port/userinfo/wildcard hosts, invalid paths, and signed/non-digit/overflow/out-of-range limits.

- [ ] **Step 2: Run red**

Run: cargo test -p autospec-core --test autonomous_tier4_config

Expected: compile failure because Tier 4 types and parser are absent.

- [ ] **Step 3: Implement the closed parser**

```rust
fn valid_id(value: &str) -> bool;
fn valid_host(value: &str) -> bool;
fn valid_path(value: &str) -> bool;
fn parse_bounded_u32(value: &str, min: u32, max: u32, line: usize, field: &str)
    -> Result<u32, String>;
```

Keep existing main_health behavior. Add an independent tier4::parse state machine: at indent zero only exact tier4: opens the relevant block; at indent two only empty sources:; at indent four only - id: scalar; at indent six each remaining scalar field exactly once. Finalize at next list item, top-level block or EOF. Reject every malformed relevant shape with established line-numbered diagnostics. Do not add activation or transport fields.

- [ ] **Step 4: Verify and commit**

Run: cargo test -p autospec-core --test autonomous_config --test autonomous_tier4_config && cargo fmt --check && cargo clippy -p autospec-core --all-targets -- -D warnings && git diff --check

Commit: feat: parse strict Tier 4 source policy

Lore records the unapproved HTTP constraint, rejects shell configuration reuse, and states parsing never activates retrieval.

### Task 2: Seal a pure typed Tier 4 funnel

**Files:**
- Modify: crates/autospec-core/src/lib.rs
- Create: crates/autospec-core/src/autonomous/tier4.rs
- Create: crates/autospec-core/src/autonomous/tier4/model.rs
- Create: crates/autospec-core/src/autonomous/tier4/candidate.rs
- Create: crates/autospec-core/src/autonomous/tier4/failure.rs
- Create: crates/autospec-core/src/autonomous/tier4/evaluate.rs
- Create: crates/autospec-core/src/autonomous/tier4/evidence.rs
- Create: crates/autospec-core/tests/autonomous_tier4.rs
- Create: crates/autospec-core/tests/autonomous_tier4_contract.rs
- Create: crates/autospec-core/tests/autonomous_tier4_authority.rs

**Interfaces:**
```rust
pub const DISABLED_REASON: &str = "tier4_external_discovery_disabled_by_checked_in_policy";
pub const TIER4_SCHEMA: u64 = 1;
pub const TIER4_RANK_LIMIT: u64 = 10;
pub fn evaluate_tier4(input: Tier4Input) -> Result<Tier4Evaluation, Tier4Failure>;
pub enum Tier4Input {
    DisabledByCheckedInPolicy,
    Enabled {
        source_policy: Tier4SourcePolicy,
        sources: Vec<Tier4StageResult<Tier4SourceEnvelope>>,
        generated: Tier4StageResult<Tier4GeneratedCandidates>,
        verifier: Tier4StageResult<Tier4VerifierVerdicts>,
        roi_policy: Tier4RoiPolicy,
    },
}
```

- [ ] **Step 1: Write failing core and authority tests**

Prove exact disabled reason/no observation; descriptor/source ordering and coverage; SHA-256/body-cap/source-fact validation; candidate references; deterministic dedup; verdict coverage; all-rejected verification; ROI rejection; ROI-descending/stable-key ranking; rank cap; exact funnel counts; every exhausted reason; canonical documents and predecessor links; sealed partial failures. Recursively strip comments/literals and reject filesystem/environment/process/network, shell/curl, GitHub, queue/claim/branch/worktree, HTTP clients/facades/free functions, model dispatch, WaterfallStore, include!, and module escapes. Assert no public model accepts raw body bytes or Vec<u8>.

- [ ] **Step 2: Run red**

Run: cargo test -p autospec-core --test autonomous_tier4 --test autonomous_tier4_contract --test autonomous_tier4_authority

Expected: compile failure because Tier 4 is absent.

- [ ] **Step 3: Implement evaluator, failures and documents**

Use closed stages SourcePolicy, Sources, Generator, Deduplicator, Verifier and RoiRank. Use failure codes MissingStageResult, InvalidSourcePolicy, InvalidSourceCoverage, InvalidSourceEnvelope, InvalidSourceFact, InvalidGeneratedCandidates, InvalidCandidate, DuplicateConflict, InvalidVerdictCoverage, InvalidRoiPolicy, InvalidRanking and CountOverflow. Envelopes match every descriptor exactly once and in policy order. Generated candidates reference typed facts. Dedup by stable_key and reject semantic conflicts. Require exactly one verdict per deduplicated key. Only fully completed empty funnels map to the three permitted dry reasons; all malformed, missing or failed stages seal Tier4Failure.

Render schema-one newline-terminated source_policy, sources, generated, dedup, verification, roi_rank and failure documents. Source_policy has no predecessor; each later document links the prior digest; only a source-policy failure has null predecessor.

- [ ] **Step 4: Verify and commit**

Run: cargo test -p autospec-core --test autonomous_tier4 --test autonomous_tier4_contract --test autonomous_tier4_authority && cargo fmt --check && cargo clippy -p autospec-core --all-targets -- -D warnings && git diff --check

Commit: feat: model native Tier 4 source evidence

Lore rejects raw payload retention and says a future transport requires separate security approval.

### Task 3: Seal Tier 4 receipts and preserve rollover auditability

**Files:**
- Modify: crates/autospec-core/src/autonomous/waterfall.rs
- Modify: crates/autospec-core/src/autonomous/waterfall/codec.rs
- Modify: crates/autospec-cli/src/commands/autonomous.rs
- Modify: crates/autospec-cli/src/commands/autonomous/waterfall.rs
- Create: crates/autospec-cli/src/commands/autonomous/waterfall/tier_evidence.rs
- Modify: crates/autospec-cli/src/commands/autonomous/waterfall/evidence.rs
- Create: crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier4.rs
- Create: crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier4_shape.rs
- Create: crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier4_consistency.rs
- Create: crates/autospec-cli/src/commands/autonomous/tier4.rs
- Create: crates/autospec-cli/src/commands/autonomous/tier4_receipts.rs
- Create: crates/autospec-cli/src/commands/autonomous/tier4_receipts_tests.rs
- Create: crates/autospec-cli/src/commands/autonomous/tier4_receipts_recovery_tests.rs
- Create: crates/autospec-cli/src/commands/autonomous/tier4_receipts_failure_prefix_tests.rs
- Create: crates/autospec-cli/src/commands/autonomous/tier4_receipts_state_tests.rs

**Interfaces:**
```rust
pub enum Tier4EvidenceArtifact {
    Policy, SourcePolicy, Sources, Generated, Dedup, Verification, RoiRank, Failure,
}
pub(super) fn record_tier4(state_root: &Path, repo: &str, scan: Tier4Scan)
    -> Result<Tier4Progress, String>;
```

- [ ] **Step 1: Write failing coordinator/tamper/rollover tests**

Seed empty Tier 1 through 3 then assert disabled Tier 4 has producer rust-tier4-disabled-policy-v1, only waterfall/1/tier4/policy.json, and zero funnel. Assert the three allowed exhausted results advance to Tier 1. Assert Produced, NotRun, Failed, Blocked, every other dry reason, missing/extra/reordered evidence, changed digest, bad producer, raw-body key, altered predecessor, noncanonical JSON and forged state all reject or retain Tier 4. Assert Tier4-to-Tier1 retains and verifies prior pass receipts, while next-pass Tier1 clears history.

- [ ] **Step 2: Run red**

Run: cargo test -p autospec-cli --bin autospec tier4

Expected: compile failure because adapter, artifacts and verifier are absent.

- [ ] **Step 3: Implement store split, receipt chain and state gate**

Move Tier 2/3 store methods into child-module impl WaterfallStore in tier_evidence.rs before adding Tier 4 methods, keeping waterfall.rs below 450 lines. Add exact artifact references and strict verifier methods. Retain all five completed receipt references only on a valid Tier4 exhausted rollover to Tier1 with next_pass_id > 1; clear history before next-pass Tier1 record. Replay the retained chain with pass next_pass_id - 1. Verify exact artifact prefixes, canonical keys, schema/kind/reference/digest/predecessor chain, policy/envelope correspondence, raw-body absence, producer identity, and terminal status/funnel/count matching. Coordinator order is verify evidence, persist receipt, persist state; replay existing receipt before input scan. Production adapter constructs only DisabledByCheckedInPolicy and is never foreground-called.

- [ ] **Step 4: Verify and commit**

Run: cargo test -p autospec-core --test autonomous_tier4 && cargo test -p autospec-cli --bin autospec tier2 && cargo test -p autospec-cli --bin autospec tier3 && cargo test -p autospec-cli --bin autospec tier4 && cargo fmt --check && cargo clippy -p autospec-cli --bin autospec -- -D warnings && git diff --check

Run: wc -l crates/autospec-cli/src/commands/autonomous/tier4*.rs crates/autospec-cli/src/commands/autonomous/waterfall.rs crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier4*.rs

Commit: feat: seal native Tier 4 discovery receipts

Lore explains why NotRun cannot be dry and retained receipts prevent forged completed state from disappearing.

### Task 4: Close authority gaps and update truthful cutover status

**Files:**
- Modify: crates/autospec-core/tests/autonomous_tier4_authority.rs
- Modify: crates/autospec-cli/src/commands/autonomous/tier4.rs
- Modify: crates/autospec-cli/src/commands/autonomous/tier4_receipts_tests.rs
- Modify: docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md

- [ ] **Step 1: Write adversarial guard/status tests**

Add literal-stripped fixtures for direct clients, facade aliases, free-function HTTP, Repository/Store types, dynamic shell, include!, and path attributes. Assert nonempty parsed Tier4Config still generates exact disabled NotRun, creates no source evidence and never advances.

- [ ] **Step 2: Implement guard/status closure**

Tighten authority matching without matching comments/literals. Correct stale Tier 2/3 foundation checkboxes and add accurate Tier 4 foundation text. Explicitly leave live retrieval, source activation, foreground wiring, Task 8 ideation, executor/premerge parity, validation/installer migration and shell deletion unchecked.

- [ ] **Step 3: Verify and commit**

Run: cargo test -p autospec-core --test autonomous_tier4_authority && cargo test -p autospec-cli --bin autospec tier4 && rg -n "legacy deletion|foreground wiring|Tier 4" docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md && git diff --check

Commit: test: prevent hidden Tier 4 discovery authority

Lore records that source configuration is data, not activation.

### Task 5: Complete final verification and independent review

**Files:** Modify only files needed to correct findings from the commands below.

- [ ] **Step 1: Run complete coverage**

```bash
cargo test -p autospec-core --test autonomous_config --test autonomous_tier4_config
cargo test -p autospec-core --test autonomous_tier3 --test autonomous_tier3_authority
cargo test -p autospec-core --test autonomous_tier4 --test autonomous_tier4_contract --test autonomous_tier4_authority
cargo test -p autospec-cli --bin autospec tier2
cargo test -p autospec-cli --bin autospec tier3
cargo test -p autospec-cli --bin autospec tier4
cargo fmt --check
cargo clippy -p autospec-core --all-targets -- -D warnings
cargo clippy -p autospec-cli --bin autospec -- -D warnings
cargo run -q -p autospec-cli -- validate --fast --json
git diff --check
git diff --check 493f0367..HEAD
```

Expected: every command exits zero.

- [ ] **Step 2: Conduct independent adversarial review**

Compare final diff against docs/superpowers/specs/2026-07-16-rust-tier4-external-discovery-design.md. Reject and repair network/process/model/GitHub behavior, raw body storage, non-exhausted advance, unsealed/reordered evidence, Tier 2/3 semantic change, source-cap breach or false cutover claim. Each repair uses a fresh Conventional Lore commit and reruns affected tests.

- [ ] **Step 3: Deliver evidence**

Report changed files, simplifications, exact passing commands, review result and remaining risk. State that Tier 4 production is disabled and legacy shell deletion still depends on foreground traversal, safe source activation, local ideation, executor/premerge parity, validation/installer replacement and final parity audit.
