# Evidence-Grounded Review Governance and Self-Improvement Design

**Date:** 2026-08-11
**Status:** Proposed for implementation
**Scope:** Native autonomous executor, multi-harness review adapters, Phase 5.5 outcome attribution, and bounded self-improvement

## Goal

Autospec must review every autonomous change with a provably independent, risk-appropriate, evidence-bearing reviewer; detect integration-shaped risk before merge; attribute escaped defects to the review policy that admitted them; and autonomously propose, canary, promote, or roll back bounded review-policy improvements from repository evidence.

## Outcome

The system will replace “a reviewer printed `LGTM`” with a canonical pipeline:

```text
issue + diff + risk signals
  -> deterministic ReviewPolicy
  -> executable pre-merge evidence
  -> independent structured review
  -> digest-bound ReviewOutcome
  -> merge admission
  -> Phase 5.5 escaped-defect attribution
  -> falsifiable self-improvement experiment
  -> promote, hold, or roll back
```

Autospec will not claim sentience. The implementable objective is autonomous engineering judgment: it asks evidence-grounded questions, states what would falsify its hypothesis, changes one bounded policy variable, measures the result, and retains or reverts the change without routine operator input.

## Existing Foundation

- The native executor already runs deterministic pre-merge evidence and CI before independent review.
- Native reviewers already execute outside the reviewed repository, receive a sanitized environment, run read-only, and fail if they mutate local or remote state.
- Review receipts already bind the issue, commit, claim, executable, and artifacts, but not semantic evidence.
- The skill-driven path already has deterministic Guardian checks, a fused correctness reviewer, and Phase 5.5 broad review.
- The advisor already has a minimum-sample policy ratchet, but its current `lgtm_first_pass` input is cache warmth rather than review quality.
- The Tier-3 waterfall already accepts deterministic self-improvement candidates and routes them through `needs-classify`.

## Design Principles

1. **One policy contract.** Native Rust owns review classification and receipt semantics; skill adapters mirror it rather than inventing another policy.
2. **Independent means provable.** A review that cannot prove a distinct read-only execution context may advise but may not authorize autonomous merge.
3. **Runtime evidence precedes semantic review.** The executor runs tests and integration smoke commands; the reviewer inspects immutable evidence and does not gain mutation authority.
4. **Risk selects cost.** Normal changes use one standard independent reviewer. High, integration, and critical changes receive stronger or provider-diverse review.
5. **Outcomes train policy, not prompts directly.** Escaped defects update a typed ledger. They do not grant a model permission to rewrite arbitrary instructions.
6. **Strengthening is easier than weakening.** Autospec may autonomously add bounded checks after evidence. Removing safety, review, security, or merge controls is never self-approved.
7. **Every policy experiment is falsifiable and reversible.** A proposal without a named metric, sample floor, and rollback cannot enter the autonomous queue.

## Approaches Considered

### A. Prompt-only review improvements

Update `SKILL.md` reviewer instructions and keep the native executor unchanged.

- Advantage: smallest immediate diff.
- Rejected: policy would drift across harnesses, the native receipt would still lack semantic evidence, and escaped defects could not be attributed reliably.

### B. Canonical native policy with mirrored adapters — selected

Add typed policy, verdict, and outcome contracts to the native executor; teach skill adapters to use the same contract; feed attributed Phase 5.5 outcomes to the advisor and Tier-3 self-improvement waterfall.

- Advantage: one admission truth, replayable evidence, incremental migration, and compatibility with current orchestration.
- Cost: coordinated Rust, shell, skill-triplet, schema, and test changes.

### C. Free-form recursive agent rewriting its own review prompts

Let a model inspect failures and directly edit review policy.

- Advantage: superficially flexible.
- Rejected: correlated self-approval, unbounded policy drift, weak falsifiability, and no reliable rollback boundary.

## Canonical Data Contracts

### Review risk

```rust
enum ReviewRisk {
    Normal,
    High,
    Integration,
    Critical,
}
```

### Review policy

```rust
struct ReviewPolicy {
    schema: u32,
    risk: ReviewRisk,
    reasons: Vec<String>,
    integration_shaped: bool,
    require_integration_smoke: bool,
    reviewer_harness: HarnessKind,
    reviewer_reasoning: ReviewReasoning,
    provider_diversified: bool,
}
```

