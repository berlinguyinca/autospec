# Adaptive Agent Runtime (AAR) — implementation design

Status: phase 1 landed (foundation), phases 2–4 partially landed
Date: 2026-09-02
Source spec: AutoSpec Adaptive Agent Runtime (AAR), 2026-09-02

## What AAR is

AAR turns a work item into an execution policy:

```text
task -> classify -> retrieve minimum context -> choose topology -> choose model
     -> choose reasoning budget -> execute -> measure -> evaluate -> learn
```

The division of responsibility is the architecture, and it is load-bearing:

| Layer | Owns |
| --- | --- |
| AutoSpec / AAR | classification, topology, model requirements, reasoning budgets, context construction, durable task memory, guards, stop conditions, escalation, telemetry, outcome scoring |
| Pi (or another harness) | model interaction, tools, file reads and edits, commands, sessions and forks, event reporting |
| InferWeave | model and node discovery, context-capacity-aware routing, session affinity, prefix caching, overload and fair-share, placement |
| Inference engine | quantization, GPU/CPU placement, KV implementation, sampling, decoding |

Nothing in `autospec_core::aar` is Pi- or Qwen-specific. Adapters live at the
edges (`aar::pi`, `aar::inferweave`), and the decision engine never names a
physical node or a vendor API.

## Module map

Every module is pure: it returns plans, verdicts and records, and the caller
performs the I/O. That is what makes a routing decision testable without a
model, a node, or a worktree.

| Module | Spec section | Responsibility |
| --- | --- | --- |
| `aar::classify` | 3 | deterministic-first rubric producing class, complexity, risk, capabilities, confidence and evidence |
| `aar::profile` | 4 | versioned model capability profiles; benchmark scores adjusted (never replaced) by production outcomes |
| `aar::reasoning` | 5 | abstract budgets, evidence-gated selection, versioned sampling profiles |
| `aar::context` | 6, 11 | retrieval ladder and budget; cache-friendly prompt assembly and stable prefix hashing |
| `aar::memory` | 7 | `.autospec/` durable worktree task memory |
| `aar::topology` | 8 | role selection, structured handoffs, separation-of-duty enforcement |
| `aar::pi` | 9 | harness session spec, working rules, argv, event parsing and folding |
| `aar::guards` | 10 | edit guards, thrashing detection, stop conditions |
| `aar::inferweave` | 12 | capability requests, seat model, node scoring |
| `aar::escalation` | 13 | fallback chain with quota checks and separation re-verification |
| `aar::telemetry` | 14 | versioned execution records with three-way token accounting and redaction |
| `aar::outcome` | 15 | outcome scoring, cheapest-adequate-profile recommendation, hard policy override |
| `aar::policy` | 18, 19 | `decide()`, the policy-versioned decision record, and the explanation |

## Decisions worth knowing about

**Classification is deterministic-first.** The rubric runs with no LLM call and
reports a confidence; only work below the threshold needs a tie-breaker, and the
repository convention is that the tie-breaker runs at the cheap tier. This
matches `scripts/classify-model-fit.sh` and keeps the common case free.

**Hard requirements are filters, not scores.** A model that cannot serve the
required class, hold the projected context, or see an image when the task needs
vision is rejected outright rather than out-scored. The same rule governs node
routing: per spec section 12, a faster node without enough free context loses to
a slower eligible one, so free context is checked before any speed term.

**More reasoning is not assumed to be better.** A larger budget is kept only
when the measured success rate at that budget beats the smaller one by more than
`MATERIAL_SUCCESS_DELTA` on at least `MIN_SAMPLES_FOR_BUDGET_COMPARISON` samples
at both. Absent evidence, the cheaper budget wins.

**Production outcomes adjust benchmarks by shrinkage.** A profile's blended
score moves toward the observed success rate in proportion to how much evidence
exists (`OBSERVATION_SHRINKAGE_K`). Five failures move a score; they do not
erase a benchmark. Profile recommendation uses a Wilson lower bound for the same
reason: three-for-three must not outrank ninety-two-for-a-hundred.

