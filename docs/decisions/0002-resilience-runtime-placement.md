# ADR 0002 — Resilient runtime placement and memory-provider boundary

- **Date:** 2026-09-13
- **Status:** Accepted
- **Satisfies:** `docs/specs/2026-09-13-autospec-artificium-resilience-memory-learning-spec.md`
  §1 (preserve architectural boundaries), §3 (current architectural placement),
  §5.2 (no duplicate memory system), §9 (source-of-truth rule), §10 (security).

## Context

The implementation spec adds five capabilities: Context Guardian + structured
continuation checkpoints, a dynamic memory map, Repository Attention Streams, a
durable work protocol, and Verified Engineering Learning. It mandates that these
extend the existing architecture rather than create a competing agent runtime.

Two load-bearing facts came out of the cross-repository inventory (Phase 0):

1. **The "Black Hole"/shared-memory service is MemPalace.** `mempalace` (3.3.5) is
   installed and AutoSpec already integrates it through
   `skills/autospec-shared/scripts/mempalace-*.sh`, `inject-relevant-memory.sh`,
   `cross-repo-search.sh`, `diary-write.sh` and `auto-init-memory.sh`. There is no
   separate OpenViking binding in the checked-out source. The narrow provider
   contract must sit on top of this existing integration, not beside it.

2. **AutoSpec is the control plane that owns these contracts.** It already owns the
   AAR module (`autospec_core::aar`), which establishes the convention: pure policy
   modules return verdicts/plans/records, and callers perform I/O. The new features
   are exactly the kind of contract that belongs in the control plane.

## Decision

### Placement

Implement the five features as a new pure module `autospec_core::resilience` in
the control plane, with the same "pure module + I/O-edge command" split AAR uses:

| Capability | Core module | CLI surface |
|---|---|---|
| Context Guardian | `resilience::context_guardian` | `autospec resilience checkpoint-verdict` |
| Dynamic Memory Map | `resilience::memory_map` | (map generation is library-first) |
| Attention Streams | `resilience::attention_stream` | (library-first) |
| Durable Work Protocol | `resilience::work_protocol` | `autospec resilience transition-check` |
| Verified Learning | `resilience::learning` | `autospec resilience lesson-verdict` |
| Identities / events | `resilience::ids`, `resilience::EVENTS` | `autospec resilience events` |

Dispatch judgement stays in `autospec-dispatcher`, execution/session/lease
mechanics stay in `autospec-orchestrator`, telemetry projection stays in
`autospec-db`, and read presentation stays in `autospec-gui`. The control plane
defines the versioned serialized contracts (`autospec.context-checkpoint.v1`,
`autospec.memory-map.v1`, `autospec.attention-stream.v1`,
`autospec.work-receipt.v1`, `autospec.lesson-candidate.v1`) and their JSON
Schemas.

### Memory boundary

`resilience::memory_map` defines a narrow `MemoryProvider` trait (`search`,
`wake_up`, `available`) and a `generate_map` builder that degrades explicitly
when the provider is unavailable — it never fabricates memory. Today the provider
backing is the existing MemPalace integration; future providers (including
OpenViking) can implement the same trait without changing AutoSpec.

### Source-of-truth rule

Correctness-critical state (checkpoints, leases, receipts, work state) must live
in a durable control/execution store. `autospec-db` mirrors events for
observability only. Telemetry-off and memory-degraded modes are first-class and
tested.

### Deterministic gates

A model may interpret a failure but may never convert a deterministic failing
gate into a passing one. The work-protocol state machine rejects illegal
transitions and stale fencing generations programmatically; the Context Guardian
blocks a new substantial phase at the required threshold without a durable
checkpoint; the learning gate requires validation + review evidence before
promotion and rejects secret-bearing or policy-weakening lessons.

## Consequences

- The control plane now carries the durable-work, checkpoint, memory-map,
  attention-stream and learning contracts as pure, tested Rust.
- No new memory backend was introduced; the MemPalace integration is the memory
  foundation.
- `autospec-db` and Grafana remain optional and are never required for correctness.
- Cross-plane mechanics (orchestrator lease/checkpoint handshake, dispatcher
  judgement wiring) are follow-up work that consumes these contracts.
