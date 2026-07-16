# Rust Autonomous Waterfall and Local Ideation Design

**Issues:** #1872, #2076

## Goal

Replace the legacy idle-rescan waterfall with a Rust-owned process that records
why discovery found no safely actionable work and, only after two genuinely
complete dry passes, creates a bounded local planning backlog. It must not
confuse a failed, blocked, or unported producer with a dry producer.

## Current foundation

`autospec_core::autonomous::no_work` is the committed pure state model. It
accepts the five ordered tiers, validates closed dry reasons, retains the two
source passes, and gives a planning-only ideation request at the two-pass
threshold. It is not the waterfall itself: a manual observation cannot prove
that every discovery tier ran.

## Authority model

- Only `autospec autonomous waterfall run-once` can commit a completed pass.
- Each tier writes a sealed receipt under
  `<operator>/<repo>/waterfall/<pass>/<tier>.json`; the no-work state stores
  the derived relative references and a pass-manifest digest.
- `Failed`, `Blocked`, and `NotRun` are retained as typed outcomes and prevent
  a full-dry count. They never become `dry` during error handling.
- The first implementation of `autospec autonomous ideate` writes only local
  `ideation-backlog.json` and Markdown. It never creates an issue, label,
  branch, PR, or `auto-implement` candidate.
- No command may call a legacy shell waterfall, parse shell stdout, or use
  `sh`/`bash` as a fallback. External sources and model execution must be
  direct argument-vector children with typed JSON contracts, explicit time
  bounds, and persisted failure receipts.

## Tier contract

All five tiers use the existing `NoWorkTier` names:

| Tier | Rust completion requirement | Non-completion outcome |
| --- | --- | --- |
| `tier1` | Scan the typed ready queue and persist its page/digest. | `failed` when queue evidence cannot be read. |
| `tier1_5` | Read-only candidate promotion/grooming classifier with closed eligibility evidence. | `not_run` until its classifier and governance rules are native. |
| `tier2` | Local discovery funnel: evidence collection, deduplication, adversarial verification, ROI, and ranked proposal receipts. | `failed` on any incomplete funnel stage. |
| `tier3` | Architecture, test-coverage, and technical-debt evidence producer with deterministic result receipts. | `not_run` only when the checked-in producer is disabled by policy. |
| `tier4` | Explicitly opted-in external-source producer with allowlists, byte/time limits, untrusted evidence storage, and a typed funnel. | `not_run` when no source configuration permits it. |

The pass is fully dry only if every tier emits an `Exhausted` receipt with a
closed reason. A `Produced` receipt returns control to normal Rust readiness;
it cannot directly launch implementation.

## Receipt and state schemas

`waterfall/<pass>/<tier>.json` is schema 1 and contains repository scope,
pass, tier, producer version, started/completed timestamps, a typed status,
funnel counts, sealed evidence references, and a SHA-256 digest. Unknown or
duplicate JSON fields, scope/tier/pass mismatch, a non-derived path, or a
digest mismatch is a failure.

`waterfall-state.json` is an atomically updated, locally serialized cursor. It
contains the next pass ID, the current tier, and completed receipt digests.
It is protected by a scoped lock so concurrent foreground cycles cannot lose a
recorded pass. It is not sufficient to replace the conductor lease; the
foreground lease still gates dispatch.

`why-no-work.json` is written only after a complete pass. Existing no-work
state gives it contiguous-pass/idempotency semantics and preserves the two dry
source observations needed for an edge-triggered ideation request.

## Local ideation contract

At the exact `1 -> 2` dry-pass transition, `ideate` receives only the sealed
receipt package. A fixed prompt asks exactly:

1. What features are missing?
2. What can we do here?
3. Find 5 new features.
4. Rank them by impact and importance.
5. Which are safe to implement autonomously now?
6. Which need a planning/spec issue first?

It validates closed JSON: at most five candidates; normalized title; only
sealed evidence references; integer impact, importance, risk, and effort in
`1..=5`; and a disposition of `review_required` or `planning_required`.
Candidates are deterministically sorted by impact × importance, then lower
risk, lower effort, and title. The output carries `remote_mutation: "none"`.
An unavailable or malformed model result records an ideation failure and leaves
the no-work state intact; it does not create a placeholder backlog.

## Foreground integration

When Rust foreground finds no ready Tier-1 work, it starts or resumes one
waterfall pass under the current local conductor lease. It persists each tier
before moving to the next. It releases no claim because no waterfall stage
owns an implementation claim. A full pass writes no-work state; an edge
ideation request invokes the local-only ideation command once for the two
source pass IDs. A produced result returns to `Scan` and the normal ready queue.

## Legacy deletion gate

The legacy waterfall remains only until all five native producer contracts,
receipt validation, foreground integration, local ideation, and source-authority
tests exist. #2076 cannot delete it after merely adding the state foundation.