The policy is deterministic and commit-bound. Inputs are issue labels and serialization reasons, base-to-HEAD changed paths, logical component count, blast radius, and sensitive path classes.

### Structured reviewer verdict

```json
{
  "schema": 1,
  "commit": "0123456789abcdef0123456789abcdef01234567",
  "verdict": "lgtm",
  "surfaces_examined": ["review selection", "receipt recovery"],
  "tests_examined": ["cargo test reviewer_receipt"],
  "integration_paths_checked": ["selection -> review -> receipt -> merge"],
  "blocking_findings": []
}
```

Admission requires exact keys, the expected 40-hex commit, `verdict == "lgtm"`, empty blocking findings, nonempty examined surfaces and tests, bounded nonempty strings, and at least one integration path for integration-shaped work.

### Review outcome

The durable review receipt gains:

- the canonical policy and policy digest;
- reviewer harness, reasoning class, and provider-diversity result;
- the structured verdict digest and parsed semantic fields;
- the existing commit, claim, executable, stdout, stderr, and artifact bindings.

The external state machine may continue consuming exact `LGTM`, but only the trusted normalizer emits it after validating the structured verdict.

### Escaped-defect outcome

Each reviewed PR receives one append-only row in `.autospec/review-outcomes.jsonl`:

```json
{
  "schema": 1,
  "pr": 123,
  "commit": "0123456789abcdef0123456789abcdef01234567",
  "review_receipt_digest": "sha256:...",
  "reviewer_harness": "codex",
  "reviewer_reasoning": "high",
  "provider_diversified": true,
  "review_risk": "integration",
  "first_pass_lgtm": true,
  "escaped_high_severity": 0,
  "escaped_total": 0,
  "phase55_run": "20260811T120000Z"
}
```

Rows are immutable observations. Corrections append superseding rows rather than rewriting history.

## Feature 1: Pre-Merge Integration-Shaped Review

`classify_review_policy` marks a change integration-shaped when any of these are true:

- the diff spans at least two normalized runtime components;
- changed paths include orchestration, installation, adapter/provider, daemon/session, state recovery, or merge-authority surfaces;
- the diff changes both a producer and a consumer surface;
- issue signals include `reasoning:deep` or `priority:high`.

Integration-shaped work must provide `### Integration smoke test (pre-merge)` containing exactly one executable command. For migration compatibility, the existing primary smoke may satisfy the requirement only when it invokes a repository test under `tests/integration/`, `tests/smoke/`, or `tests/e2e/`.

The executor parses the command with the existing safe direct-command parser, runs it with the same containment and output bounds as other pre-merge evidence, and binds its observation into the evidence bundle. Failure, absence, ambiguity, or identity drift blocks review and merge with a typed reason.

The reviewer receives the policy, component inventory, immediate producer/consumer surfaces, and immutable smoke evidence. It remains read-only.

## Feature 2: Automatic Reviewer Strength and Diversity

Reviewer selection becomes risk-aware:

| Risk | Selection |
| --- | --- |
| Normal | Current available provider, standard reasoning |
| High | High reasoning; prefer a provider different from the implementer |
| Integration | High reasoning; prefer a different provider; require integration evidence |
| Critical | Different provider required; fail closed when unavailable |

No dated model identifier is hard-coded. Selection uses installed harness aliases and configured frontier/default model aliases.

Provider diversity supplements but does not replace process independence. Every reviewer remains a fresh, external, read-only invocation. The selected identity and fallback reason are receipt-bound.

## Feature 3: Fail-Closed Independent Review Everywhere

The native executor’s existing invariant becomes universal:

```text
no provably independent reviewer -> no autonomous merge
```

The shell-facing admission boundary is `independent-review-adapter.sh`: `prepare`
writes a commit-bound request or returns typed requeue exit `75`, and `validate`
is the only path that normalizes a closed-schema clean verdict to `LGTM`. The
Claude, Codex, and OpenCode adapters share this behavior; none may substitute
the author context when foreground delegation is unavailable.

Both the end-to-end `autospec` workflow and the implementation-only
`autospec-run` workflow consume this same admission rule, so entering Phase 4
through either surface produces the same typed blocker and resume behavior.
The rule is stored as a generated skill block so future trio regeneration cannot
quietly restore an in-thread review fallback in only one harness.

