# CONTRACT.md — Cross-Cutting Contracts

Home for assumptions shared by more than one subsystem. A comment in an
implementing file reaches only that file's readers; an entry here reaches the
next subsystem that would otherwise re-derive the assumption independently and
fail on it.

> An assumption shared by three subsystems and stated by none is not a bug,
> it is a contract nobody wrote down.

## Entry rules

- Every entry **names the assumption**, lists the **components it binds**,
  states its **status** (held / broken / superseded), and links where it is
  implemented and where it was decided.
- When a capability breaks an entry, the entry's status and components are
  updated **in the same change** that breaks it — never silently.
- A new component bound by an existing entry is added to that entry's
  components list in the PR that introduces the component (same rule as
  [`docs/invariants.md`](docs/invariants.md)).
- An entry's failure modes must be concrete (the symptoms it produced), not
  abstract, so the next search hits it.

## C-1: A worker serves exactly one model, and that model generates text

- **Status:** broken as of #3701. Embedding-only backends (e.g. Ollama
  `bge-m3:latest`) exist on the fleet.
- **Components bound:** the three gateway layers that each assumed it
  independently — the router, the admission prober, and the model-name
  validator — in the InferWeave gateway (`berlinguyinca/autospec-inferweave`;
  discovery, admission and placement live there per
  [`docs/specs/2026-09-02-adaptive-agent-runtime-design.md`](docs/specs/2026-09-02-adaptive-agent-runtime-design.md)
  §"Not built here"). The request-side half of the same contract is
  `crates/autospec-core/src/aar/inferweave.rs`.
- **Failure modes when assumed, not written down.** Each layer fails with a
  different 4xx, and each defect was invisible until the previous layer was
  fixed — a single reading of any one layer finds none of the others:

  | layer | symptom | cause |
  |---|---|---|
  | route | `404` | `/v1/embeddings` was never routed |
  | admission | `422 admission probe failed: completion returned 400 … does not support generate` | the probe demands a one-token completion, which an embedding model refuses by design |
  | name validation | `400 invalid model name: bge-m3:latest` | the validator forbids `:`, which Ollama uses on every model |

- **Decided in:** issue #3701.

## Adding a capability: enumerate the layers before implementing

A capability crosses a stack, and it only exists when it passes **every**
layer of that stack. Before implementing, enumerate these layers and check
each one; a layer whose answer is "no" is part of the work, not a deployment
surprise:

| # | layer | check |
|---|---|---|
| 1 | route | is the new path/method actually mounted? |
| 2 | auth | do the existing authn/authz rules admit this capability's callers? |
| 3 | admission | does the admission probe assume the new capability shares the old one's request shape (a probe demanding a completion from a model that only embeds)? |
| 4 | liveness | does the health/liveness probe send a request the backend is designed to refuse? |
| 5 | routing key | does the routing-key / identifier grammar accept the backend's native identifiers (Ollama's `name:tag`)? |
| 6 | telemetry | does metrics/tracing record the new capability instead of dropping it? |

The list is short and stable; its value is that **one reading finds all the
layers**, instead of rediscovering them one 4xx at a time.

## The end-to-end path is a test, not a deployment

Every path a capability crosses needs an integration test that crosses
**all** of its layers. A test that stops at one layer passes while the path
is broken — this is exactly how #3701's three defects each passed their own
unit tests and only met at deployment.

- Before first deployment of a capability, add an integration test that
  registers the narrowest real backend for that capability (for embeddings:
  an embedding-only backend) and asks the gateway for the capability
  end-to-end, through every layer.
- Any layer that cannot be exercised in a test is a finding: fix the layer or
  file an issue. A capability is not verified by its per-layer tests alone.

## Smallest deployable slice

- Landing one layer per PR (route → probe → validator) is correct for
  review; each layer is still verified only against its own unit tests.
- A slice is **not finished when its own tests pass**; it is finished when
  the path it belongs to works — i.e. when the end-to-end test above is
  green.
- The deployable slice is the smallest change set after which the path
  crosses every layer.
