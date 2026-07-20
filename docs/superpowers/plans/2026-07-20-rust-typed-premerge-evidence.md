# Rust Typed Premerge Evidence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Rust-only premerge admission foundation that binds QA and security evidence to one exact claim generation and commit before a successful executor result can be accepted.

**Architecture:** A pure core module owns strict typed evidence, canonical digests, and Pass/Blocked/Failed decisions. A dedicated CLI module derives branch and commit from a clean attached Git worktree, reads only fixed lane-digest paths, and persists immutable repository-scoped decision/quarantine receipts; explicit successful `executor-result` then consumes an exact persisted Pass receipt and verifies it against the active claim generation and PR head OID.

**Tech Stack:** Rust 2021 workspace, existing strict JSON parser, existing SHA-256 dependency, direct `git`/`gh` argv, filesystem integration fixtures.

## Global Constraints

- No new dependencies.
- Do not call or modify shell/Python autonomous authority, `omx`, `/autospec-run`, or `scripts/autonomous-premerge-gate.sh`.
- Never accept caller-supplied branch, commit, arbitrary artifact paths, or a repository-global quarantine sentinel.
- Derive branch and commit with direct `git -C <repo-dir>` argv; reject tracked/staged dirt or detached worktrees while allowing the fixed untracked evidence directory.
- Bind every decision to repository, issue, worker, claim ID, branch, and commit.
- Read evidence only from `.autospec/evidence/premerge/<lane-digest>/{qa,security}.json` beneath the canonical checkout.
- Missing, unreadable, malformed, incomplete, foreign-producer, or identity-mismatched evidence is Failed and cannot record success.
- Blocking findings create an immutable quarantine only for that exact lane digest; another issue, claim generation, or commit remains eligible.
- Only two complete identity-matching Pass artifacts produce Pass.
- Explicit succeeded `executor-result` must verify the exact active claim ID, a persisted Pass receipt, and a matching PR head OID.
- Bare deferred, blocked, and retryable executor-result protocols remain compatible.
- Decision receipts are observability/admission evidence, never standalone merge or replay authority.
- Every behavior change follows RED → GREEN.

---

### Task 1: Pure typed premerge model and decision engine

**Files:**
- Create: `crates/autospec-core/src/autonomous/premerge.rs`
- Modify: `crates/autospec-core/src/lib.rs`
- Create: `crates/autospec-core/tests/autonomous_premerge.rs`

**Interfaces:**
- Consumes: strict schema-1 QA/security documents and one expected lane identity.
- Produces: lane/evidence types, strict codecs, stable lane/evidence digests, and deterministic decisions.

- [ ] **Step 1: Write failing core tests**

Define fixture builders around these wished-for interfaces:

```rust
use autospec_core::autonomous::premerge::{
    evaluate_premerge, EvidenceAvailability, PremergeDecision, PremergeLaneIdentity,
    QaEvidence, SecurityAuditEvidence,
};

fn lane(issue: u64, claim_id: &str, commit: &str) -> PremergeLaneIdentity {
    PremergeLaneIdentity::new(
        "test/repo",
        issue,
        format!("worker-{issue}"),
        claim_id,
        format!("autonomous/issue-{issue}"),
        commit,
    ).expect("valid lane")
}
```

Add independent tests for: missing QA; missing security; malformed JSON; unknown schema/key/verdict; wrong fixed producer; repo/issue/worker/claim/branch/commit mismatch; Pass; explicit Failed; QA Blocked; security Blocked; Blocked with no codes; Failed with an empty reason; digest stability; changed evidence changes the decision digest; changed claim ID or commit changes the lane digest; and a blocked lane A does not affect a passing lane B.

- [ ] **Step 2: Run focused tests and verify RED**

```bash
cargo test -p autospec-core --test autonomous_premerge
```

Expected: compilation fails because `autonomous::premerge` is absent.

- [ ] **Step 3: Implement exact public types**

Create these public interfaces:

```rust
pub const QA_PRODUCER: &str = "autospec-qa";
pub const SECURITY_AUDIT_PRODUCER: &str = "autospec-secaudit";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PremergeLaneIdentity {
    pub repo: String,
    pub issue: u64,
    pub worker_id: String,
    pub claim_id: String,
    pub branch: String,
    pub commit: String,
}

impl PremergeLaneIdentity {
    pub fn new(repo: impl Into<String>, issue: u64, worker_id: impl Into<String>, claim_id: impl Into<String>, branch: impl Into<String>, commit: impl Into<String>) -> Result<Self, String>;
    pub fn lane_digest(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceVerdict {
    Pass,
    Blocked { finding_codes: Vec<String> },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaEvidence { pub lane: PremergeLaneIdentity, pub run_id: String, pub completed_at: u64, pub verdict: EvidenceVerdict }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityAuditEvidence { pub lane: PremergeLaneIdentity, pub run_id: String, pub completed_at: u64, pub verdict: EvidenceVerdict }

impl QaEvidence { pub fn parse(document: &str) -> Result<Self, String>; pub fn to_json(&self) -> String; }
impl SecurityAuditEvidence { pub fn parse(document: &str) -> Result<Self, String>; pub fn to_json(&self) -> String; }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceAvailability<T> { Present(T), Missing, Malformed(String) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneQuarantine { pub lane: PremergeLaneIdentity, pub evidence_digest: String, pub finding_codes: Vec<String> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PremergeDecision {
    Pass { lane: PremergeLaneIdentity, evidence_digest: String },
    Blocked { lane: PremergeLaneIdentity, reason: String, evidence_digest: String, quarantine: LaneQuarantine },
    Failed { lane: PremergeLaneIdentity, reason: String, evidence_digest: String },
}

pub fn evaluate_premerge(lane: &PremergeLaneIdentity, qa: EvidenceAvailability<QaEvidence>, security: EvidenceAvailability<SecurityAuditEvidence>) -> PremergeDecision;
```

The strict evidence JSON contains exactly: `schema`, `kind`, `producer`, `repo`, `issue`, `worker_id`, `claim_id`, `branch`, `commit`, `run_id`, `completed_at`, `verdict`, `finding_codes`, and `reason`. Require schema `1`; kind/producer pairs `qa`/`autospec-qa` and `security-audit`/`autospec-secaudit`; positive issue/completed timestamp; bounded nonempty identifier/reason/code fields; and a 40- or 64-character lowercase hexadecimal commit. Pass requires empty codes/reason, Blocked requires at least one code and empty reason, Failed requires no codes and nonempty reason.

Use length-prefixed SHA-256 canonicalization. `lane_digest` hashes version `autospec-premerge-lane-v1` plus all six identity fields. The decision digest hashes version `autospec-premerge-evidence-v1`, the lane digest, and the canonical QA/security availability payloads in QA-then-security order. Decision precedence is invalid/missing/malformed/mismatch → Failed; explicit Failed → Failed; Blocked → Blocked; otherwise Pass.

- [ ] **Step 4: Run tests and verify GREEN**

```bash
cargo test -p autospec-core --test autonomous_premerge
```

Expected: the full new matrix passes.

- [ ] **Step 5: Commit Task 1**

Commit `feat: type premerge evidence decisions` with Lore trailers naming strict producer/identity binding and the focused test.

---