- Production `AUTOSPEC_EXECUTOR_REVIEW_COMMAND` either passes through the same structured adapter and identity checks or is removed; unstructured commands cannot authorize merge.
- Skill adapters may not fall back to reviewing in the author/implementer context.
- When independent delegation is unavailable, the issue is visibly requeued or blocked with a typed reason and no PR merge.
- Author, verifier, and approver lane identities remain distinct and auditable.

## Feature 4: Evidence-Bearing Review Verdicts

Reviewer prompts request only structured JSON. The trusted normalizer:

1. reads the bounded private result artifact;
2. rejects duplicate/unknown keys and malformed values;
3. checks the exact commit and policy requirements;
4. rejects any blocking finding;
5. persists the parsed semantic evidence and its digest;
6. emits exact `LGTM` only after every check passes.

A receipt authenticates what was reviewed and which evidence supported admission. It does not claim that an LLM is infallible.

## Feature 5: Escaped-Defect Quality Governance

Phase 5.5 findings gain optional origin attribution:

- originating PR and commit;
- review receipt digest;
- reviewer harness and reasoning;
- provider-diversity result;
- review risk.

Gap normalization and remediation preserve attribution end to end. Missing attribution is explicit and cannot count as a clean reviewed sample.

Advisor quality metrics become:

1. high-severity escaped defects per attributed reviewed PR;
2. all escaped defects per attributed reviewed PR;
3. review cost per attributed reviewed PR;
4. first-pass LGTM as diagnostic information only.

Policy behavior:

- Any attributed high-severity escape may strengthen the matching review policy immediately.
- Relaxation requires at least `AUTOSPEC_ADVISOR_MIN_SAMPLES` samples, default `20`, with zero high-severity escapes and no regression in total escape rate.
- No attributed samples means hold; it never means success.
- A failed or unavailable Phase 5.5 review emits `review_unavailable`, not an empty clean result.

## Autonomous Self-Improvement Loop

### Observation

The loop consumes review outcomes, Phase 5.5 gaps, review receipts, blocked pre-merge evidence, learning-ledger repetitions, and rollback events.

### Questions Autospec asks itself

For each evidence cluster, Autospec instantiates this bounded taxonomy:

1. Which invariant failed after review admission?
2. Which producer/consumer boundary was omitted?
3. What executable test would have falsified approval before merge?
4. Is the escape correlated with a reviewer provider, reasoning class, risk class, or missing evidence type?
5. What is the smallest reusable policy or test change?
6. What legitimate change could the proposal falsely block?
7. What sample and metric would justify promotion?
8. What exact rollback restores the prior policy?

### Hypothesis contract

A self-improvement candidate must contain:

- exact evidence paths and receipt digests;
- one named failed invariant and one named consumer;
- one falsifier executable against repository evidence;
- bounded files and a dedupe key;
- expected before/after metrics;
- sample floor and maximum cost regression;
- rollback command or prior-policy digest;
- classification as strengthening, neutral, or weakening.

Evidence-free candidates are discarded. Weakening candidates cannot be autonomously promoted.

### Experiment lifecycle

```text
candidate
  -> deterministic validation
  -> adversarial falsifier
  -> needs-classify/origin:self issue
  -> normal autonomous implementation
  -> independent review
  -> shadow observation
  -> bounded canary
  -> promote | hold | rollback
  -> learning ledger + next hypothesis
```

The existing Tier-3 waterfall owns candidate scheduling. Autospec does not recursively launch an unrestricted agent. It creates ordinary, auditable work that must survive the same review system it proposes to change.

### Promotion authority

Without routine operator input, Autospec may promote a review-policy change only when all conditions hold:

- the change is strengthening or behavior-neutral;
- the diff does not alter protected merge authority, credential boundaries, destructive-action policy, or the rule forbidding self-approval;
- targeted and full validation pass;
- a provider-diverse independent reviewer admits the exact commit;
- the shadow/canary sample floor is satisfied;
- high-severity escape rate is zero and total escape rate does not regress;
- cost remains within the candidate’s declared bound;
- a tested rollback handle exists.

Otherwise Autospec holds or rolls back. Protected-boundary changes remain visible proposals rather than autonomous merges.

## Failure Handling

