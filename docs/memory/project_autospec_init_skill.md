---
name: project-autospec-init-skill
description: Queued — new /autospec-init skill that reverse-engineers specs + generates user manual + sets up doc-drift detection for existing repos; also amends every autospec run to always produce/update docs
metadata: 
  node_type: memory
  type: project
  wing: episodic
  drawer_class: session-log
  originSessionId: 2d7883e9-9977-428f-8919-ef9b88df12a4
---

User asked 2026-05-21, immediately after v2 issues #341–#351 were filed + classified. Verbatim:

> "please add an init skill next to autospec, which scans the repo, uses the code to generate missing spec, track down existing spec, generate proper documentation of what has been done and sets this repo up to keep tracking changes in the documentation so they don't drift apart. Generating proper documentation and a user manual in form of md files is always part of autospec, so we got everything together"

**Two distinct deliverables:**

### 1. NEW skill `/autospec-init`

Onboarding skill for existing repos that have code but no spec/docs discipline. Pipeline:

1. **Scan repo** — walk source tree, identify modules/features/APIs/CLI entry points. Use language-aware parsers (per autospec convention: tree-sitter / native AST per language).
2. **Reverse-engineer specs** — for each significant unit (module, command, API endpoint, schema), generate a backfill design spec at `docs/specs/YYYY-MM-DD-<unit>-design.md` derived from code. Cite source files + line ranges. Mark these as `reverse-engineered: true` in frontmatter so they don't pretend to be hand-authored.
3. **Track down existing specs** — scan `docs/specs/`, `docs/architecture/`, `README*.md`, top-level `*.md`, git log for design docs. Map each existing spec to the code it describes (and flag drift where the spec is outdated).
4. **Generate user-facing docs** —
   - `docs/USER_MANUAL.md` (operator-facing: install, configure, run, troubleshoot)
   - `docs/API_REFERENCE.md` (per-module / per-endpoint reference)
   - `docs/ARCHITECTURE.md` (high-level structure + module graph)
5. **Set up drift detection:**
   - Pre-commit hook: any change to a source file in a tracked-spec area writes a stub note that flags whether the spec needs an update
   - CI check: lint that fails the PR if a source change has no companion doc update in `docs/`
   - `make docs-check` / equivalent runs the same lint locally
6. **Output:** an updated `docs/specs/` directory + new docs + drift hooks + a one-page `docs/AUTOSPEC_INIT_REPORT.md` summarizing what was found, what was generated, and what still needs human review.

### 2. AMEND existing autospec family — docs as a first-class output of every run

Every `/autospec-define` / `/autospec-run` / `/autospec-split` invocation must:
- After spec lands → generate / update `docs/USER_MANUAL.md` sections touched by the feature
- After implementation merges → update `docs/API_REFERENCE.md` for any new endpoints / commands
- Drift gate runs as part of Phase 4 implementer's QA cycle (right beside build/lint/test). PR blocked if source changed without doc update.

This makes the docs guarantee universal across the autospec pipeline, not just the init skill.

**Why this matters:** the autospec ecosystem currently optimizes for *building* features but leaves the *describing-to-humans* gap to chance. Reverse-engineering specs from existing code closes the onboarding gap; drift detection keeps the closed gap closed.

**SCOPE CHOICE 2026-05-21 (user picked B):** Drop the standalone `/autospec-init` skill. Fold the same capabilities into the EXISTING autospec family as universal behavior. `/autospec-define` gains a reverse-engineer-from-existing-repo mode (detect "code without specs", run code-scan + spec backfill before brainstorm). `/autospec-run` Phase 4 implementer gains a docs-generation step (USER_MANUAL.md / API_REFERENCE.md / ARCHITECTURE.md updates) + drift gate. No new skill file, just amendments.

**Sequencing 2026-05-21 (user picked c — parallel):** Docs amendment runs IN PARALLEL with v2 implementation. v2 monitor processes #342–#351 in background; docs-amendment design + decomposition + implementation happens on its own track. Tooling optimization (per [[project_autospec_tooling_optimization]]) follows after.

**Spec landed 2026-05-22:** `docs/specs/2026-05-22-autospec-docs-amendment-design.md`, PR #358 admin-merged (504 lines).

**All design decisions locked via 7 clarifying questions:**
- Q1 trigger: B (always-on + `docs: skip` escape hatch)
- Q2 doc set: C (USER_MANUAL + API_REFERENCE + ARCHITECTURE triad)
- Q3 drift detection: B (section-level markdown-comment scope, deterministic)
- Q4 reverse-engineer trigger: C (auto-detect prompt + `--init` flag)
- Q5 reverse-engineer granularity: D (top-level ARCHITECTURE spec + per-module specs)
- Q6 language scope: D (tree-sitter universal, shared with queued tooling-optimization)
- Q7 drift enforcement: D (Phase 4 + CI + optional pre-commit, 3 layers)
- §4e amendment (user-driven): screenshots IN scope (Playwright reuse), mermaid arch diagrams IN scope (tree-sitter graph), AI-as-reviewer with confidence grading replaces blanket needs-human-review

**v2 monitor progress at checkpoint 2026-05-22:**
- Batches 1+2 complete: #342–#347 merged (PRs #352–#357). 6/10 v2 phases done.
- Batch 3 launched (background) targeting #348/#349/#350.

**Plan landed 2026-05-22:** `docs/superpowers/plans/2026-05-22-autospec-docs-amendment.md` (commit `168616d`, 594 lines, 10 phases).

**docs-amendment issues filed 2026-05-22 via /autospec-split:**
- Epic umbrella #360
- 13 children #361–#369 + #371–#374 (Phase 9 split into 9a/9b, Phase 10 into 10a/10b/10c due to ≤3 files cap)
- All `Depends on #351` (v2 final phase, OPEN at filing time) + linear chain within docs-amendment
- Labels created: `area:docs`, `area:reverse-engineer`, `area:tree-sitter`
- Phase 3.5 classification: 12 × ctx:64k, 1 × ctx:120k (#374); 10 × reasoning:medium, 3 × reasoning:deep (#364, #367, #374); 0 quality flags, 0 dep warnings, 0 cycles
- All bodies passed lint with 0–2 retries each

**v2 final batch monitor launched (background) 2026-05-22:** targeting #351 first, then will auto-pick up unblocked docs-amendment children #361 onwards. Will run BATCH_SIZE=3 then signal BATCH_COMPLETE; orchestrator relaunches until queue drains.

**Sequencing now in flight:**
- v2 #351 finishes → docs-amendment children unblock → monitor processes #361 onwards
- After docs-amendment completes → pick up tooling optimization (per [[project_autospec_tooling_optimization]])

**How to apply:** Just wait for monitor batches. Relaunch on BATCH_COMPLETE signals. When queue fully drained (ALL_DONE), invoke /autospec-define for tooling-optimization spec per the queued memory.
