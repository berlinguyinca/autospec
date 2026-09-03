# AutoSpec Agentic RAG Subsystem — design and implementation record

**Status:** Phase 1–3 core implemented in `crates/autospec-core/src/rag/`; phases 4–8 not started
**Target:** AutoSpec (core + CLI), with declared integration points for Pi and InferWeave
**Implementation Language:** Rust
**Spec Type:** Architecture + Implementation
**Date:** 2026-09-03
**Source specification:** "AutoSpec Agentic RAG Subsystem — Implementation Specification" v1.0, 2026-09-02

## 1. What landed

The specification's sections 6–40 describe one coherent machine: an iterative
retrieval loop over pluggable knowledge sources, producing evidence with
provenance, evaluated for sufficiency, budgeted, traced, and packaged for one
agent role. That machine is implemented as a pure Rust module tree under
`crates/autospec-core/src/rag/`, with an `autospec rag` command for inspecting
its effective policy.

| Module | Specification sections | What it does |
|---|---|---|
| `score` | 10 | Integer `0.000`–`1.000` scores |
| `authority` | 9 | Authority classes and the precedence ladder |
| `scope` | 15, 46, 47 | Revision and worktree resolution, sharing rules |
| `evidence` | 10, 11, 53 | The Evidence object, provenance, privacy |
| `source` | 8, 50 | `KnowledgeSource` adapter trait and registry |
| `budget` | 39, 40, 41.1 | Budgets, the ledger, stop reasons |
| `policy` | 7, 20, 22 | Per-role source ordering and token ceilings |
| `query` | 13, 41.2 | Query planning, reformulation, repeat suppression |
| `graph` | 14, 15 | Revision-aware knowledge graph and traversal |
| `contradiction` | 9, 30 | Contradiction records and severity |
| `freshness` | 31 | Per-source staleness rules |
| `evaluator` | 12 | Deduplication, coverage, sufficiency verdicts |
| `injection` | 29 | Trust bands, injection scanning, quarantine |
| `compression` | 19 | Six compression levels, token estimation |
| `context_package` | 18, 19, 20 | The structured package and its budget |
| `cache` | 25, 41.4 | Revision-aware cache and invalidation |
| `trace` | 35, 54 | Per-iteration retrieval traces |
| `metrics` | 37, 38 | Counters derived from traces |
| `routing` | 23, 24 | Capability declaration, context-aware node selection |
| `memory` | 16, 17 | Memory tiers and the seven-check write gate |
| `config` | 51 | The `agentic_rag:` block |
| `coordinator` | 6, 32, 33, 40 | The agentic loop itself |
| `baseline` | 56, 57.15 | Fixed top-K, for measured comparison |

## 2. Decisions that depart from the specification text

Three places where the specification's literal text and this codebase's
constraints disagreed, and how each was resolved.

### 2.1 Scores are integers, not decimals

Section 10's schema writes `confidence: 0.98`. This workspace's
`financial_no_f64` architecture fitness function forbids `f64` in `crates/**`,
and there is a second reason to avoid floating point here that applies even
without the rule: the loop branches on threshold comparisons, so two hosts that
round differently would make different stopping decisions on identical evidence.

`score::Score` holds permille (`0..=1000`) and renders `0.980` at the
serialization boundary. `Score::parse` accepts the specification's decimal form
and rejects more precision than permille can hold, rather than rounding — two
inputs that a threshold would treat differently must not silently become equal.

### 2.2 The trait is synchronous

Section 50 sketches `async fn search(...)`. The AutoSpec Rust core carries no
async runtime and no HTTP client, and section 50 says its Rust is illustrative
and the implementation follows the existing stack. `KnowledgeSource::search` is
therefore synchronous; adapters that need I/O perform it on the caller's thread.
Nothing in the loop's logic depends on the choice, so an async variant can be
introduced later behind the same trait shape.

### 2.3 The gate on policy-controlled sources is the task, not the role

Section 8.9 makes web retrieval "optional and policy-controlled", used when "the
task explicitly requires external research". Section 7 separately gives each role
a source ordering that lists the web as low priority. Reading the role's ordering
as the policy gate would make every role permanently web-enabled, which is the
opposite of what section 8.9 asks for.

`RagConfig::source_allowed(kind, role, task_permits_gated)` therefore takes three
inputs: the administrator's availability setting is the ceiling, the role's
policy must list the source at all, and a `policy`-gated source additionally
needs the task to have asked for it. `autospec rag sources --role spec` and
`--role spec --external` show the two answers side by side, because "why did it
not search the web" has two possible causes and an operator needs to see which.

## 3. Design choices worth recording

### 3.1 Sufficiency is a conjunction, never a similarity score

Section 12 ends by forbidding acceptance on similarity alone. The evaluator
returns `Sufficient` only when every declared aspect is covered, enough distinct
items survived deduplication, and mean relevance clears the role's threshold.
`EvidenceEvaluator::narrow_with` folds a model evaluator's opinion in, and it is
deliberately one-way: a model may downgrade `Sufficient` to `Insufficient` and
add follow-up queries, never the reverse.

### 3.2 Required evidence is never dropped for budget

`ContextPackageBuilder` places required evidence first and returns an error if it
alone exceeds the role ceiling, rather than truncating. Supporting evidence is
dropped from the end and every dropped id is reported in `omitted_evidence`, so
an agent can tell "retrieval found nothing more" from "retrieval found more than
it could carry" and ask for the rest.

### 3.3 Worktree evidence is never cached

