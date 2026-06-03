# autospec-doc evolution — change-over-time timelines & presentations (Spec B)

- **Date:** 2026-06-03
- **Status:** Design (Phase 2) — implement AFTER Spec A ships
- **Author:** berlinguyinca (brainstormed with Claude)
- **Tracker target:** `berlinguyinca/autospec`
- **Depends on:** `2026-06-03-autospec-doc-core-design.md` (Spec A: the
  `/autospec-doc` skill, folder contract, `doc-style.mjs` palette, drift-gate
  integration)

## Problem statement

Nothing today explains **how the software changed over time**. `CHANGELOG.md`
is hand-maintained and terse; release knowledge (what changed, why, what it
means per audience, before/after behavior) evaporates. We want generated
evolution narratives and presentation decks, in the same always-in-sync,
light-blue-styled doc system Spec A establishes.

## Goals / non-goals

**Goals**
1. **`EVOLUTION.md` timeline** — continuously generated narrative of how the
   project and each major feature changed over time.
2. **Marp markdown slide decks** per release — renderable to HTML/PDF, themed
   with Spec A's light-blue palette: what changed, why, before/after diagrams,
   migration notes.
3. Both regenerate via `/autospec-doc --evolution` and automatically during
   `autospec-release`; staleness is governed by the existing drift gate.

**Non-goals**
- Replacing `CHANGELOG.md` (it remains a source).
- Video/animation output; PowerPoint. Marp markdown only (rendering to
  HTML/PDF is the operator's `marp` CLI invocation; generation must not
  require marp to be installed).
- Rewriting history for releases that predate autospec adoption (best-effort
  from git/CHANGELOG only).

## Design

### E1 — Sources (all existing)

`CHANGELOG.md` (keep-a-changelog sections), merged-PR history
(`gh pr list --state merged` + bodies/`Closes #N` links), `docs/specs/**`
recency/supersession data (`resolve-spec-supersession.sh`), and git tags.
A deterministic collector (`evolution-collect.mjs`) normalizes these into a
per-release JSON: `{release, date, features:[{name, prs, issues, specs,
audience_impact, breaking}]}`. LLM prose is generated FROM this JSON (Tier A +
AI-review + validator-retry, per Spec A conventions) so every claim traces to a
PR/spec — no free-floating history.

### E2 — Outputs

```
docs/general/evolution/
  EVOLUTION.md                     # timeline narrative (newest first)
  <release>/slides.marp.md         # per-release deck, light-blue Marp theme
  <release>/notes.md               # per-audience "what this means for you"
docs/assets/diagrams/evolution/    # before/after mermaid pairs (themed)
```

- `EVOLUTION.md`: one H2 per release; each feature gets 2-4 sentences of
  narrative + links to its PRs/specs + its audience-impact line. A per-feature
  "history" footer section is appended to each `features/<feature>.md` page
  from Spec A (same scope/preservation rules).
- `slides.marp.md`: Marp frontmatter with a generated `light-blue` theme css
  derived from `doc-style.mjs`'s palette export; slide sequence: title →
  highlights → per-feature before/after (mermaid pair) → breaking changes →
  migration → roadmap pointer. Decks are valid standalone markdown (readable
  without rendering).
- Before/after diagrams: the generator diffs the feature's mermaid diagram
  between the prior release tag and HEAD (re-running Spec A's diagram
  generation at both refs in worktrees) and emits the pair side-by-side.

### E3 — Triggers & sync

- `/autospec-doc --evolution [--release <tag>]` — regenerate the timeline and
  the named (default: latest) release deck.
- `autospec-release` invokes it as a release-readiness step.
- Evolution outputs carry `autospec-doc-scope` comments with
  `src: ["CHANGELOG.md"]` + the release-tag ref, so the existing drift gate
  flags a new release/CHANGELOG entry without a regenerated deck as drift; the
  Phase-4 self-heal `regenerate` action (Spec A) covers it.
- `llms-full.txt` (Spec A) includes the evolution pages automatically.

### Error handling

- No tags / no CHANGELOG → emit a single "pre-history" EVOLUTION.md section
  from merged-PR history with an INFO note; never fail.
- Diagram-diff worktree failures degrade to current-state diagram only (WARN).
- Marp theme is generated css — no marp binary required at generation time.

### Testing & validation

- Unit tests for `evolution-collect.mjs` (fixture CHANGELOG + fake `gh` PR
  JSON → normalized release JSON; traceability: every narrative claim has a
  PR/spec id).
- Deck lint: generated `slides.marp.md` parses as valid Marp frontmatter +
  slides split on `---`; palette hexes match `doc-style.mjs` (single source).
- `EVOLUTION.md` regeneration is idempotent for an unchanged repo state.
- validate.sh: named-content check that evolution outputs carry doc-scope
  comments.

## Team personality / counter-team

Same as Spec A (docs-platform engineering), with added emphasis: the
storytelling must stay **traceable** (counter-team challenge: "find one
narrative sentence with no PR/spec behind it").

## Decomposition hint for /autospec-define

1. **`evolution-collect.mjs`** — deterministic source collector + unit tests.
2. **`EVOLUTION.md` + per-feature history footers** — narrative generator.
3. **Marp deck generator + light-blue theme css + before/after diagram pairs.**
4. **Wiring** — `--evolution` subcommand in the autospec-doc trio,
   autospec-release step, drift-gate scope comments. (Trio edit — lock-step.)
5. **Phase 5.5 audit issue** (standard).