- Unknown risk defaults upward, never to Normal.
- Missing integration evidence blocks integration-shaped work.
- Missing alternate provider blocks Critical work and records an availability outcome.
- Malformed or truncated verdicts fail review.
- Verdict/receipt/policy digest drift invalidates recovery and reruns review.
- Phase 5.5 unavailability records unknown quality and freezes policy relaxation.
- Candidate-generation errors do not mutate policy or GitHub state.
- Canary regression invokes the recorded rollback and appends the outcome.

## Compatibility and Migration

- Existing normal-risk issues continue using their primary smoke test.
- Existing integration-shaped issues may reuse a qualifying integration/smoke/e2e primary command.
- Skill bodies remain lock-step across `SKILL.md`, `opencode/agent.md`, and `codex/prompt.md`.
- Existing bare-LGTM reviewer fixtures migrate to structured artifacts; the outer merge gate still receives exact `LGTM` from the normalizer.
- Historical unattributed gaps remain readable but do not count as clean samples.

## Testing Strategy

Tests are written and observed failing before implementation.

### Native Rust

- Review-risk classification for normal, multi-component, sensitive-path, deep, and critical changes.
- Integration smoke absence, compatibility fallback, execution failure, successful evidence binding, and tamper recovery.
- Provider-diverse selection, high-reasoning fallback, and critical fail-closed behavior.
- Structured verdict rejection for wrong commit, unknown keys, empty evidence, integration-without-path, findings, truncation, and artifact replacement.
- Review receipt policy/verdict identity and recovery tamper tests.

### Shell and skill adapters

- No documented or executable in-thread review fallback on auto-merge paths.
- Lock-step triplet validation.
- Phase 5.5 attribution survives gap emission and remediation composition.
- Advisor computes escaped-defect rates and never treats cache warmth as quality.
- Advisor strengthens on attributed high-severity escapes and refuses relaxation below the sample floor.
- Failed broad review emits `review_unavailable`.

### Self-improvement

- A high-severity attributed escape creates one falsifiable, deduplicated candidate.
- Evidence-free and weakening candidates cannot enter auto-promotion.
- Repeated escape classes update frequency instead of creating duplicate work.
- Canary success promotes only after the declared sample floor.
- Canary quality or cost regression invokes rollback.
- A policy change receives its own subsequent outcome measurement.

## Acceptance Criteria

- [ ] Native review admission is derived from a commit-bound `ReviewPolicy` and structured `ReviewVerdict`.
- [ ] Integration-shaped changes cannot merge without executable integration evidence.
- [ ] High/integration reviews prefer a provider different from the implementer; Critical reviews require one.
- [ ] No skill or native auto-merge path authorizes an in-context reviewer.
- [ ] Review receipts preserve policy, reviewer identity, semantic evidence, and artifact digests.
- [ ] Phase 5.5 gaps preserve originating review attribution or explicitly mark it unavailable.
- [ ] Advisor governance uses escaped-defect rates as quality and first-pass LGTM only as a diagnostic.
- [ ] Self-improvement candidates contain evidence, falsifier, metric, sample floor, scope, and rollback.
- [ ] Only bounded strengthening or neutral review-policy changes can auto-promote.
- [ ] `cargo test --workspace` and `autospec validate` pass after implementation.

## Scope Cuts

- No second reviewer for every normal PR.
- No reviewer permission to execute tests or mutate repository state.
- No unrestricted self-modifying prompt loop.
- No cryptographic signing beyond existing private-artifact ownership and digest binding.
- No autonomous weakening of review, security, destructive-action, credential, or merge-authority invariants.

## Expected Files

The implementation plan will minimize changes around these surfaces:

- `crates/autospec-cli/src/commands/autonomous.rs`
- `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- a focused native review-policy module if extraction keeps `executor_bridge.rs` bounded
- `skills/autospec-run/` and `skills/autospec/` lock-step triplets
- `skills/autospec-shared/scripts/emit-gaps.sh`
- `skills/autospec-shared/scripts/gap-json-lib.sh`
- `skills/autospec-shared/scripts/gap-remediation-loop.sh`
- `skills/autospec-shared/scripts/advisor-observe.sh`
- `skills/autospec-shared/scripts/advisor-govern.sh`
- `skills/autospec-shared/scripts/advisor-sweep-tick.sh`
- `scripts/autonomous-self-improvement.sh`
- Rust, Bats, validation, and lock-step tests covering each changed behavior
