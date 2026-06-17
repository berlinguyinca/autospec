# Design spec — round 2: raise `/autospec-doc` LLM-artifact content fidelity

- **Branch:** `feat/autodoc-fidelity`
- **Follows:** round 1 (`docs/specs/2026-06-16-explore-autodoc-improve-round-1-design.md`)
- **Trigger:** the round-1 preview shipped complete *structure* but placeholder-grade
  *content* in the LLM artifacts. This round closes that gap.

## Result first

Three content-fidelity defects in `skills/autospec-doc/scripts/gen-llms-full.mjs`,
all fixed scripts+tests only (no SKILL.md/trio change):

1. **Descriptions were the `src:` glob, not prose.** `pageSummary` and
   `fillManifest`'s summary loop skipped only lines that *individually* start with
   `<!--`, so the multi-line `autospec-doc-scope` comment's interior `src: [...]`
   line was returned as the "summary." Fixed: a shared `firstProseLine()` that
   skips whole comment blocks and the italic audience-boilerplate line, reaching
   the real feature summary. This corrects `llms.txt` link descriptions, manifest
   `summary`, and `llms-full.txt` per-section summaries at once.

2. **`reverse-routing` was doc→itself identity** (`docs/x.md#L1 -> docs/x.md`).
   Fixed: `parseScopeSrc()` reads each page's scope `src:` list and the block now
   emits the intended **source_file → docs** map (`src/foo.mjs -> docs/.../foo.md`),
   sorted and deterministic.

3. **Manifest `public_api` was empty** on audience pages (only `## CLI/API`
   backticks were scanned). Fixed: `extractExports()` reads each page's real
   `code_entry_points` under a `repoRoot` and records ESM exports
   (`function/const/let/var/class` + `export { … }`) and module-level Python
   `def/class`; backtick tokens remain a supplement. `fillManifest` gains an
   optional `{ repoRoot = process.cwd() }`.

## Out of scope
- Reshaping `modules[]` away from one-entry-per-page (kept for backward
  compatibility; `public_api` now carries the real code surface instead).
- Glossary auto-extraction beyond existing `<!-- autospec-concept: -->` markers.
- Mermaid node-label polish (Track D heuristic).

## Acceptance criteria
- [ ] `llms.txt` link descriptions and manifest `summary` are the feature prose, never `src: [`.
- [ ] `reverse-routing` maps source files to docs; no `#L1 -> ` identity lines.
- [ ] `fillManifest(manifest, pages, { repoRoot })` populates `public_api` from real exports; non-exported symbols excluded.
- [ ] `generateLlmsFull` stays byte-identical across runs on the same input.
- [ ] `node --test skills/autospec-doc/tests/gen-llms-full.test.mjs` passes; `bash scripts/validate.sh` exits 0.

## Verification
- Primary: `node --test skills/autospec-doc/tests/gen-llms-full.test.mjs`
- Full: `bash scripts/validate.sh` + a regenerated preview showing prose descriptions, source→doc routing, and populated `public_api`.
