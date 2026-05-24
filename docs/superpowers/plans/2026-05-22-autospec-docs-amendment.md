# Autospec Family Docs Amendment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Decomposition:** 10 sequential phases. Each phase = one GitHub issue for `/autospec-split`. All phases `Depends on #351` (v2 Phase 10 Stage 2.5 orchestrator) as the root prerequisite, plus linear deps within this plan.

**Goal:** Ship the docs amendment across the existing autospec skills — drift gate in Phase 4, `--init` reverse-engineer mode, deterministic doc generation (USER_MANUAL/API_REFERENCE/ARCHITECTURE/llms.txt/.llm-manifest.json), auto-generated screenshots and mermaid architecture diagrams, AI-as-reviewer with confidence grading.

**Architecture:** No new skill. All amendments to existing skills (`autospec-define`, `autospec-run`, `autospec-test`). New shared tooling lives at `$AUTOSPEC_SCRIPTS_DIR` (NOT vendored). Tree-sitter is the foundation primitive — lands in Phase 1, gets reused by every downstream phase + by the queued tooling-optimization work.

**Tech Stack:** Bash 4+, Node 20+, tree-sitter CLI + per-language grammars, `yq`, `ajv`, Playwright ≥1.40 (reused from autospec-test v1), `mermaid` (output is plain text, no runtime needed; renders in GitHub), `asciinema` (optional, falls back to `script -c`).

