# Autospec Tooling Optimization — Deterministic Templates for Token Reduction

**Status:** Draft design (2026-05-22)
**Scope:** Closes tracker #421. Converts LLM-driven steps to deterministic shell+templates where possible. Target: 30-60% token reduction across decomposer + reviewer + report paths.

## 1. Goal & non-goals

### Goal
Replace LLM-driven steps with deterministic template + shell tooling for the high-frequency paths in the autospec pipeline. LLM calls stay only where genuine creative judgment is required (Goal sentence, Implementation outline specifics). Everything else — issue body scaffolding, model-fit classification, PR report composition, implementer-prompt assembly, reviewer prompt assembly — becomes template-driven with deterministic field substitution.

### Non-goals
- Removing LLM from the pipeline entirely
- Reworking the spec → issue → PR flow
- Replacing per-skill SKILL.md content with generated docs

## 2. Architecture

5 new deterministic tools at `$AUTOSPEC_SCRIPTS_DIR`. Each fills a specific high-frequency-LLM-call point with a template + structured input contract.

| Tool | Replaces | LLM saving |
|---|---|---|
| `gen-issue-skeleton.sh` | Phase 3 decomposer's per-issue body generation | ~70% of decomposer tokens |
| `classify-model-fit.sh` | Phase 3.5 classifier's per-issue ctx/reasoning rubric | ~80% of classifier tokens |
| `gen-pr-report.sh` | Phase 4 monitor's PR comment composition | 100% (currently inline LLM in monitor prompt) |
| `gen-implementer-prompt.sh` | Phase 4 monitor's implementer-dispatch prompt assembly | Combined with bundle-static-context for ~90% cache + template |
| `gen-reviewer-prompt.sh` | Fused guardian+LGTM reviewer prompt assembly | Same caching + template |

LLM calls preserved for:
- Goal sentence + Implementation outline (creative judgment per issue)
- Decomposer ambiguous-classification escalation
- Per-section module summary in reverse-engineer
- AI-as-reviewer doc grading

## 3. Component 1 — `gen-issue-skeleton.sh`

**Input contract** (stdin or `--input <file>`):
```yaml
issue_id: phase4-impl
spec_path: docs/specs/2026-05-22-autospec-test-design.md
spec_url: https://github.com/.../docs/specs/2026-05-22-autospec-test-design.md
goal_sentence: "Implement Phase 4 reverse-engineer pipeline producing tree-sitter-based module specs."
files_to_read:
  - { path: docs/specs/2026-05-22-autospec-test-design.md, anchor: "§4b" }
  - { path: skills/autospec-shared/scripts/tree-sitter-walk/walker.mjs, reason: "WalkOutput shape" }
implementation_scope:
  - "scripts/reverse-engineer.sh"
  - "scripts/reverse-engineer/inventory.mjs"
out_of_scope:
  - "Cross-skill dispatch"
implementation_outline_lines: [...]  # 5-30 lines
tests_required: [...]
acceptance_criteria: [...]
verification:
  primary_smoke: "bats tests/reverse-engineer.bats"
  operator_full:
    - "bash scripts/reverse-engineer.sh --repo-root ."
branch_name: feat/phase4-reverse-engineer
dependencies: [328]  # issue numbers
```

**Output:** the complete 11-section markdown body, validated by `lint-issue.sh` before emission. Falls back to LLM only for the `goal_sentence` field if the operator didn't provide one (and even then, the prompt is much smaller — just "write one sentence about X").

## 4. Component 2 — `classify-model-fit.sh`

Deterministic rubric:

```
ctx_tier =
  if (files_to_read.count <= 3 AND avg_anchor_size_kb < 1):
    "32k"
  elif (files_to_read.count <= 7 AND total_anchor_size_kb < 5):
    "64k"
  elif (files_to_read.count >= 8 OR cross_skill OR total_anchor_size_kb >= 5):
    "120k"
  else:
    "64k"  # default

reasoning =
  if (verbs in {copy, rename, transcribe, list, format}):
    "shallow"
  elif (verbs in {mirror, adapt, integrate, wire, follow, extend}):
    "medium"
  elif (verbs in {design, reconcile, resolve, redesign, decide}):
    "deep"
  else:
    "medium"  # default

LLM_ESCALATION_THRESHOLD = 0.3  # confidence below this triggers LLM tie-breaker
confidence = compute_confidence(verbs_matched_count, file_count_explicit, anchor_size_explicit)
if confidence < LLM_ESCALATION_THRESHOLD:
  llm_classify(issue_body) -> { ctx, reasoning, rationale }
else:
  deterministic_classify -> { ctx, reasoning, rationale }
```

