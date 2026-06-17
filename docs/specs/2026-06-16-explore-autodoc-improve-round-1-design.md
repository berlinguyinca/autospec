# Design spec — make `/autospec-doc` actually teach an LLM the project

- **Sandbox branch:** `autospec/explore/2026-06-16-autodoc-improve`
- **Round:** 1
- **Driver:** `/autospec-explore` (focused, single-skill investigation)
- **Operator complaint:** the auto-documentation skill "seems a bit lacking and
  was missing critical information for the LLMs to learn."

## Result first

The complaint is correct, and it has **two independent root causes** — both
must be addressed or the docs stay thin:

1. **Content-model poverty (design gap).** Even when the generators run, a
   feature page is just `title + summary + spec_sections (prose) + an AI
   confidence marker`. The categories an LLM needs to *learn* a system — data
   models/schemas, invariants/constraints, error semantics, config/env/CLI
   reference, the "why"/rationale, and cross-feature dependencies — are **never
   captured**. `gen-audience-docs.mjs:158-244` emits only the summary and the
   provided prose sections.

2. **Pipeline incompleteness (implementation gap).** The high-signal artifacts
   that exist on paper are not wired:
   - `.llm-manifest.json` is a **legacy stub**: `modules`, `cli_entry_points`,
     `http_endpoints`, `faq` are all `[]` and `concepts` holds a literal
     `"<name>"` placeholder (dated 2026-05-22). The symbol map an LLM/RAG would
     navigate by is empty.
   - `doc-style.mjs` exports `generateExplainerDiagram()` and
     `isLogicFlowSection()` but **no generator imports it** — only the test does.
     Architecture/flow diagrams are dead code.
   - `verify-examples.mjs` is fully built, but `gen-audience-docs.mjs` never
     emits `<!-- example -->` blocks for it to run, so no verified runnable
     examples (with captured `output`) ever reach the docs or `llms-full.txt`.
   - `llms.txt` does not follow the llmstxt.org shape (duplicate `# autospec`
     H1, no per-link descriptions, no routing), and `llms-full.txt` has no
     navigable index, no per-section summaries, no token-budget annotations, and
     no source-file→doc reverse routing.

Net: an LLM reading today's output gets *what* a feature is named and a one-line
summary, but not *how it works, how it fails, how to configure it, or why it
exists* — exactly the "critical information … to learn" the operator flagged.

## Evidence

| Claim | Evidence |
|---|---|
| Page content is summary + prose only | `gen-audience-docs.mjs:158-244` (renderIndex/getting-started/tutorial/feature) |
| Diagram generator orphaned | `gen-audience-docs.mjs` imports only `gen-docs-from-spec`, `ai-review-doc`, `scan-doc-scope`; `doc-style.mjs` imported only by `doc-style.test.mjs` |
| Manifest is an empty stub | `docs/.llm-manifest.json`: empty `modules/cli_entry_points/http_endpoints/faq`, `concepts[0].name == "<name>"` |
| Examples never emitted into pages | `verify-examples.mjs` parses/executes `<!-- example -->` blocks; generators never produce them |
| `llms.txt` off-standard | duplicate H1 at lines 1 & 5; no per-link descriptions/routing |
| Much of the skill is still stubbed | `SKILL.md:20-24` "scaffold contract"; generators "filled in by #917-#924" |

## Improvement tracks (severity-first)

### Track A — Enrich the content model *(severity: correctness)* — the direct fix
Extend the feature/config schema (`doc-config.mjs`) and the audience renderers
(`gen-audience-docs.mjs`) to emit structured, LLM-targeted sections per feature,
each audience-scoped:
- `## Data model` — schemas/types for the feature's inputs/outputs.
- `## Invariants & constraints` — what must hold; pairs with the project's
  existing invariant/guard discipline.
- `## Errors & failure modes` — error taxonomy: cause → recovery, what fails
  silently vs. loudly.