**Spec reference:** `docs/specs/2026-05-22-autospec-docs-amendment-design.md` (merged via PR #358, commit `a17a907`).

**v2 prerequisite:** v2 #351 must merge before Phase 1 of this plan starts — drift gate hooks the same orchestration point as Stage 2.5. All issues in this plan `Depends on #351`.

---

## File Structure (locked across phases)

```
skills/autospec-define/SKILL.md                       # Phase 9 (modify: --init mode + auto-docs)
skills/autospec-run/SKILL.md                          # Phase 9 (modify: Phase 4 drift gate hook)
skills/autospec-test/SKILL.md                         # Phase 10 (modify: drift gate composes with Stage 2.5)

skills/autospec-shared/scripts/
  tree-sitter-walk/                                   # Phase 1
    walker.mjs                                        # Phase 1
    queries/
      typescript.scm
      javascript.scm
      python.scm
      go.scm
      rust.scm
      java.scm
  scan-doc-scope.mjs                                  # Phase 2
  check-doc-drift.sh                                  # Phase 2
  loop-classifier-docs-extension.mjs                  # Phase 3 (patches v1 loop-classifier.mjs)
  reverse-engineer.sh                                 # Phase 4
  reverse-engineer/
    inventory.mjs                                     # Phase 4
    cluster.mjs                                       # Phase 4
    emit-spec.mjs                                     # Phase 4
  gen-docs-from-spec.mjs                              # Phase 5
  gen-docs/
    user-manual.mjs                                   # Phase 5
    api-reference.mjs                                 # Phase 5
    architecture.mjs                                  # Phase 5
  gen-llms-txt.sh                                     # Phase 6
  gen-llm-manifest.mjs                                # Phase 6
  gen-assistant-prompt.mjs                            # Phase 6
  gen-screenshots.mjs                                 # Phase 7
  gen-arch-diagram.mjs                                # Phase 7
  ai-review-doc.mjs                                   # Phase 8
  install-doc-drift-hook.sh                           # Phase 9 (pre-commit installer)
  install-doc-drift-workflow.sh                       # Phase 9 (GH Actions workflow installer)

.github/workflows/autospec-doc-drift.yml              # Phase 9 (the workflow shipped by installer)

skills/autospec-shared/test-targets/                  # Phase 10
  target-doc-drift-bait/
  target-reverse-engineer-bait/
  target-manifest-stale-bait/
  target-visual-stale-bait/
  target-ai-low-confidence-bait/
  lang-matrix/{node,python,go,rust,jvm}/

skills/autospec-shared/tests/
  unit/                                               # Per-phase tests added in phases 1..8
  integration/                                        # Phase 10
```

Note: `skills/autospec-shared/` is a new directory housing cross-skill tooling. Phase 1 creates the directory structure. Scripts there are installed to `$AUTOSPEC_SCRIPTS_DIR` by the autospec installer (Phase 9 task wires this up).

---

## Phase 1 — Tree-sitter foundation + per-language queries

**GH issue title:** `feat(autospec-docs): tree-sitter foundation + per-language queries (phase 1)`
**Depends on:** #351

### Files
- Create: `skills/autospec-shared/scripts/tree-sitter-walk/walker.mjs`
- Create: `skills/autospec-shared/scripts/tree-sitter-walk/queries/{typescript,javascript,python,go,rust,java}.scm`
- Create: `skills/autospec-shared/tests/unit/tree-sitter-walk.test.mjs`
- Modify: `skills/autospec-shared/package.json` (add `web-tree-sitter` + grammar deps)

### Tasks

- [ ] **1.1** Initialize `skills/autospec-shared/` with package.json declaring deps: `web-tree-sitter`, `tree-sitter-typescript`, `tree-sitter-javascript`, `tree-sitter-python`, `tree-sitter-go`, `tree-sitter-rust`, `tree-sitter-java`.

- [ ] **1.2** Write `walker.mjs` exporting:
  ```ts
  walk(filePath: string): Promise<{
    language: 'typescript' | 'javascript' | 'python' | 'go' | 'rust' | 'java' | 'unknown',
    exports: Array<{ name: string, kind: 'function'|'class'|'type'|'const', signature: string, line: number }>,
    entry_points: Array<{ kind: 'cli_command'|'http_route', identifier: string, line: number }>,
    imports: Array<{ source: string, names: string[] }>,
    file_path: string
  }>
  ```
  Detect language by file extension. Load the matching grammar. Run the per-language `.scm` query. Walk the parse tree mapping captures to the output schema.

- [ ] **1.3** Write per-language `.scm` queries. Each captures:
  - Exported declarations (functions/classes/types/constants)
  - Imports
  - CLI entry points (heuristics per language: e.g., TypeScript = `commander.command(...)`, Python = `@click.command()` / `if __name__ == '__main__'`, Go = `cobra.Command`, Rust = `clap`, Java = `main` method)
  - HTTP routes (Express `app.{get,post,put,delete}`, FastAPI `@app.get`, Echo/Gin, Actix-web, Spring `@*Mapping`)

- [ ] **1.4** Build a CLI wrapper `bin/tree-sitter-walk` that takes a file path on stdin or `--root <dir>` and emits the JSON output.

- [ ] **1.5** Unit tests with one tiny fixture per language under `tests/fixtures/tree-sitter/<lang>/sample.<ext>`. Assert exports + entry_points + imports for each.

### Acceptance criteria
- 6 languages parse cleanly; tests pass
- Walker handles malformed files gracefully (returns `{ language: 'unknown', ... empty arrays }`)
- CLI wrapper emits well-formed JSON
- Commit: `feat(autospec-docs): tree-sitter foundation + per-language queries (phase 1)`

---

## Phase 2 — Doc-scope parser + drift checker

**GH issue title:** `feat(autospec-docs): scope parser + drift checker (phase 2)`
**Depends on:** #351 + Phase 1 issue

### Files
- Create: `skills/autospec-shared/scripts/scan-doc-scope.mjs`
- Create: `skills/autospec-shared/scripts/check-doc-drift.sh`
- Create: `skills/autospec-shared/tests/unit/scan-doc-scope.test.mjs`
- Create: `skills/autospec-shared/tests/unit/check-doc-drift.bats`

### Tasks

<!-- autospec-doc-scope:
  src: ["skills/autospec-shared/scripts/scan-doc-scope.mjs", "skills/autospec-shared/scripts/check-doc-drift.sh", "tests/unit/test_doc_drift_mismatch_action.bats", ".github/workflows/autospec-doc-drift.yml", "tests/classify-model-fit.bats", "tests/fixtures/classify-model-fit/small.md", "tests/fixtures/classify-model-fit/medium.md", "tests/fixtures/classify-model-fit/large.md", "tests/fixtures/classify-model-fit/deep-reasoning.md", "tests/fixtures/classify-model-fit/low-confidence.md"]
  reason: "Phase 2 tasks cover scan-doc-scope.mjs and check-doc-drift.sh implementation"
  mismatch_action: warn
  generated: false
-->

_Note (2026-05-22): `classify-model-fit.sh` (tooling-opt T2, #460) ships bats coverage under `tests/classify-model-fit.bats` and fixtures under `tests/fixtures/classify-model-fit/`. These are in scope here because the Phase 2 drift checker validates all test coverage patterns. PR #478._

- [ ] **2.1** Write `scan-doc-scope.mjs` exporting `parse(markdownPath): Array<{ heading_path: string, src_globs: string[], visual_glob?: string, generated?: boolean, reason?: string, mismatch_action: 'hard_fail'|'warn', byte_range: [start, end] }>`. Parses `<!-- autospec-doc-scope: ... -->` blocks. Uses YAML inside the comment for structured fields. The `byte_range` covers the section body (from after the comment to the next same-or-higher heading). The `mismatch_action` field defaults to `hard_fail` when absent.

- [ ] **2.2** Fixture markdown files covering: valid scope, multiple sections, malformed YAML inside scope (must error cleanly), section with `generated: true`, section with `visual:` glob, file with no scope at all (returns empty array).

- [ ] **2.3** Write `check-doc-drift.sh`:
  ```
  Inputs: --diff <git_diff_output> | --pr <PR_number> | --working-tree
  Outputs: exit 0|1|2, stdout JSON per spec §3b

  1. Use `git diff --name-only` and `git diff` to compute changed_source_files + changed_doc_files_with_lines
  2. For each docs/*.md, run scan-doc-scope.mjs
  3. Apply algorithm from spec §3b
  4. Emit JSON; exit 0=clean, 1=drift, 2=missing-scope
  ```

- [ ] **2.4** Honor `docs: skip` from PR body (when `--pr <N>` mode): demote DRIFT findings to warnings, set `skipped: true` in JSON.

- [ ] **2.5** Bats tests with fixture diffs + scope declarations + expected exit codes + JSON shape. Cases:
  - No source change → exit 0, empty JSON
  - Source change matching a scoped doc section, doc edited → exit 0
  - Source change matching scope, doc NOT edited → exit 1, drift array populated
  - Source change with no scope anywhere → exit 2, missing_scope populated
  - `docs: skip` in fake PR body → exit 0 with warnings
  - `visual:` glob + source change + screenshot mtime older → visual_stale entry
  - `generated: true` section + source change → REGEN_REQUIRED hint

### Acceptance criteria
- All bats tests pass
- JSON output validates against the gate JSON sub-schema in spec §3b
- Drift algorithm matches spec exactly
- Commit: `feat(autospec-docs): scope parser + drift checker (phase 2)`

---

## Phase 3 — Self-heal loop classifier extension

**GH issue title:** `feat(autospec-docs): self-heal classifier extension (phase 3)`
**Depends on:** #351 + Phase 2 issue

### Files
- Create: `skills/autospec-shared/scripts/loop-classifier-docs-extension.mjs`
- Modify: `skills/autospec-test/scripts/loop-classifier.mjs` (register the extension module)
- Create: `skills/autospec-shared/tests/unit/loop-classifier-docs.test.mjs`

### Tasks

- [ ] **3.1** Define new category schema:
  ```ts
  type DocsCategory =
    | 'failing_doc_drift'
    | 'missing_doc_scope'
    | 'failing_visual_stale'
    | 'failing_ai_review_stale'
    | 'failing_manifest_stale';
  ```

- [ ] **3.2** Write `loop-classifier-docs-extension.mjs` exporting `classify(gateJson, lastIterations): { category: DocsCategory|null, target_files: string[], suggested_action: string, estimated_minutes: number }`.

  Input: the JSON from `check-doc-drift.sh`. Output: classification + the doc files to edit + a one-line action ("update `docs/USER_MANUAL.md` section `## Installing autospec-test` to reflect changes in `install.sh`").

- [ ] **3.3** Modify v1's `loop-classifier.mjs` to register this extension. When loop iteration sees a `docs:*` failure in the gate JSON, route through extension before falling back to default classifier.

- [ ] **3.4** Update priority ordering (per spec §3c). Insert between existing categories:
  ```
  product_bug
  > missing_unit_test
  > missing_doc_scope                ← new
  > missing_test (E2E)
  > missing_invariant
  > missing_window_contract
  > missing_contract_symmetry
  > failing_doc_drift                ← new
  > failing_visual_stale             ← new
  > failing_ai_review_stale          ← new
  > failing_manifest_stale           ← new
  > selector_brittle
  > failing_unit_test
  > ... (existing)
  ```

- [ ] **3.5** Unit tests with fixture gate-JSON payloads → assert correct classification and priority resolution.

### Acceptance criteria
- Extension module integrates with v1 classifier without breaking existing tests
- Priority ordering tested with at least one fixture per new category
- Commit: `feat(autospec-docs): self-heal classifier extension (phase 3)`

---

## Phase 4 — Reverse-engineer pipeline

**GH issue title:** `feat(autospec-docs): reverse-engineer pipeline (phase 4)`
**Depends on:** #351 + Phase 3 issue

### Files
- Create: `skills/autospec-shared/scripts/reverse-engineer.sh`
- Create: `skills/autospec-shared/scripts/reverse-engineer/inventory.mjs`
- Create: `skills/autospec-shared/scripts/reverse-engineer/cluster.mjs`
- Create: `skills/autospec-shared/scripts/reverse-engineer/emit-spec.mjs`
- Create: `skills/autospec-shared/tests/unit/reverse-engineer.test.mjs`

### Tasks

- [ ] **4.1** Write `inventory.mjs` exporting `inventory(repoRoot): Promise<Array<{ file, language, size_bytes }>>`. Walks repo respecting `.gitignore`, classifies by extension, skips `docs/`, `vendor/`, `node_modules/`, `.git/`, and any dirs declared in `.autospec/init.yml` `skip_dirs:`.

- [ ] **4.2** Write `cluster.mjs` exporting `cluster(walkOutputs: WalkOutput[]): Array<{ module_path, exports[], entry_points[], dependency_count: number, significant: boolean }>`. Groups walker output by module (directory or single-file module). Marks `significant=true` when: ≥1 public export OR is a CLI entry point OR is imported by ≥3 other modules.

- [ ] **4.3** Write `emit-spec.mjs` exporting `emitSpec(cluster, repoMeta): { architecture_spec: string, per_module_specs: Array<{ slug, content }> }`. Produces markdown specs with frontmatter:
  ```
  ---
  reverse_engineered: true
  source_root: <module_path>
  generated_at: <ISO>
  commit: <sha>
  ai_reviewed: { confidence: pending }
  ---
  ```
  Body includes module purpose (left as placeholder for Phase 8 AI fill-in), public API list, dependencies, observed test coverage (if any test files reference exports).

- [ ] **4.4** Write `reverse-engineer.sh` orchestrator:
  ```
  Input: --repo-root <dir>
  1. inventory.mjs → file list
  2. For each file, walker.mjs (parallel-safe, capped at 8 concurrent)
  3. cluster.mjs → significant units
  4. emit-spec.mjs → per-spec markdown
  5. Write specs to docs/specs/<DATE>-{architecture,<module-slug>}-reverse-engineered-design.md
  6. Detect operator-edited specs (frontmatter reverse_engineered: false OR file mtime newer than its commit:) and SKIP rewriting
  7. Emit a manifest JSON listing what was generated and what was skipped
  ```

- [ ] **4.5** Idempotency tests: run twice against the same fixture repo, assert second run produces zero diffs unless source changed.

- [ ] **4.6** Operator-edit detection test: edit a generated spec body, flip frontmatter to `reverse_engineered: false`, rerun → assert the file is NOT rewritten.

### Acceptance criteria
- Tiny fixture repo with 5 source files across 2 languages → expected specs generated
- Idempotency confirmed
- Operator-edit detection confirmed
- Commit: `feat(autospec-docs): reverse-engineer pipeline (phase 4)`

---

## Phase 5 — Initial doc generators (USER_MANUAL + API_REFERENCE + ARCHITECTURE)

**GH issue title:** `feat(autospec-docs): initial doc generators (phase 5)`
**Depends on:** #351 + Phase 4 issue

### Files
- Create: `skills/autospec-shared/scripts/gen-docs-from-spec.mjs`
- Create: `skills/autospec-shared/scripts/gen-docs/user-manual.mjs`
- Create: `skills/autospec-shared/scripts/gen-docs/api-reference.mjs`
- Create: `skills/autospec-shared/scripts/gen-docs/architecture.mjs`
- Create: `skills/autospec-shared/tests/unit/gen-docs.test.mjs`

### Tasks

- [ ] **5.1** Define interface shared across three generators:
  ```ts
  generate(input: { clusters: ClusterOutput[], specs: SpecDoc[], existingDocs: string|null }):
    { path: string, content: string, section_anchors: Array<{ heading, src_globs }> }
  ```
  `existingDocs` is the current content of the target file (for incremental updates).

- [ ] **5.2** `user-manual.mjs`: produces sections per CLI entry point + per HTTP route group. Each section has scope declaration `<!-- autospec-doc-scope: src: [<source_files>] -->` matching the entry point's source. Stub prose ("To run the X command, …") with placeholders filled in Phase 8 (AI reviewer also drafts replacement prose).

- [ ] **5.3** `api-reference.mjs`: per-public-export entry, name + signature + first-line of any leading docstring/comment + source file:line. Scope per source file.

- [ ] **5.4** `architecture.mjs`: high-level system summary (one paragraph per significant cluster) + module graph placeholder (`<!-- mermaid-graph-placeholder -->` filled in Phase 7 by `gen-arch-diagram.mjs`). Scope declared per module directory.

- [ ] **5.5** Write `gen-docs-from-spec.mjs` top-level orchestrator that calls all three. Idempotent: when run against existing docs, preserves human-edited sections (detected via missing `<!-- autospec-doc-scope: generated: true -->`) and updates only what is generated.

- [ ] **5.6** Unit tests:
  - Empty existing docs → full generation
  - Existing docs with one human-edited section → that section preserved
  - Each generator's output validates: contains scope declarations, valid markdown

### Acceptance criteria
- Generators emit valid markdown with every section scoped
- Human edits preserved on regeneration
- Commit: `feat(autospec-docs): initial doc generators (phase 5)`

---

## Phase 6 — llms.txt + manifest + assistant prompt

**GH issue title:** `feat(autospec-docs): llms.txt + manifest + assistant prompt (phase 6)`
**Depends on:** #351 + Phase 5 issue

### Files
- Create: `skills/autospec-shared/scripts/gen-llms-txt.sh`
- Create: `skills/autospec-shared/scripts/gen-llm-manifest.mjs`
- Create: `skills/autospec-shared/scripts/gen-assistant-prompt.mjs`
- Create: `skills/autospec-shared/tests/unit/gen-llms.test.mjs`

### Tasks

- [ ] **6.1** Write `gen-llms-txt.sh`:
  - `llms.txt` (short index ≤200 lines): repo summary (from README.md first paragraph + first heading after), key doc paths (USER_MANUAL/API_REFERENCE/ARCHITECTURE/specs), primary entry points (from cluster output)
  - `llms-full.txt`: concatenation of the three docs + key spec excerpts (sections marked `concept:` in spec frontmatter)
  - Both written to repo root

- [ ] **6.2** Write `gen-llm-manifest.mjs`:
  - Input: cluster output (Phase 4) + spec frontmatter (Phase 4) + doc files (Phase 5)
  - Output: `docs/.llm-manifest.json` per spec §5b shape
  - Schema validated via JSON Schema (commit `schemas/llm-manifest.schema.json`)
  - `concepts:` array extracted from doc sections tagged `<!-- autospec-concept: <name> -->`
  - `faq:` array empty on first generation; populated by Phase 8 AI reviewer when it has high-confidence Q&A pairs

- [ ] **6.3** Write `gen-assistant-prompt.mjs`:
  - Composes `docs/ASSISTANT_PROMPT.md` from a fixed template referencing `.llm-manifest.json`, `llms-full.txt`, `docs/specs/`
  - Includes sample Q&A pairs (LLM-generated during initial reverse-engineer; cached per source state)
  - Top-of-file warning: "This file is auto-generated. Edits will be overwritten."

- [ ] **6.4** Unit tests against fixture cluster output:
  - llms.txt ≤200 lines
  - llms-full.txt = concatenation of the three docs (byte-equal to a deterministic golden)
  - .llm-manifest.json validates against schema
  - Manifest idempotent: regenerate with no source change → byte-equal output

### Acceptance criteria
- llmstxt.org convention followed (top-level summary, key doc list, primary entries)
- Manifest schema validation passes
- Idempotency confirmed
- Commit: `feat(autospec-docs): llms.txt + manifest + assistant prompt (phase 6)`

---

## Phase 7 — Visual artifacts (screenshots + mermaid)

**GH issue title:** `feat(autospec-docs): screenshots + mermaid diagrams (phase 7)`
**Depends on:** #351 + Phase 6 issue

### Files
- Create: `skills/autospec-shared/scripts/gen-screenshots.mjs`
- Create: `skills/autospec-shared/scripts/gen-arch-diagram.mjs`
- Create: `skills/autospec-shared/tests/unit/gen-screenshots.test.mjs`
- Create: `skills/autospec-shared/tests/unit/gen-arch-diagram.test.mjs`

### Tasks

- [ ] **7.1** Write `gen-screenshots.mjs`:
  - Reuses v1 forbidden-url-check.mjs + network-intercept-inject.mjs (network safety identical to Stage 2)
  - Reuses v2 extended-crawler.mjs for route discovery
  - For each (route, viewport) in `{ desktop: 1280x800, mobile: 375x667 }`: page.goto → wait for `[data-loaded=true]` or 2s timeout → screenshot to `docs/assets/screenshots/<route-slug>__<viewport>.png`
  - Mode II: scope token enforcement also runs (no out-of-scope mutations during screenshot capture)

- [ ] **7.2** Write CLI transcript path: if `.autospec/init.yml` declares `cli_commands:` array, run each command via `asciinema rec --command "<cmd>"` (or fallback `script -c "<cmd>"` if asciinema unavailable). Output to `docs/assets/transcripts/<cmd-slug>.cast` (or `.txt` for fallback).

- [ ] **7.3** Write `gen-arch-diagram.mjs`:
  - Input: cluster output (Phase 4) + walker imports
  - Output: mermaid syntax strings
  - Three diagrams:
    - Top-level module graph: `graph LR` with directory clusters via mermaid `subgraph`
    - Per-entry-point call trees (depth 3): one per CLI/HTTP entry
  - Diagrams replace `<!-- mermaid-graph-placeholder -->` markers in ARCHITECTURE.md (Phase 5 inserted these)

- [ ] **7.4** Unit tests:
  - Screenshots: mock Playwright API; assert capture invocations + filenames match expected
  - Mermaid: fixture cluster output → expected mermaid string (golden byte-comparison)
  - Mode II violation test: screenshots aborted on forbidden URL match

### Acceptance criteria
- Screenshots respect Mode I + Mode II safety
- Mermaid output renders in GitHub markdown (verified by checking it parses with `mermaid-cli` in CI — or by golden against known-good output)
- Commit: `feat(autospec-docs): screenshots + mermaid diagrams (phase 7)`

---

## Phase 8 — AI-as-reviewer + confidence routing

**GH issue title:** `feat(autospec-docs): AI-as-reviewer + confidence routing (phase 8)`
**Depends on:** #351 + Phase 7 issue

### Files
- Create: `skills/autospec-shared/scripts/ai-review-doc.mjs`
- Create: `skills/autospec-shared/tests/unit/ai-review-doc.test.mjs`

### Tasks

- [ ] **8.1** Write `ai-review-doc.mjs` exporting `review({ section_heading, section_body, scope_globs, source_files_text }): Promise<{ confidence: 'high'|'medium'|'low', concerns: string[] }>`.

- [ ] **8.2** Use the deterministic prompt template from spec §7a verbatim. Strict output parser: only accept lines matching `^ai_reviewed:\s*\{\s*confidence:\s*(high|medium|low),\s*concerns:\s*\[.*\]\s*\}$` (single line). Anything else → reject + retry with adaptive directive ("ANSWER FORMAT: single line matching ai_reviewed: { confidence: ..., concerns: [...] }"). Per [[feedback_llm_validator_adaptive_retry]]: max 5 retries.

- [ ] **8.3** Caching: SHA-256 `(section_body || scope_globs sorted || source_files_concat)` → cache file at `~/.autospec/ai-review-cache/<sha>.json`. Cache hit returns cached result without LLM call.

- [ ] **8.4** Confidence routing per spec §7b:
  - high → `<!-- ai-reviewed: high -->` annotation, no label
  - medium → `<!-- ai-reviewed: medium; concerns: ... -->` annotation, no label
  - low → `<!-- ai-reviewed: low; concerns: ... -->` + caller responsible for adding `needs-human-review` + `docs:ai-low-confidence` labels + posting concerns as PR comment

- [ ] **8.5** Cost ceiling enforcement: per-section budget ≤2000 input tokens. If section body + source files exceed budget, truncate source files keeping only the FIRST occurrences of each public_export until under limit. Log truncation in concerns.

- [ ] **8.6** Module-summary integration: Phase 4's `emit-spec.mjs` left module purpose as placeholder; AI reviewer also produces module summaries during the same pass when invoked with `mode: 'summarize'`.

- [ ] **8.7** Unit tests with stubbed LLM responses:
  - High-confidence path → expected annotation
  - Medium → expected concerns formatting
  - Low → expected label suggestions
  - Cache hit → no LLM call (mock asserts zero invocations)
  - Adaptive retry: stubbed malformed-output × 4 then valid → expected confidence

### Acceptance criteria
- Confidence routing matches spec exactly
- Cache prevents redundant LLM calls
- Adaptive retry recovers from malformed LLM output
- Commit: `feat(autospec-docs): AI-as-reviewer + confidence routing (phase 8)`

---

## Phase 9 — Integration: --init mode + Phase 4 drift gate + CI workflow + pre-commit installer

**GH issue title:** `feat(autospec-docs): --init mode + Phase 4 drift gate + CI workflow (phase 9)`
**Depends on:** #351 + Phase 8 issue

### Files
- Modify: `skills/autospec-define/SKILL.md` (add --init mode + auto-docs)
- Modify: `skills/autospec-run/SKILL.md` (add Phase 4 drift gate hook)
- Modify: `skills/autospec-run/scripts/phase4-implementer.sh` (or equivalent — locate during task)
- Create: `skills/autospec-shared/scripts/install-doc-drift-hook.sh`
- Create: `skills/autospec-shared/scripts/install-doc-drift-workflow.sh`
- Create: `.github/workflows/autospec-doc-drift.yml` (template the workflow installer copies)
- Modify: top-level `install.sh` (ensure all new shared scripts get copied to `$AUTOSPEC_SCRIPTS_DIR`)

### Tasks

- [ ] **9.1** Modify `autospec-define/SKILL.md` to add the `--init` flag handling (per spec §4a):
  - Detect `docs/specs/` empty OR no doc has `autospec-doc-scope` comment
  - Prompt user: "Run reverse-engineer first? [yes / no / always-this-repo]"
  - Headless mode: `--init` flag bypasses prompt
  - `always-this-repo` writes `.autospec/init-done.flag`
  - On yes/--init: invoke `reverse-engineer.sh`, open spec-PR per existing spec-landing flow (Phase 2 of autospec-define), wait for admin-merge before continuing

- [ ] **9.2** Modify `autospec-define/SKILL.md` to add auto-docs to normal mode: after spec PR merges, invoke `gen-docs-from-spec.mjs` to update USER_MANUAL/API_REFERENCE/ARCHITECTURE/llms.txt/.llm-manifest.json. Open a doc-PR. Admin-merge. Then proceed to Phase 3 decompose.

- [ ] **9.3** Modify `autospec-run/SKILL.md` Phase 4 implementer prompt: add a step between build/lint/test and LGTM:
  ```
  Run: bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/check-doc-drift.sh" --pr <PR_NUMBER>
  Exit codes:
    0 → continue to LGTM
    1 → drift findings, classify via loop-classifier-docs-extension, feed back into self-heal loop
    2 → missing scope, label `docs:missing-scope` + needs-human-review, exit failure cleanup
  ```

- [ ] **9.4** Write `install-doc-drift-workflow.sh`: idempotent installer that drops `.github/workflows/autospec-doc-drift.yml` into the target repo if absent. Workflow runs `check-doc-drift.sh --pr ${{ github.event.pull_request.number }}` and fails the check on non-zero exit. Uses `$AUTOSPEC_SCRIPTS_DIR` from the shared install. The installer modifies the workflow content in place to point at the right script path.

- [ ] **9.5** Write `install-doc-drift-hook.sh`: optional opt-in pre-commit hook installer. Writes `.git/hooks/pre-commit` (or appends to existing) that runs `check-doc-drift.sh --working-tree`. Hook prints warnings on drift but does not block commit (local feedback only — CI is authoritative).

- [ ] **9.6** Modify top-level `install.sh` to copy `skills/autospec-shared/scripts/**` to `$AUTOSPEC_SCRIPTS_DIR` on install/update. Ensure executable bits + symlink targets remain stable.

- [ ] **9.7** Integration test: launch `autospec-run` in dry-run mode against `target-doc-drift-bait` (Phase 10), assert PR labels + report comment + exit code.

### Acceptance criteria
- `--init` mode works headless + interactive
- Auto-docs PR opens after every `/autospec-define` normal-mode spec PR
- Phase 4 drift gate exit codes plumb correctly (0=continue, 1=loop heals, 2=block)
- Workflow installer copies workflow yml idempotently
- Pre-commit hook installer is opt-in + non-blocking
- Commit: `feat(autospec-docs): --init mode + Phase 4 drift gate + CI workflow (phase 9)`

---

## Phase 10 — Synthetic targets + SKILL.md updates + autospec-self-eats-dogfood + lockstep validation

**GH issue title:** `feat(autospec-docs): synthetic targets + SKILL.md updates + lockstep (phase 10)`
**Depends on:** #351 + Phase 9 issue

### Files
- Create: `skills/autospec-shared/test-targets/target-doc-drift-bait/`
- Create: `skills/autospec-shared/test-targets/target-reverse-engineer-bait/`
- Create: `skills/autospec-shared/test-targets/target-manifest-stale-bait/`
- Create: `skills/autospec-shared/test-targets/target-visual-stale-bait/`
- Create: `skills/autospec-shared/test-targets/target-ai-low-confidence-bait/`
- Create: `skills/autospec-shared/test-targets/lang-matrix/{node,python,go,rust,jvm}/`
- Create: `skills/autospec-shared/tests/integration/run-against-target.bats`
- Modify: `skills/autospec-test/SKILL.md` (drift gate composes with Stage 2.5)
- Create: top-level `docs/USER_MANUAL.md`, `docs/API_REFERENCE.md`, `docs/ARCHITECTURE.md` (autospec dogfooding its own amendment)
- Create: top-level `llms.txt`, `llms-full.txt`, `docs/.llm-manifest.json`, `docs/ASSISTANT_PROMPT.md`
- Modify: `validate.sh` (add lockstep checks for new structural sections)

### Tasks

<!-- autospec-doc-scope:
  src: ["scripts/validate.sh"]
  reason: "Phase 10 tasks reference validate.sh lockstep checks (historical plan record)"
  mismatch_action: warn
  generated: false
-->

_Phase 2 roadmap addition (2026-05-22): `check_lockstep_duo()` added to `validate.sh` to cover
duo-harness skills (SKILL.md + codex/prompt.md, no opencode/agent.md). See PR #425._

- [ ] **10.1** Build 5 synthetic targets per spec §9b:
  - `target-doc-drift-bait`: source change without doc update → check fails with specific reason
  - `target-reverse-engineer-bait`: tiny repo with code, no docs → reverse-engineer.sh emits expected golden
  - `target-manifest-stale-bait`: docs updated, manifest not → manifest-stale flag
  - `target-visual-stale-bait`: component changed, screenshot not regenerated → visual_stale flag
  - `target-ai-low-confidence-bait`: deliberately mismatched doc + source → AI returns low confidence
  - Each ships with `.autospec/test.yml` + golden gate-JSON + golden PR-comment markdown

- [ ] **10.2** Build language matrix: 5 tiny repos (node/jest, python/pytest, go, rust, jvm/JUnit) — each with one CLI entry point + one HTTP route + one exported function. Assert `tree-sitter-walk` + `reverse-engineer.sh` produce expected outputs per language.

- [ ] **10.3** Modify `autospec-test/SKILL.md`: add a note in Stage 2.5 section that the doc-drift gate composes after Stage 2.5 (per spec §2). No code change here — just doc clarification.

- [ ] **10.4** Run `reverse-engineer.sh` against the autospec repo itself (eating its own dog food). Commits land in the same Phase 10 PR. Generated:
  - `docs/USER_MANUAL.md` covering how operators invoke autospec
  - `docs/API_REFERENCE.md` listing every autospec script's CLI surface
  - `docs/ARCHITECTURE.md` with the mermaid module graph + per-skill summaries
  - `llms.txt`, `llms-full.txt`, `docs/.llm-manifest.json`, `docs/ASSISTANT_PROMPT.md`

- [ ] **10.5** Extend `validate.sh` with lockstep checks per saved-memory pattern:
  - SKILL.md files for autospec-define + autospec-run must include the `## Docs amendment` section (presence check)
  - Adapter row block includes the doc-amendment surface
  - Required scripts present under `$AUTOSPEC_SCRIPTS_DIR` after install
  - llms.txt + docs/.llm-manifest.json exist at top-level

- [ ] **10.6** Integration test harness in `tests/integration/run-against-target.bats`: for each target, run the full amended Phase 4 implementer (using the dry-run mode of `run-gate.sh` from v1 Phase 9). Diff actual JSON + PR report markdown against golden. Goldens checked in.

- [ ] **10.7** Update top-level `README.md` + `SKILLS.md` with the new amendment.

- [ ] **10.8** End-to-end smoke: run `./validate.sh`; everything green.

### Acceptance criteria
- All 5 synthetic targets produce expected golden outputs
- Language matrix passes for all 5 languages
- autospec's own docs (USER_MANUAL/API_REFERENCE/ARCHITECTURE/llms.txt) generated and committed via this PR
- `validate.sh` lockstep checks pass
- Lockstep gotchas honored: structural sections, adapter row, no shell-out of user text, no RETURN traps, no `[ test ] && action` under set -e
- Commit: `feat(autospec-docs): synthetic targets + SKILL.md updates + lockstep (phase 10)`

---

## Cross-cutting acceptance (final gate before declaring done)

- [ ] Every phase merged to main via autospec-run
- [ ] All 5 synthetic targets golden-diff clean
- [ ] Language matrix passes for all 5 languages
- [ ] autospec dogfoods its own amendment — USER_MANUAL/API_REFERENCE/ARCHITECTURE/llms.txt all present and up-to-date in this repo
- [ ] Pre-commit hook installer is opt-in; CI workflow is opt-in via `install-doc-drift-workflow.sh`
- [ ] No commit edits `.autospec/test.yml` or shared script paths outside of operator-driven workflows

---

## Self-review

**Spec coverage:**
- §1 goal/non-goals → covered by phase boundaries
- §2 architecture → Phase 9 integration
- §3 scope declarations + drift checker → Phases 2 + 3
- §4 reverse-engineer mode → Phase 4 + Phase 9 (--init wiring)
- §5 LLM-ingestible outputs → Phase 6
- §6 visual artifacts → Phase 7
- §7 AI-as-reviewer → Phase 8
- §8 failure semantics → Phase 9 (wiring) + Phase 10 (synthetic targets validate)
- §9 testing → Phase 10
- §10 dependencies → all phases Depends-on #351; tree-sitter foundation lands in Phase 1
- §11 decision log → captured at top-level

**Placeholder scan:** clean — every task has exact file path + acceptance criterion. No TBD/TODO.

**Type consistency:**
- `WalkOutput` schema consistent (Phase 1 → 4 → 6 → 7)
- Gate JSON shape from spec §3b consistent (Phase 2 produces, Phase 3 consumes, Phase 9 acts on exit codes)
- Confidence enum `high|medium|low` consistent (Phase 8)
- File paths under `$AUTOSPEC_SCRIPTS_DIR` consistent (no vendoring into target repos)

**Open follow-ups (NOT in this plan, per spec §12):**
- Sequence-diagram auto-derivation
- Multi-language docs translation
- Visual regression baseline for screenshots
