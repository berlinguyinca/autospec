# Rust Tier 4 External Discovery Design

**Parent design:** `docs/superpowers/specs/2026-07-16-rust-autonomous-waterfall-design.md`
**Scope:** #1872 Task 7 native foundation; no live retrieval or foreground wiring

## Goal

Add a Rust-owned, receipt-backed Tier 4 contract for strictly configured external
discovery so unavailable, malformed, failed, or untrusted source evidence never
becomes a dry pass or creates remote work.

## Decision

Tier 4 has three authority-separated layers.

1. `autospec_core::autonomous::config` owns the strict repository-owned Tier 4
   source descriptor schema. Parsing a descriptor does not enable retrieval.
2. `autospec_core::autonomous::tier4` owns pure typed-source validation,
   proposal normalization, deterministic deduplication/verification/ROI ranking,
   partial failure context, and evaluator-sealed canonical documents.
3. The CLI owns local receipt persistence and replay. Its V1 production adapter
   constructs only `Tier4Input::DisabledByCheckedInPolicy` and seals exact
   `NotRun { reason: "tier4_external_discovery_disabled_by_checked_in_policy" }`
   evidence. Tests inject typed source and stage results.

The foundation deliberately does not fetch a URL. Neither Rust crate currently
has an approved HTTP/TLS/URL dependency; shell, `curl`, legacy explorers, model
children, and GitHub issue filing are prohibited. A later activation needs a
separate transport/security decision and a source revalidation design.

## Strict source configuration

`.autospec/autonomous.yml` gains an optional top-level `tier4` mapping. Its
absence is valid and yields an empty descriptor list. If present, it has exactly
one `sources` block list of one through four descriptors:

```yaml
tier4:
  sources:
    - id: release-feed
      host: api.example.test
      path: /v1/releases
      max_bytes: 65536
      deadline_millis: 5000
```

`id` is a unique lower-kebab identifier of at most 64 ASCII bytes. `host` is a
unique lowercase ASCII DNS name with no scheme, port, userinfo, wildcard,
whitespace, or IP literal. `path` starts with `/`, is at most 256 bytes, has no
query/fragment/backslash, and has no empty, `.` or `..` segment. `max_bytes` is
an unsigned decimal integer from 1 through 1,048,576; `deadline_millis` is an
unsigned decimal integer from 100 through 30,000. The protocol is fixed to
HTTPS and is never user-configurable.

The existing `main_health` parser behavior remains unchanged. Tier 4 rejects
duplicate blocks, fields, IDs, malformed indentation, non-scalar values, inline
collections, unknown Tier 4 fields, and all invalid bounds. Unrelated top-level
policy remains ignored. Parsed descriptors are data only: no config value enables
network access in V1.

## Pure typed funnel

```rust
pub fn evaluate_tier4(input: Tier4Input) -> Result<Tier4Evaluation, Tier4Failure>;
```

`Tier4Input::Enabled` is test-only injected data: a canonical source-policy
identity, ordered typed source envelopes, generated candidates, verifier
verdicts, and a fixed ROI policy. Source envelopes carry only a descriptor ID,
schema/versioned producer identity, byte length, sealed body digest, and typed
candidate facts; raw response bytes never enter the core or public documents.
Every configured source must complete exactly once in descriptor order. Missing,
duplicate, malformed, oversized, conflicting, or failed source/stage input is a
closed `Tier4Failure`, never a dry result.

Candidates are source-attributed, bounded, and canonicalized only after all typed
input validates. The core deduplicates on a closed stable candidate key, requires
complete verifier coverage, applies a fixed ROI threshold, ranks by descending
ROI then stable key, and caps the rank at ten. The funnel is monotonic:
`observed >= deduplicated >= verified >= roi_approved >= ranked`.

Only a fully completed typed funnel may produce an exhausted result:

| Condition | Tier status |
| --- | --- |
| no generated candidates | `Exhausted(NoProposalsGenerated)` |
| all candidates rejected by verifier | `Exhausted(VerificationRejected)` |
| verified candidates rejected by ROI | `Exhausted(RoiFiltered)` |
| ranked candidates | `Produced { count }` |
| any source/stage/config validation error | `Failed` |

`Produced` is planning evidence only. It never creates issues, labels, claims,
branches, worktrees, PRs, executor requests, or remote mutations. The pure core
has no filesystem, environment, process, network, HTTP, GitHub, queue, claim,
branch, worktree, foreground, `WaterfallStore`, or model authority.

## Sealed receipts and replay

Tier 4 evidence lives at `waterfall/<pass>/tier4/`.

| Artifact | Contents |
| --- | --- |
| `policy.json` | exact checked-in disabled policy identity |
| `source_policy.json` | canonical configured descriptor identities and fixed limits |
| `sources.json` | typed source envelope metadata and sealed body digests, never raw bytes |
| `generated.json` | canonical generated candidates and predecessor digest |
| `dedup.json` | canonical deduplication groups and predecessor digest |
| `verification.json` | canonical verifier verdicts and predecessor digest |
| `roi_rank.json` | ranked candidates, cap, funnel, and predecessor digest |
| `failure.json` | closed stage/code/status reason, bounded detail, zero funnel, predecessor digest |

Every referenced document is schema-one canonical one-line JSON ending in
`\n`, with strict lexical framing, exact keys, ordered references, digest checks,
and predecessor links. Unreferenced evidence from an interrupted pre-receipt
write is ignored. The V1 disabled receipt has exactly one `policy.json` artifact,
the producer `rust-tier4-disabled-policy-v1`, and an all-zero funnel.

Enabled source evidence has no raw response body in a receipt. A future live
adapter may store a byte-capped opaque body file only after its own source,
privacy, retention, and revalidation contract is approved; that activation is
outside this design.

The coordinator persists evidence, verifies it, persists the receipt, then
updates the cursor. It replays an existing receipt before scanning. Only a
completed Tier 4 receipt with one of the three closed exhausted reasons can be
recorded into `WaterfallState` and roll to the next Tier 1 pass. `NotRun`,
`Produced`, `Failed`, `Blocked`, and every other exhausted reason retain Tier 4;
hand-forged completed state using those statuses is rejected during load and
persist replay.

## Authority and activation boundary

Production Tier 4 is disabled even when descriptors parse successfully. Tests
may inject typed completed and failed source/stage inputs; they must not mock a
network client or execute a local server. Static authority tests recursively scan
the core, policy adapter, receipt coordinator, and all Tier 4 verifier helpers.
Only the local receipt store and read-only replay readers are allowed I/O.

V1 does not implement direct retrieval, body storage, model ideation, local
ideation command wiring, foreground waterfall traversal, source activation,
Tier 2/Tier 3 activation, executor parity, validation/installer replacement, or
legacy deletion. Those remain separate gates. In particular, a disabled Tier 4
receipt cannot contribute to a complete dry pass or the two-pass local ideation
trigger.

## Verification

- Config tests prove exact parsing, absence behavior, all relevant malformed
  shapes, descriptor bounds, duplicates, and that parsing cannot enable retrieval.
- Core tests prove source/stage precedence, canonicalization, deduplication,
  verifier coverage, ROI/rank cap, every exhausted reason, sealed documents,
  and no authority leaks.
- CLI tests prove disabled, all completed terminal outcomes, every failure
  prefix, evidence-before-receipt-before-cursor ordering, replay/tamper rejection,
  forged-state retention, and no direct process/network/GitHub/queue action.
- Formatting, scoped clippy, package tests, native fast validation, and
  `git diff --check` pass. Every new Tier 4 source or test file stays at 450
  lines or fewer.