- `## Configuration` — CLI flags, env vars, config keys (admin + developer).
- `## Why` — rationale / design decision / alternatives rejected (developer).
- `## Related features` — cross-links from a `depends_on` edge (dependency graph).

Acceptance criteria:
- [ ] `doc-config.mjs` accepts and validates the new per-feature fields
  (`data_model`, `invariants`, `errors`, `config_reference`, `rationale`,
  `depends_on`) with safe defaults when absent (backward compatible).
- [ ] `gen-audience-docs.mjs` renders each present field as its own H2 section,
  audience-gated per the table above, and omits empty sections.
- [ ] A fixture feature carrying all new fields round-trips into the four
  audience pages with the expected headings (unit test).
- [ ] No regression: a feature with only `summary`/`spec_sections` produces
  byte-identical output to today.

### Track B — Populate `.llm-manifest.json` with real symbol data *(severity: correctness)*
Make the manifest the navigable symbol map it is specified to be.

Acceptance criteria:
- [ ] `gen-llms-full.mjs` populates `modules[]` (name + summary + public_api +
  doc path), `cli_entry_points[]`, and `concepts[]`/glossary from the generated
  corpus and `<!-- autospec-concept: -->` markers — no literal `<name>` stub.
- [ ] Each manifest entry carries `source_anchor` provenance and an
  `approx_tokens` size hint.
- [ ] A fixture corpus produces a manifest with non-empty `modules` and
  `concepts` and zero placeholder strings (unit test).

### Track C — Wire verified runnable examples end-to-end *(severity: stability)*
Close the loop between example authoring, execution, and LLM-facing output.

Acceptance criteria:
- [ ] `gen-audience-docs.mjs` emits `<!-- example -->` fenced blocks from a
  `feature.examples[]` field.
- [ ] `verify-examples.mjs` runs them and the captured ` ```output ` block plus
  `<!-- example-verified: <sha> <iso> -->` marker survive concatenation into
  `llms-full.txt` (not stripped).
- [ ] A fixture example executes, its output is embedded, and the verified
  marker appears in both the page and `llms-full.txt` (unit test).

### Track D — Activate architecture/flow diagrams (kill the dead code) *(severity: operability)*
Import and call the orphaned `doc-style.mjs` diagram functions.

Acceptance criteria:
- [ ] `gen-audience-docs.mjs` imports `doc-style.mjs` and calls
  `isLogicFlowSection()` / `generateExplainerDiagram()` for logic-flow sections,
  emitting palette-themed mermaid diagrams.
- [ ] A logic-flow fixture section yields a themed mermaid block; a non-flow
  section yields none (unit test).
- [ ] No second palette source is introduced (existing single-source guard
  still passes).

### Track E — Make `llms.txt` a real index + `llms-full.txt` navigable *(severity: operability)*
Conform to the llmstxt.org convention and add LLM-navigation affordances.

Acceptance criteria:
- [ ] `llms.txt`: single H1, blockquote summary, curated link sections each with
  a one-line description; no duplicate heading.
- [ ] `llms-full.txt`: a top table-of-contents with anchors, a 1–2 line summary
  before each section, per-section `approx_tokens` annotation, a
  source-file→doc reverse-routing block, and a `generated_at`+`commit` freshness
  stamp.
- [ ] Generation is deterministic and idempotent (re-run on unchanged corpus is
  a no-op diff).

## Out of scope (this round)
- Completing the still-stubbed Phase 4 self-heal / Phase 5.5 completeness / sweep
  + define redirects (#922-#924). Tracks A–E raise content quality on the
  generation path that already runs; the wiring is a separate program.
- Per-audience `llms-full-<audience>.txt` splits (future, once A–E land).

## Verification
- `bash scripts/validate.sh` stays green (lockstep trio + goldens regenerated for
  any `SKILL.md` prose touched).
- New unit tests under `skills/autospec-doc/tests/` for each track.
- A smoke generation against a fixture repo shows all six new section types,
  a populated manifest, an executed example, a themed diagram, and a navigable
  `llms-full.txt`.
