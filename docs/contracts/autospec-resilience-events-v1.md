# `autospec.resilience.events.v1` — resilient runtime lifecycle events

Stable, additive event names for the Resilient Agent Runtime. Telemetry mirrors
these for observability; **correctness never depends on them being delivered**.
`autospec-db` and Grafana are optional and lossy by design.

- **Source spec:** [`docs/specs/2026-09-13-autospec-artificium-resilience-memory-learning-spec.md`](../specs/2026-09-13-autospec-artificium-resilience-memory-learning-spec.md)
- **Machine-readable source of truth:** `autospec_core::resilience::EVENTS`
  and `autospec resilience events`.

## Event names

### Context Guardian
| event |
|---|
| `context.threshold_reached` |
| `checkpoint.requested` |
| `checkpoint.persisted` |
| `checkpoint.acknowledged` |
| `execution.resumed` |

### Work protocol
| event |
|---|
| `work.assigned` |
| `work.delivered` |
| `claim.acquired` |
| `claim.renewed` |
| `claim.expired` |
| `attempt.started` |
| `attempt.completed` |
| `validation.completed` |
| `review.completed` |

### Attention streams
| event |
|---|
| `attention.started` |
| `attention.progressed` |
| `attention.completed` |

### Memory map
| event |
|---|
| `memory.map_generated` |
| `memory.retrieved` |

### Verified learning
| event |
|---|
| `lesson.candidate_created` |
| `lesson.validated` |
| `lesson.promoted` |
| `lesson.rejected` |

## Contract rules

- **Additive-only.** New event names may be appended; existing names never change
  meaning.
- **Telemetry-off is the default and is fully supported.** With `AUTOSPEC_DB_DSN`
  unset, the emit path is a no-op and every feature still works.
- **Events are a projection, not a source of truth.** A checkpoint is durable when
  the control/execution store acknowledges it, not when `checkpoint.persisted` is
  emitted.
