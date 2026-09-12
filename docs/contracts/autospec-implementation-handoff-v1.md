# `autospec.implementation-handoff.v1` — producer, consumer, and receipt contract

Single source of truth for the top-level implementation gateway of automatic
spec projects (issue #3425 family: producer #3440, conformance #3441).

- **Source spec:** [`docs/specs/2026-08-31-automatic-spec-projects-design.md`](../specs/2026-08-31-automatic-spec-projects-design.md) — "top-level implementation gateway"
- **Scope of this doc:** the producer response schema, the consumer-trace
  schema the conformance suite validates, and the versioned receipt schema
  (schema version 1) the deployment-owned serverless consumer publishes.
  The deployment-owned consumer edit itself is out of scope for this
  repository; the conformance command and this contract are not.

`autospec.implementation-handoff.v1` is the **sole** external implementation
handoff. There is no mutating fallback: when the producer reports
`route: null` with a typed unavailable reason, zero implementation mutations
are permitted.

## Producer schema (`autospec.implementation-handoff.v1`)

Produced by `autospec handoff probe --repo OWNER/NAME --intent TEXT
[--artifact issue:N|spec:PATH] [--correlation ID] [--intent-kind
implement|explain|plan] [--repo-dir PATH]`
(`crates/autospec-cli/src/commands/handoff.rs`). Side-effect-free: the
producer probes the installed Autospec workflow surface and local run-state
read-only; it never creates branches, pushes, opens PRs, or merges.

| field | required | value |
|---|---|---|
| `schema` | yes | literal `autospec.implementation-handoff.v1` |
| `availability` | yes | `available` \| `unknown` \| `unavailable` |
| `unavailable_reason` | no | typed reason when unavailable/fail-closed (`cli_missing`, `workflow_surface_missing`, `workflow_surface_incompatible`, `probe_transient`, `run_state_ambiguous`) |
| `guidance` | no | operator guidance string |
| `route` | yes | `run` \| `start` \| `split_then_run` \| `recover` \| `none`, or `null` when blocked |
| `artifact` | yes | `{ kind: issue\|spec\|none, ref }` or `null` |
| `run` | yes | `{ run_id, entry_point, follow_up, status: proposed\|recovered }` or `null` |
| `project` | yes | `{ key, state: planned\|recovered }` or `null` |
| `stream` | yes | `{ stream_id }` or `null` |
| `cancellation` | yes | `{ token }` or `null` |
| `correlation_id` | yes | caller-supplied or deterministically derived |

The consumer must consume `run` identity and `cancellation.token` as
machine-readable values. Scraping terminal prose or synthesizing claims,
branches, PR state, or Project state is a conformance violation.

## Consumer-trace schema (`autospec.implementation-handoff-consumer-trace.v1`)

A recorded trace of the **deployed** consumer handling implementation
requests. Recorded, not mocked: the fixture is captured from the deployed
revision and replayed hermetically.

| field | required | value |
|---|---|---|
| `schema` | yes | literal `autospec.implementation-handoff-consumer-trace.v1` |
| `consumer_revision` | yes | the deployed consumer revision the trace was recorded from |
| `events` | yes | array of typed event objects |

Event types:

| `type` | fields | meaning |
|---|---|---|
| `handoff_request` | `schema`, `route` | the consumer received an `autospec.implementation-handoff.v1` response |
| `dispatch` | `target`, `via` | an implementation dispatch occurred; `via` must be `autospec.implementation-handoff.v1` |
| `status` | `source` | status was consumed; the only typed source is `run-identity` |
| `cancellation` | `source` | cancellation was exercised; the only typed source is `cancellation-token` |
| `direct_dispatch` | `agent` | a direct implementation-agent dispatch occurred — **always a conformance failure** |

## Receipt schema (`autospec.implementation-handoff-receipt.v1`)

Published by the deployment-owned consumer, tied to its deployed revision.
Schema version 1.

| field | required | value |
|---|---|---|
| `schema` | yes | literal `autospec.implementation-handoff-receipt.v1` |
| `handoff_schema` | yes | literal `autospec.implementation-handoff.v1` (the sole handoff the consumer uses) |
| `consumer_revision` | yes | the deployed consumer revision this receipt covers |
| `checks` | yes | `{ direct_dispatch_unreachable: true, status_typed: true, cancellation_typed: true }` — each must be boolean `true` |
| `signature` | yes | sha256 hex of the canonical compact JSON (keys sorted) of the receipt **with the `signature` field removed** |

Signing rule: the consumer signs the canonical form; the conformance suite
recomputes it and **rejects** any mismatch. The suite never recomputes a
passing signature in place of a failing one — there is no receipt
fabrication path.

## Verdict schema (`autospec.implementation-handoff-receipt-verdict.v1`)

Emitted by `autospec handoff conformance --receipt PATH --trace PATH`
(`crates/autospec-cli/src/commands/handoff/conformance.rs`). Read-only: no
deployment mutation, no external repository guess.

| field | required | value |
|---|---|---|
| `schema` | yes | literal `autospec.implementation-handoff-receipt-verdict.v1` |
| `admitted` | yes | boolean — true only when every check passes |
| `findings` | yes | array of `{ code, detail }` typed findings |
| `prerequisite` | yes | `{ state: satisfied \| blocked, detail }` — the portfolio blocked-prerequisite projection |

Finding codes: `RECEIPT_MALFORMED`, `RECEIPT_SCHEMA_MISMATCH`,
`HANDOFF_SCHEMA_MISMATCH`, `RECEIPT_SIGNATURE_MISMATCH`,
`TRACE_MALFORMED`, `TRACE_SCHEMA_MISMATCH`, `REVISION_MISMATCH`,
`PROTOCOL_NOT_CONSUMED`, `DIRECT_DISPATCH_REACHABLE`, `STATUS_UNTYPED`,
`CANCELLATION_UNTYPED`, `RECEIPT_CHECK_MISMATCH`.

Exit status: `0` when admitted, `1` when rejected, `2` on diagnostic input
errors.

## Contract rules

- **Reject on stale revision.** A receipt whose `consumer_revision` does not
  equal the deployed consumer revision in the trace is stale: the receipt
  describes a revision that is no longer deployed and is rejected
  (`REVISION_MISMATCH`).
- **Reject on reachable direct dispatch.** Any `direct_dispatch` event, or
  any `dispatch` event whose `via` is not the handoff schema, rejects the
  receipt (`DIRECT_DISPATCH_REACHED` is reported as
  `DIRECT_DISPATCH_REACHABLE`).
- **Reject on untyped status or cancellation.** Status must be consumed from
  the typed run identity (`run-identity`) and cancellation must use the
  typed cancellation token (`cancellation-token`); any other source, or a
  missing event, rejects the receipt.
- **No mutating fallback.** The conformance command and the producer are
  side-effect-free. Nothing in this contract authorizes a consumer to bypass
  the protocol.
- **Portfolio prerequisite.** Repository-local completion remains **Blocked**
  until the receipt is admitted against the deployed consumer revision; the
  verdict's `prerequisite` field is the projection the portfolio consumes.
  A rejected verdict blocks with every finding code named.