### Task 2: Rust premerge evaluate command and immutable lane receipts

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/premerge.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/src/commands/claim.rs`
- Create: `crates/autospec-cli/tests/autonomous_premerge_commands.rs`

**Interfaces:**
- Consumes: Task 1 model plus active claim state, canonical Git worktree identity, and fixed evidence files.
- Produces: `autospec autonomous premerge evaluate` and immutable repository-scoped decision state.

- [ ] **Step 1: Write failing CLI tests**

Add fixtures for:

```text
autospec autonomous premerge evaluate --repo test/repo --repo-dir <worktree> --issue 42 --worker-id worker-42 --claim-id claim-42 --json
```

Cover: clean attached branch derives exact branch/HEAD; tracked or staged dirt and detached worktrees fail before receipt creation; untracked files in the fixed evidence directory remain readable; only the fixed lane-digest paths are read; missing/malformed/foreign evidence returns Failed; blocking evidence creates `quarantine.json`; passing evidence creates no quarantine; decisions are create-once and idempotent; attempts to replace an existing immutable decision fail closed; blocked lane A does not affect passing lane B; and poisoned `bash`, `sh`, `omx`, `/autospec-run`, and legacy-script markers are never invoked.

- [ ] **Step 2: Run and verify RED**

```bash
cargo test -p autospec-cli --test autonomous_premerge_commands -- --test-threads=1
```

Expected: `premerge evaluate` is an unknown subcommand.

- [ ] **Step 3: Implement the closed command boundary**

Route `args.first() == "premerge"` to the new module. Accept only subcommand `evaluate` and flags `--repo`, `--repo-dir`, `--issue`, `--worker-id`, `--claim-id`, and optional `--json`, each exactly once. Canonicalize `--repo-dir`; use direct `git -C <repo-dir> symbolic-ref --quiet --short HEAD`, `git -C <repo-dir> rev-parse HEAD`, and `git -C <repo-dir> status --porcelain --untracked-files=no`; reject detached or tracked/staged dirty state without rejecting required untracked evidence artifacts.

Build `PremergeLaneIdentity`, then call a new read-only `claim::active_claim_generation_matches(repo, issue, worker_id, claim_id, branch) -> Result<bool, CommandFailure>` helper before reading evidence. The helper reuses the existing comment selection and freshness logic and requires exact repo/issue/worker/claim/branch identity without writing comments, labels, or run state. Then read only:

```text
<repo-dir>/.autospec/evidence/premerge/<lane-digest>/qa.json
<repo-dir>/.autospec/evidence/premerge/<lane-digest>/security.json
```

Map NotFound to Missing, invalid UTF-8/parse to Malformed, and other I/O errors to diagnostics. Do not discover, execute, or accept producer command paths.

- [ ] **Step 4: Persist exact lane decisions**

Use the existing repository-scoped `RunLayout.state_dir` and atomic-write helper to create:

```text
premerge/lanes/<lane-digest>/decisions/<decision-digest>.json
premerge/lanes/<lane-digest>/latest.json
premerge/lanes/<lane-digest>/quarantine.json
```

The decision document contains schema `1`, decision, all lane identity fields, lane digest, evidence digest, reason, and finding codes. Decision files are create-once immutable; `latest.json` is atomic and may point only to a decision file that was re-read successfully. Create `quarantine.json` only for Blocked and never overwrite it. Failed creates no quarantine. Emit exit `0` for Pass, `20` for Blocked, and `2` for Failed/diagnostic.

- [ ] **Step 5: Run tests and verify GREEN**

```bash
cargo test -p autospec-cli --test autonomous_premerge_commands -- --test-threads=1
```

- [ ] **Step 6: Commit Task 2**

Commit `feat: persist lane-bound premerge receipts` with Lore trailers naming Git-derived identity, fixed paths, and lane-only quarantine.

---

### Task 3: Bind successful executor results to claim, receipt, and PR commit

**Files:**
- Modify: `crates/autospec-core/src/claim/mod.rs`
- Modify: `crates/autospec-cli/src/commands/claim.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/premerge.rs`
- Modify: `crates/autospec-cli/tests/autonomous_conductor_commands.rs`
- Test: `crates/autospec-core/tests/claim_tiebreak.rs`

**Interfaces:**
- Consumes: Task 2 persisted Pass receipt.
- Produces: exact claim-generation and commit-bound success ingestion.

- [ ] **Step 1: Write failing claim and CLI tests**

Add tests that require `OpenPullRequest` to parse `headRefOid`; require a successful result to supply `--claim-id <id> --premerge-receipt <64-lower-hex>`; reject missing receipt, wrong claim generation, non-Pass receipt, quarantined lane, wrong receipt identity, and PR head OID mismatch; accept one exact Pass receipt; persist claim ID, commit, and premerge receipt digest in `ExecutorResultEvidence`; reject replay from a successor claim even if worker and branch are reused; and retain the bare deferred plus blocked/retryable protocols unchanged.

- [ ] **Step 2: Run focused tests and verify RED**

```bash
cargo test -p autospec-core --test claim_tiebreak executor_result
cargo test -p autospec-cli --test autonomous_conductor_commands executor_result -- --test-threads=1
```

Expected: success currently accepts only worker/branch/PR and PR JSON lacks `headRefOid`.

- [ ] **Step 3: Strengthen claim evidence**

Add `head_ref_oid: String` to `OpenPullRequest`, parse exact GitHub field `headRefOid`, and extend the `gh pr list --json` field list. Extend `ExecutorResultEvidence` schema with nonempty `claim_id`, `commit`, and 64-character lowercase `premerge_receipt`. Update constructors, parser, serializer, equality confirmation, and core tests. Change claim ownership validation for successful results to require the exact active claim ID as well as worker/branch.

- [ ] **Step 4: Gate explicit success on a persisted Pass receipt**

Extend only explicit `--outcome succeeded` with required `--claim-id` and `--premerge-receipt`; forbid them on other outcomes. Re-read Task 2's immutable receipt by digest from the repository-scoped state, require decision Pass and exact repo/issue/worker/claim/branch identity, take its commit as authoritative, and require the selected open PR's `headRefOid` to equal that commit before calling `record_executor_result`. Pass claim ID, commit, and receipt digest into immutable executor-result evidence.

- [ ] **Step 5: Run focused and regression tests**

```bash
cargo test -p autospec-core --test claim_tiebreak
cargo test -p autospec-cli --test autonomous_conductor_commands -- --test-threads=1
```

Expected: all success hardening and compatibility cases pass.

- [ ] **Step 6: Commit Task 3**

Commit `feat: bind executor success to premerge receipt` with Lore trailers naming claim-generation and PR-head constraints.

---

### Task 4: Documentation, source authority, and final verification

**Files:**
- Modify: `docs/API_REFERENCE.md`
- Modify: `docs/runbooks/mainline-health-admission.md`
- Modify: `crates/autospec-cli/tests/autonomous_premerge_commands.rs`

**Interfaces:**
- Consumes: Tasks 1-3 public contracts.
- Produces: operator documentation and negative reachability proof.

- [ ] **Step 1: Add the negative authority regression**

Read the new CLI module source and assert it contains direct Git argument construction plus `evaluate_premerge`, and does not contain shell command construction, `omx`, `/autospec-run`, environment command overrides, or legacy autonomous/premerge script paths.

- [ ] **Step 2: Run the source regression**

```bash
cargo test -p autospec-cli --test autonomous_premerge_commands source_has_no_legacy_authority
```

- [ ] **Step 3: Document the admission-foundation scope**

Document the exact evidence schema and producer IDs, fixed artifact paths, clean attached-worktree requirement, decision/quarantine paths, exit codes, explicit success flags, claim/commit/PR binding, and observability-only receipt boundary. State explicitly that foreground still has a deferred implementation executor and that native QA/security producers plus foreground quarantine-and-continue belong to the supervised executor follow-up; do not claim production premerge parity yet.

- [ ] **Step 4: Run final verification**

```bash
cargo test -p autospec-core --test autonomous_premerge
cargo test -p autospec-core --test claim_tiebreak
cargo test -p autospec-cli --test autonomous_premerge_commands -- --test-threads=1
cargo test -p autospec-cli --test autonomous_conductor_commands -- --test-threads=1
cargo fmt --all -- --check
cargo clippy -p autospec-core --all-targets -- -D warnings
cargo clippy -p autospec-cli --bin autospec -- -D warnings
cargo run -q -p autospec-cli -- validate --fast --json
git diff --check
```

Any unchanged baseline validator failure must be reproduced on `origin/main` and tracked separately rather than added to #1697.

- [ ] **Step 5: Commit Task 4**

Commit `docs: define Rust premerge admission boundary` with Lore trailers listing exact verification and the remaining producer/executor dependency.
