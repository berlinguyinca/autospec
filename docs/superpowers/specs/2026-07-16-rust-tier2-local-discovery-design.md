# Rust Tier 2 Local Discovery Design

**Parent design:** `docs/superpowers/specs/2026-07-16-rust-autonomous-waterfall-design.md`
**Scope:** #1872 native Tier 2 only

## Goal

Replace the legacy shell-driven Tier 2 discovery path with a Rust-owned local
evidence collector and deterministic proposal funnel that never turns an
incomplete generator or verifier into a dry result.

## Decision

Deliver Tier 2 in two authority-separated parts.

1. The first part is fully native and model-independent: a strict no-cache
   local-signal collector, a pure typed funnel, canonical evidence documents,
   and a receipt coordinator.
2. The production model child is disabled by checked-in policy until its direct
   argument-vector protocol is version-pinned and proves bounded execution,
   strict result compatibility, read-only behavior, and network policy. While
   disabled it writes a sealed `NotRun` receipt; it never emits an empty
   `Exhausted` result.

This is deliberately more honest than the legacy explorer, which converted
command errors and missing verification into a dry pass. It also avoids
pretending that a domain lexicon match proves an implementation defect.

## Rejected approaches

- Reuse `autospec-explore`, `bash -c`, the specialist cache, environment-fed
  stub data, or legacy verifier scripts. They retain shell, cache, ambient
  configuration, and sometimes GitHub/file-mutation authority.
- Treat a detector signal as an actionable issue or as adversarial
  verification. A signal is evidence of a domain, not proof of a gap.
- Enable `codex exec` immediately. The installed CLI supports direct argv,
  read-only sandbox selection, JSONL, and response schemas, but its documented
  interface supplies no deadline, output cap, stable event schema, or direct
  network-disable guarantee.

## Architecture

### Pure core

`autospec_core::autonomous::tier2` owns only deterministic types and
evaluation. It has no filesystem, environment, process, network, GitHub,
queue, claim, label, branch, worktree, PR, foreground, or waterfall-store
authority.

The pure input consists of:

- a versioned `Tier2CollectorEvidence` snapshot containing sorted
  `DetectedDomain` and `FileLineEvidence` records;
- typed generator proposals with nonempty stable keys, titles, named consumers,
  collector-contained evidence, closed severity/complexity, and bounded integer
  confidence;
- exactly one typed `Survived` or `Refuted` verdict for every deduplicated key;
- a small closed policy including the maximum five ranked candidates.

`evaluate_tier2` validates the complete input, rejects conflicting duplicates
and incomplete verdict coverage, then produces a canonical observation with
candidate, deduplication, verification, ROI, and rank decisions. It uses only
integer score ordering: severity ascending, `confidence / complexity_units`
descending (`1`, `2`, or `4`), then stable key ascending. It does not create
issues or claim that a candidate can be auto-implemented.

### Strict local collector

The collector is a new read-only sibling of the existing specialist scanner.
It shares the checked-in lexicon, bounded path traversal, token matching, and
stable sorting, but never reads or writes `.autospec/explore-specialists.json`,
never reads `AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT`, and returns selected-root,
containment, directory, and signal-file failures explicitly. The existing
cache-backed `scan_specialists` API remains unchanged for compatibility.

The collector owns its own `StrictCollectorEvidence` and
`StrictCollectorError` types in `explore::specialists`; the Tier 2 core consumes
that evidence one-way. This prevents a collector-to-autonomous dependency cycle
while keeping its read-only API reusable. It must not call the `explore` CLI,
legacy scripts, or a proposal generator/verifier.

### CLI evidence and state boundary

The CLI owns filesystem persistence and the waterfall lock only. It persists
immutable, digest-checked evidence before a Tier 2 receipt, then persists the
receipt before advancing the cursor. Recovery re-verifies every referenced
artifact before trusting a receipt.

Tier 2 artifacts live under `waterfall/<pass>/tier2/`:

| Artifact | Contents |
| --- | --- |
| `policy.json` | closed policy and the disabled/active producer identity |
| `collector.json` | strict local scan policy, canonical scope, domains, line evidence |
| `generated.json` | generator identity, sealed input, and typed proposals |
| `dedup.json` | normalization version, winners, suppressions, and conflicts |
| `verification.json` | total verdict coverage and bounded reasons |
| `roi-rank.json` | integer score inputs, ROI decisions, rank tuple, cap, candidates |
| `failure.json` | bounded stage and diagnostic code when a stage cannot complete |

Only artifacts relevant to the terminal result are referenced. A policy-disabled
Tier 2 receipt references `policy.json`; it does not manufacture empty stage
artifacts.

## Terminal outcomes

| Condition | Receipt status | Cursor behavior |
| --- | --- | --- |
| Checked-in policy disables the runner | `NotRun { tier2_local_discovery_disabled_by_policy }` | retain Tier 2 |
| Collector, generator, dedupe, verifier, ranking, or evidence persistence cannot complete | `Failed { tier2_<stage>_... }` | retain Tier 2 |
| Completed generator returns no valid proposals | `Exhausted { NoProposalsGenerated }` | advance to Tier 3 |
| Every deduplicated proposal is refuted | `Exhausted { VerificationRejected }` | advance to Tier 3 |
| ROI removes every verified proposal | `Exhausted { RoiFiltered }` | advance to Tier 3 |
| At least one candidate survives rank/cap | `Produced { count > 0 }` | retain Tier 2 and return to normal Rust readiness |

`Produced`, `Failed`, and `NotRun` never advance the pass. Only a sealed
`Exhausted` receipt advances the waterfall.

## Future model-runner activation gate

A later, separate activation change may add a narrow CLI-local model adapter
only after it provides all of the following:

1. an allowlisted absolute executable and version policy;
2. fixed direct argv roles for generation and verification, never a shell;
3. a parent-enforced deadline, concurrent capped stdout/stderr capture,
   termination, and reaping;
4. strict versioned final-result codecs and a compatibility test for the child
   event/final-result protocol;
5. an integration proof that its configured sandbox denies repository writes;
6. a verified network policy that distinguishes required model transport from
   model-initiated external retrieval;
7. static authority guards excluding `sh`, `bash`, `zsh`, `omx`, `gh`, legacy
   explorer names, environment proposal input, queue/claim, labels, branches,
   worktrees, PRs, and GitHub mutation.

Until that gate is met, a direct model execution is out of scope. A test fake
may supply complete typed outputs to prove the CLI-to-core contract without
granting production process authority.

## Verification

- Strict collector tests cover deterministic order, bounded evidence,
  containment, unreadable selected inputs, and no cache/environment/write
  authority.
- Pure-funnel tests cover schema validation, conflicting duplicates, stable
  winner selection, total verifier coverage, all-refuted and all-ROI-filtered
  outcomes, rank/cap order, and canonical JSON.
- CLI tests cover evidence-before-receipt ordering, replay/tamper rejection,
  each terminal cursor rule, disabled-policy behavior, and zero GitHub/queue/
  claim/label side effects.
- Static tests reject legacy shell and promoter authority. Formatting, scoped
  clippy, package tests, and `autospec validate --fast` must pass.