Outputs `## Model fit` block in standard format. Logs to telemetry the deterministic-vs-LLM ratio.

## 5. Component 3 — `gen-pr-report.sh`

100% template-driven. Inputs:
- Gate JSON (Stage 1 + Stage 2 + Stage 2.5 results)
- Drift JSON (drift findings)
- Loop iterations log (heartbeat history)

Output: markdown PR comment matching the marker convention (`<!-- autospec-test-report-marker -->`), composed from a fixed Handlebars-like template with no LLM call.

Template format (excerpt):
```
<!-- autospec-test-report-marker -->
## autospec-test — {{status_emoji}} {{status_text}}

**Mode:** {{mode}}
**Coding time used:** {{coding_time_used}} / {{coding_time_budget}}
**Iterations:** {{iter_count}} / {{max_iters}}

### Why {{outcome_short}}
{{#each metric in metrics}}
{{#unless metric.passed}}- {{metric.label}}: {{metric.failure_reason}}{{/unless}}
{{/each}}
```

## 6. Component 4 — `gen-implementer-prompt.sh`

Wraps `bundle-static-context.sh` + dynamic suffix from `gen-issue-skeleton.sh`-style structured input. Eliminates the orchestrator-side LLM call that currently composes implementer prompts inline.

Composes:
1. CACHE PREFIX (from bundle-static-context.sh): SKILL.md + AGENTS.md + RULE_IDs + tagged saved memory
2. DYNAMIC SUFFIX: parsed issue body + branch name + worktree commands + "begin coding now"

100% template-driven, zero LLM call to build the prompt itself.

## 7. Component 5 — `gen-reviewer-prompt.sh`

Same pattern as #4 but for the fused guardian+LGTM reviewer. Wraps `bundle-static-context.sh --role reviewer` + dynamic PR-diff + previous-iteration findings.

## 8. Decomposition (5 phases)

| # | Phase | Size | Deps |
|---|---|---|---|
| T1 | `gen-issue-skeleton.sh` + structured input contract + bats fixtures | 1 PR | none |
| T2 | `classify-model-fit.sh` + LLM-escalation threshold + telemetry | 1 PR | none |
| T3 | `gen-pr-report.sh` + template format + bats goldens | 1 PR | none |
| T4 | `gen-implementer-prompt.sh` + orchestrator wire-up | 1 PR | depends T1 (uses same input structure) |
| T5 | `gen-reviewer-prompt.sh` + orchestrator wire-up | 1 PR | depends T4 (pattern shared) |

All carry `priority:high`. T1-T3 parallel-shippable. T4 after T1. T5 after T4.

## 9. Testing

- Per tool: bats fixtures (structured input → expected output) + golden diffs
- Integration: end-to-end decompose-classify-report-implement chain against a synthetic issue; assert token cost drops ≥40% vs LLM-only baseline (measured via #422 telemetry dashboard)
- Backward compat: legacy LLM-driven paths remain available as fallback when env `AUTOSPEC_FORCE_LLM=1` set

## 10. Dependencies & scope boundaries

| Dependency | Status | Failure mode |
|---|---|---|
| `bundle-static-context.sh` (#402) | live | required for T4/T5 |
| `bundle-and-dispatch.sh` (#418) | live | required for T4/T5 |
| Telemetry capture (#403) | live | needed to measure savings |

### Out of scope
- Replacing LLM for genuine creative writes (Goal sentence, Implementation outline specifics)
- Replacing the AI-reviewer for doc grading
- Cross-language template generation (handlebars-style is one fixed format)

## 11. Decision log

| Q | Decision | Rationale |
|---|---|---|
| Template engine — handlebars / mustache / sed? | Plain bash + envsubst-style substitution | No new dependency; deterministic; portable |
| Goal-sentence still LLM? | Yes — but with a tiny prompt | Creative judgment per issue; saves <100 tokens per call vs current ~3k |
| Classifier escalation threshold? | 0.3 confidence | Empirical; can tune via telemetry feedback loop |
| Backward compat? | `AUTOSPEC_FORCE_LLM=1` env flag | Rollback path if template misses an edge case |

## 12. Open follow-ups

- Telemetry-driven auto-tuning of classification confidence threshold (future)
- Cross-skill template inheritance (future)
- LLM-driven mutant generation for templates (research)