**Separation of duties is about the instance, not the family.** An implementer
and a reviewer on the same profile key, or in the same session, is not an
independent review. Every fallback re-runs the check
(`preserves_separation_after_fallback`), because a quota failure is the usual way
separation quietly erodes.

**A session is a seat, not a slot.** Its demand is current context plus
projected growth plus KV, so a node that could hold four idle sessions may not
hold two growing ones.

**Memory is bounded.** Durable files that grow without limit recreate the
problem they exist to solve, so `WorktreeMemory::over_budget` reports the files
that need summarizing.

## CLI surface

```text
autospec aar classify --title <text> [--body-file <path>] [--path <p>]... [--json]
autospec aar plan     --title <text> [...] [--policy-version <v>] [--json]
autospec aar explain  --title <text> [...]
autospec aar memory init [--worktree <dir>] [--json]
autospec aar rules
```

`plan --json` emits the auditable decision record: policy version, registry
version, classification and its evidence, roles, candidate and rejected models,
reasoning budget and reasons, retrieval ladder, guard limits and escalation
chain. That record is the API half of spec section 17; the live dashboard is
the half that is not built yet.

## Acceptance criteria status

| # | Criterion | Status |
| --- | --- | --- |
| 1 | classify a work item and produce an execution policy | done (`aar::policy::decide`) |
| 2 | Pi launches using that policy | partial — the session spec, rules and argv are built and tested; no process is spawned |
| 3 | reasoning budgets and model profiles are configurable | done (`PolicyConfig`, `ReasoningLimits`, `ModelProfileRegistry`) |
| 4 | worktree-local durable task memory exists | done (`aar::memory`, `autospec aar memory init`) |
| 5 | context is targeted rather than full-history | done (`ContextPolicy`, default `include_full_history: false`) |
| 6 | edit and stop guards are enforceable | done (`aar::guards`) |
| 7 | agent roles can run in isolated contexts | done as policy (`AgentTopology::isolated_contexts`, structured handoffs); the executor that runs them is not built |
| 8 | separation of duties survives fallback | done (`aar::topology`, `aar::escalation`) |
| 9 | InferWeave receives model/context/cache/session requirements | done as a contract and reference scorer; the InferWeave-side scheduler is a separate repository |
| 10 | execution telemetry is persisted | schema, validation and JSONL rendering done; the writer is not wired into an executor |
| 11 | benchmarks inform profile recommendations | the ingestion path (`record_outcome`, `ProfileStats`, `recommend`) is done; the benchmark harness that feeds it is not |
| 12 | dashboard/API explains selected profile and state | API half done (`explain()`, `plan --json`); the GUI is not built |
| 13 | decisions are policy-versioned and auditable | done (`DecisionRecord`) |

## Not built here

Deliberately out of scope for this change, and each large enough to deserve its
own one:

- **Live execution.** Nothing spawns a Pi process, folds its real event stream,
  or writes telemetry to `.autospec/telemetry/executions.jsonl`. The adapter
  boundary is built and tested against synthetic event streams.
- **InferWeave server side.** `aar::inferweave` is the request contract plus a
  reference scorer that pins the specification's routing rules. Discovery,
  admission control and real placement live in `berlinguyinca/autospec-inferweave`.
- **Dashboard and GitHub project board** (spec section 17), beyond the JSON
  decision record.
- **Benchmark harness** (spec section 16). AAR consumes measurements; it does
  not yet produce them, and every score in `ModelProfileRegistry::starter()` is
  an unmeasured placeholder.
- **Adaptive optimization** (spec section 20, phase 6). V1 is deterministic
  rules over measured statistics, as the specification prescribes; contextual
  bandits are explicitly later work.

## Validation

`scripts/validate-aar.sh` guards the structural invariants a compiler cannot:
that every module and test suite exists, that the documented defaults are the
values in the code, and that the load-bearing rules (verbatim working rules,
context-as-a-filter, separation re-checked on fallback, token accounting that
adds up) are enforced in code rather than only in prose. It then runs the AAR
test suites single-threaded.
