# `autospec.protocol` v1 — shared protocol contract

Single source of truth for the envelope that every message of the autospec
shared protocol carries. Rust and TypeScript implementations both follow this
contract and the envelope schema; neither side may be more lenient than it.

- **Source spec:** [`docs/specs/2026-10-14-pi-autospec-autonomous-delivery-design.md`](../specs/2026-10-14-pi-autospec-autonomous-delivery-design.md) — §5 R-PROTO (shared protocol), R-EVENT (observability contract) and R-RES (`ResourceRecord`)
- **Envelope schema:** [`schemas/autospec-protocol-envelope.v1.schema.json`](../../schemas/autospec-protocol-envelope.v1.schema.json)
- **Entities** (separate schemas, defined by later issues): Mission, Task, Execution, Worker, Event, ResourceRecord, Checkpoint, Lease, Budget.
- **Scope of this doc:** the envelope fields, version negotiation, fail-closed
  rules, per-mission sequence, idempotency, the allowed schema keyword subset
  and the mapping of existing seed contracts. Entity field lists and the Rust
  and TypeScript code are out of scope here.

Every message is one JSON object whose `schema` is
`autospec.protocol.<entity>.v<N>` (for example `autospec.protocol.event.v1`).
The envelope fields below are present on every message, next to the entity's
own fields.

| field | required | value |
|---|---|---|
| `schema` | yes | `autospec.protocol.<entity>.v<N>` |
| `protocol_version` | yes | integer >= 1, the negotiated protocol major version |
| `idempotency_key` | yes | string, unique within one `mission_id` |
| `mission_id` | yes | string, scope of the sequence |
| `sequence` | yes | integer >= 1, monotonic per `mission_id` |
| `required_extensions` | yes | array of `extensions` key names the reader must understand (empty array when none) |
| `extensions` | no | object, the only home of optional unknown data |

## Version negotiation

1. Each peer advertises `supported_versions`: a non-empty array of integers
   >= 1 (the protocol major versions it can read and write).
2. The negotiated version is the **highest** value present in both lists.
3. When the lists share no value, or either list is empty or malformed, the
   exchange fails with `PROTOCOL_VERSION_UNSUPPORTED`. There is no fallback
   to a lower or "best effort" version.
4. After negotiation every message carries the negotiated value as
   `protocol_version`. A message whose `protocol_version` differs from the
   negotiated one is rejected with `PROTOCOL_VERSION_UNSUPPORTED`.
5. The `v<N>` in `schema` is the entity schema's own version. Protocol
   version 1 defines every entity at `v1`; an entity version that the
   negotiated protocol version does not define is rejected with
   `SCHEMA_VIOLATION`.

## Fail-closed fields

A reader never guesses at data it does not understand.

- Envelope and entity schemas are closed (`additionalProperties: false`). An
  unknown top-level key, a missing required key, a wrong type or an `enum`
  or `const` mismatch is rejected with `SCHEMA_VIOLATION`.
- Optional unknown data lives only under `extensions`. A reader ignores any
  `extensions` key it does not understand **unless** the key is named in
  `required_extensions`.
- A `required_extensions` entry the reader does not understand rejects the
  whole message with `UNKNOWN_REQUIRED_FIELD`. No part of the message is
  applied, journaled as accepted, or acknowledged.
- A `required_extensions` entry that has no matching key in `extensions` is a
  malformed message and is rejected with `SCHEMA_VIOLATION`.
- `required_extensions` itself is a required field. An absent list is not
  treated as "none".
- Error codes are stable machine strings: `PROTOCOL_VERSION_UNSUPPORTED`,
  `UNKNOWN_REQUIRED_FIELD`, `SCHEMA_VIOLATION`, `SEQUENCE_GAP`,
  `SEQUENCE_CONFLICT`, `IDEMPOTENCY_KEY_REUSED`.
- `SCHEMA_VIOLATION` is the class of every structural refusal. Readers report it with one
  of two specific machine codes: `MISSING_FIELD` (a required key is absent, including
  `required_extensions`) and `INVALID_FIELD` (wrong type or value, an unknown key, a
  non-object message, `sequence < 1`, or a `required_extensions` entry with no matching
  `extensions` key). The Rust reader (`protocol::negotiate`) emits these two; a reader
  that reports plain `SCHEMA_VIOLATION` is equally conforming. Golden fixtures assert the
  class, not the specific code.

## Sequence and idempotency

The sequence rule reuses the evaluation journal rule
(`crates/autospec-core/src/evaluation/store/journal.rs`, `Journal::open`): the
first message of a mission has `sequence` 1, every later one advances by exactly
1, and a gap is an integrity failure.

For each `mission_id` a consumer keeps `next` (the next expected sequence,
initially 1) and the set of accepted `(idempotency_key, sequence)` pairs.

Rules are evaluated top to bottom; the first match wins.

| # | incoming message | outcome |
|---|---|---|
| 1 | key seen at a different `sequence` | reject `IDEMPOTENCY_KEY_REUSED` |
| 2 | key seen at the same `sequence` | **duplicate**: acknowledge again, apply no effect, leave `next` unchanged |
| 3 | `sequence == next` | accept, apply the effect once, record the pair, `next += 1` |
| 4 | `sequence > next` | reject `SEQUENCE_GAP`, apply nothing, leave `next` unchanged; the producer redelivers in order |
| 5 | `sequence < next` | reject `SEQUENCE_CONFLICT` (that position is already taken by a different key) |

Delivery is at-least-once, so a duplicate is a normal outcome and never an
error. The first accepted message at a position is authoritative; a later one
never replaces it. Sequence numbers and keys are scoped to one `mission_id` and
are never compared across missions.

## Schema subset

Protocol schemas use JSON Schema draft 2020-12 restricted to this keyword
subset, so both toolchains can implement the validator without a full
JSON Schema engine:

`type properties required additionalProperties enum const items $ref($defs) minimum default title description $schema $id $defs`

- `$ref` resolves only to a local `#/$defs/<name>` entry. An entity schema that
  needs an envelope field declares an identical `$defs` entry of its own.
- Any other keyword (including string-shape keywords such as `pattern`,
  `format` and length bounds, and composition keywords such as `oneOf`) is not
  part of the subset. A schema that uses one fails to load; a validator never
  ignores it silently.
- Shapes the subset cannot express, such as the `autospec.protocol.<entity>.v<N>`
  form of `schema` and the cross-field rules in this document, are checked in
  code by both readers and covered by shared golden fixtures.

## Seeds

Existing contracts seed the entities. They are mapped, not replaced.

| seed | maps to |
|---|---|
| `autospec.implementation-handoff.v1` ([contract](autospec-implementation-handoff-v1.md)) | Task (the unit of work handed to an implementer) |
| `autospec.work-receipt.v1` (`schemas/autospec-work-receipt.schema.json`) | Event: the receipt `idempotency_key` is the envelope `idempotency_key` (the producer `idem-` prefix is a convention, not enforced by the subset) |
| `autospec.context-checkpoint.v1` (`schemas/autospec-context-checkpoint.schema.json`) | Checkpoint |
| evaluation journal `sequence` (`journal.rs`) | envelope `sequence` and the gap rule above |
| `autospec.events.v1` `event_uuid` ([contract](autospec-events-v1.md)) | precedent for the idempotency key |
