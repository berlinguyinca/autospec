# Rust Tier 3 Metadata Producer Design

**Parent design:** `docs/superpowers/specs/2026-07-16-rust-autonomous-waterfall-design.md`  
**Scope:** #1872 Task 6 foundation only

## Goal

Add a Rust-owned Tier 3 contract for deterministic architecture, coverage, and
technical-debt metadata. It must seal truthful outcomes and never turn missing,
malformed, or unavailable metadata into a dry pass.

## Decision

Tier 3 has two authority-separated layers.

1. `autospec_core::autonomous::tier3` owns pure validation, canonicalization,
   deduplication, ranking, partial failure context, and canonical documents.
2. The CLI owns local receipt persistence and replay. Its checked-in V1
   production policy is `NotRun { reason:
   "tier3_metadata_disabled_by_checked_in_policy" }`; tests inject typed
   completed and failed metadata stages.

There is no trustworthy native source for coverage, duplicate-code, churn, or
complexity measurements today. A future activation must add typed repository
configuration and a strictly validated checked-in metadata package; it is not
permitted to call the legacy shell workstreams, parse prose, or infer those
facts from source keywords.

## Rejected approaches

- Reuse `architecture-fitness.sh`, `technical-debt-workstream.sh`,
  `test-quality-workstream.sh`, `autospec-explore`, or the shell loop. They
  execute shell commands, parse ambient output, and can file GitHub issues.
- Treat source keyword matches or unstructured self-improvement reports as
  architecture, coverage, or debt findings. They cannot establish a bounded,
  reproducible measurement.
- Treat disabled metadata, a missing input, a malformed stage, or an adapter
  error as empty evidence. Each remains `NotRun` or typed `Failed`.

## Pure model

The core contains no filesystem, environment, process, network, GitHub,
queue, claim, label, branch, worktree, PR, foreground, or `WaterfallStore`
authority.

```rust
pub fn evaluate_tier3(input: Tier3Input) -> Result<Tier3Evaluation, Tier3Failure>;
```

`Tier3Input::Enabled` carries closed `Tier3StageResult::{Complete, Failed,
Missing}` values for architecture, coverage, and debt, evaluated in that order.
A stage failure takes precedence over later completed or empty stages; a missing
result returns the matching closed failure. `Tier3Input::DisabledByCheckedInPolicy`
returns only the exact V1 `NotRun` reason and no observation.

Each completed adapter supplies `Tier3AdapterEvidence` with a schema version,
adapter/rule version, and sorted findings. A finding has a closed kind
(`Architecture`, `Coverage`, or `Debt`), a versioned rule ID, bounded
root-relative UTF-8 path, positive line, bounded message, and closed severity.
The core rejects absolute or escaping paths, blank/oversized fields,
wrong-kind stage evidence, noncanonical input ordering, duplicate/conflicting
keys, and count overflow. It canonicalizes only already-validated evidence.

Findings deduplicate by `(kind, rule_id, path, line, message)`, rank by
severity ascending, rule ID ascending, path ascending, line ascending, then
message ascending, and cap at ten. A completed empty set is valid. Funnel
counts are `observed >= deduplicated >= verified >= roi_approved >= ranked`;
Tier 3 uses `verified == roi_approved == deduplicated`, while `ranked` may be
smaller only because of the fixed cap. It has no model verifier or ROI
inference.

`Tier3Failure` carries only validated predecessor evidence: no predecessor for
architecture; architecture before coverage; architecture and coverage before
debt; and all three before the closed `Ranking` stage. An opaque, evaluator-derived
`Tier3EvidenceDocuments` is the only public document-rendering surface. Raw
adapter structs cannot be serialized as sealed evidence.

## CLI receipts and replay

Tier 3 artifacts live below `waterfall/<pass>/tier3/`:

| Artifact | Contents |
| --- | --- |
| `policy.json` | checked-in disabled policy identity and exact reason |
| `architecture.json` | sealed architecture adapter evidence |
| `coverage.json` | sealed coverage adapter evidence and architecture digest |
| `debt.json` | sealed debt adapter evidence and coverage digest |
| `findings.json` | canonical deduplicated/ranked findings and debt digest |
| `failure.json` | closed stage, code, bounded detail, funnel, predecessor digest |

Every document is schema-one, canonical one-line JSON ending in `\n`, and is
validated on replay with strict parsing, canonical lexical framing, exact keys,
exact reference order, digest checks, and predecessor links. Only referenced
files participate in replay; unreferenced files left by an interrupted
pre-receipt write are tolerated.

| Terminal result | Ordered receipt evidence | Cursor |
| --- | --- | --- |
| `NotRun` (disabled policy) | policy | retain Tier 3 |
| `Exhausted(NoMetadataFindings)` | architecture, coverage, debt, findings | advance Tier 4 |
| `Produced { count }` | architecture, coverage, debt, findings | retain Tier 3 |
| failed architecture | failure | retain Tier 3 |
| failed coverage | architecture, failure | retain Tier 3 |
| failed debt | architecture, coverage, failure | retain Tier 3 |
| failed ranking | architecture, coverage, debt, failure | retain Tier 3 |

The coordinator persists evidence before the receipt and the receipt before the
cursor. A replay, persistence, or integrity error creates no replacement
receipt and leaves the cursor unchanged. `Produced` findings are planning
evidence only; they never create issues, labels, claims, branches, PRs, or
implementation work.

## Activation boundary

V1 production constructs only `DisabledByCheckedInPolicy`. A later activation
may consume a typed metadata package only after #1602 supplies a strict native
configuration surface and the package contract defines producer identity,
snapshot digest, field limits, rule allowlists, symlink/special-file rejection,
and revalidation after reads. It must remain a read-only local adapter.

This design does not activate Tier 2's model child, wire foreground waterfall
progression, implement Tier 4 or ideation, run a shell command, or delete
legacy code. Those remain separately gated work.

## Verification

- Core tests prove closed stage precedence, validation, deterministic dedup and
  rank order, partial failure prefixes, count invariants, canonical documents,
  and no authority leaks.
- CLI tests prove disabled, exhausted, produced, and every failure cursor rule;
  evidence-before-receipt-before-cursor ordering; replay and tamper rejection;
  exact artifact references; and no direct process/network/GitHub/queue action.
- Formatting, scoped clippy, package tests, native fast validation, and
  `git diff --check` pass. Every new Tier 3 source or test file stays at 450
  lines or fewer.