Section 47 forbids uncommitted source from one worktree reaching another, and a
shared cache is the obvious leak path. `RetrievalCache::store` refuses
worktree-scoped entries outright and returns `false`. Committed evidence is still
shared freely across worktrees at the same revision, which is the reuse section
47 wants.

### 3.4 A likely injection is quarantined, not fenced

Section 29 requires retrieved text to be treated as data. Fencing handles content
that merely resembles an instruction. Content that unambiguously tries to seize
the agent — "ignore previous instructions", "exfiltrate", "send the contents
to" — is withheld from the package entirely: its informational value does not
justify the residual risk of a model reading inside the fence. Suspicious-tier
content is fenced with a visible warning and still shown. Evidence classed as an
explicit user requirement is exempt from scanning, because the user really can
say "ignore the previous plan".

### 3.5 The core reads no clock and no repository

`RetrievalCoordinator::new` takes `now` and `current_revision` as parameters.
Freshness and cache validity are facts about the caller's world, and a
coordinator that read them itself could not be tested against a fixed
expectation. Every budget, stopping-rule, staleness and worktree behavior in this
subsystem is consequently a pure function of its inputs.

## 4. Evidence that it works

Ten integration suites, the CLI surface, and the module unit tests — 160 tests,
all deterministic and side-effect-free. They hold no files, no environment and no
shared state, so they neither require nor benefit from serialized execution:

| Suite | Section | Tests |
|---|---|---|
| `rag_evidence` | 10, 11, 53 | 11 |
| `rag_retrieval_loop` | 6, 12, 39, 40, 55.1, 55.4 | 12 |
| `rag_graph` | 14, 15, 55.2 | 12 |
| `rag_contradiction` | 9, 30, 55.3 | 8 |
| `rag_cache_worktree` | 25, 31, 46, 47, 55.5, 55.6 | 11 |
| `rag_injection` | 29, 55.7 | 9 |
| `rag_policy_context` | 7, 18, 19, 20, 22 | 16 |
| `rag_routing_config` | 16, 17, 23, 24, 31, 51 | 23 |
| `rag_trace_query` | 13, 35, 37, 41 | 14 |
| `rag_benchmark` | 56, 57.15 | 6 |
| `rag_commands` (CLI) | — | 16 |
| module unit tests | 9, 10, 15 | 22 |

`scripts/validate-agentic-rag.sh` gates the module inventory, the specification
cross-references, the no-`f64` rule, the file-size ratchet, and the suites above.

### 4.1 The benchmark comparison

`rag_benchmark` runs `rag::baseline::retrieve_top_k` and the agentic coordinator
against the same corpora, which is what acceptance criterion 15 asks for. On a
corpus where the answer needs two facts and the higher-similarity chunks all
concern the first, fixed top-3 drops the second fact and says nothing about the
omission; the loop reformulates and covers both. On a corpus of near-duplicate
filler with one authoritative answer, the agentic package is smaller than the
baseline's K chunks, because it stops when the question is answered rather than
when K is reached. And where a poisoned chunk ranks highest, top-K puts it at the
head of the prompt while the loop quarantines it.

## 5. Not implemented

Named so the gap is not mistaken for a gap in the specification.

- **Real source adapters.** The `KnowledgeSource` trait and registry are
  complete; no adapter for git, GitHub, the specification tree, or memory ships
  in this change. Phase 1 in section 48 is otherwise done.
- **Symbol indexing (phase 2).** `KnowledgeGraph` is complete as a data
  structure with revision-aware traversal, but nothing populates it from an AST.
- **Model-backed evaluation and rewriting.** The deterministic evaluator and
  planner run standalone. `narrow_with` is the seam a model evaluator plugs into;
  `RagModelTask::capabilities` is the seam InferWeave routing plugs into. Neither
  is wired to an inference call.
- **Persistence (sections 34, 52).** Traces, evidence and packages are in-memory
  values. The `/v1/rag/*` endpoints of sections 32–34 have their request and
  response types (`RetrievalRequest`, `RetrievalOutcome`) but no HTTP surface.
- **Dashboard (section 36) and distributed retrieval (sections 26–27).** The
  metrics and trace data the dashboard needs is produced; nothing renders it.

## 6. Acceptance criteria status

Against section 57:

| # | Criterion | Status |
|---|---|---|
| 1 | Iterative retrieval requests | Done — `RetrievalCoordinator::retrieve` |
| 2 | Repository, spec, GitHub, memory sources | Partial — trait and registry done, adapters not written |
| 3 | Complete provenance | Done — `Evidence`, enforced at build time |
| 4 | Reformulation after insufficiency | Done — `QueryPlanner::plan_followup` |
| 5 | Configurable budgets and stopping rules | Done — `RetrievalBudget`, `StopReason` |
| 6 | Role-specific policies | Done — `RetrievalPolicy` for all eight roles |
| 7 | Pi agents can request context | Partial — `RetrievalRequest` exists, no Pi tool binding |
| 8 | Worktree-aware retrieval | Done — `RetrievalScope`, tested |
| 9 | Packages obey token budgets | Done — `ContextPackageBuilder` |
| 10 | Contradictions surfaced | Done — `ContradictionSet`, never auto-resolved |
| 11 | Revision-aware cache | Done — `RetrievalCache`, tested for invalidation |
| 12 | Traces persisted | Partial — traces produced, no durable store |
| 13 | Dashboard | Not started |
| 14 | InferWeave routes RAG subtasks | Partial — `select_node` decides, no dispatch |
| 15 | Beats fixed top-K | Done — `rag_benchmark` |
