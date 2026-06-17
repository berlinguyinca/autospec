# autospec-harmonize — design spec

**Status:** draft · **Date:** 2026-06-16 · **Owner:** berlinguyinca

## Summary

`autospec-harmonize` is a new top-level autospec skill that takes a web app with
an inconsistent, organically-grown UI and harmonizes it into one coherent design
system. It **discovers** the app's de-facto style (runtime-first, source
fallback), **generalizes** the mess into a single token system, **generates
several style options** for the operator to **preview and pick**, then applies
the chosen system **spec-first** across every page — reusing the existing
autospec loop to implement, generate Playwright tests, and run a UX pass.

It is the inverse of `autospec-design`, which adopts an *external* vendor design
language. `autospec-harmonize` derives the system from the *app's own* style and
puts a human-in-the-loop preview-and-pick gate at the center.

## Motivation

AI-built and fast-iterated web apps accrete style drift: five blues, seven
button treatments, three type scales, ad-hoc spacing. No existing skill closes
this loop. `autospec-design` only pulls a vendor language from a catalog;
`autospec-playwright` and `autospec-qa` validate UI but do not redesign it. The
operator wants to *see* options grounded in their own app and choose, not accept
a single auto-applied verdict.

## Goals

- Discover a normalized design-token profile from a running app or its source.
- Surface concrete inconsistencies as the evidence for generalizing.
- Offer a faithful baseline plus operator-chosen variants, previewed
  side-by-side, with one explicit pick gate.
- Apply the pick spec-first via the existing `/autospec-define` → `/autospec-run`
  → `autospec-playwright` → `autospec-qa` chain.
- Degrade gracefully at every stage; never stall.

## Non-goals

- No new implementation engine: applying the design, writing tests, and the UX
  pass are existing skills (`run` / `playwright` / `qa`), wired not rebuilt.
- No light/dark theme toggle system; this harmonizes one coherent system.
- No autonomous merge to `main`; the operator reviews issues and PRs.
- v1 does not invent novel design languages — variants are bounded transforms of
  the discovered tokens or blends toward the existing vendor catalog.

## Team personality

**Frontend/product team** — frontend developer, UX designer, accessibility
reviewer, API/backend developer, QA engineer.

This is user-facing UI work whose payoff is visual consistency, a clear
preview-and-pick UX, and trustworthy generated tests — exactly a frontend/product
team's wheelhouse. Risks this team is expected to notice: visual regressions
across pages after harmonizing, WCAG contrast failures in generated variants,
brittle selectors in the generated Playwright tests, a confusing preview-pick
flow, and divergence between the runtime and source extractors producing
different token profiles for the same app. Emphasis carried into child issues:
every UI-touching child carries Design reference / Interaction states / UX flows.

## Review counter-team

**Accessibility & contract review** — accessibility auditor, API-contract
reviewer, QA/regression engineer.

This counter-team challenges the implementation team's likely blind spots: that
the discovered tokens are accurate, that each variant preserves accessibility
(AA contrast, focus states), that the migration spec actually covers every page
(not just the representative sample), and that the generated Playwright tests
assert real effects rather than merely that a page renders. Review stays inside
each issue's scope by judging it against its own acceptance criteria through the
a11y / contract / regression lens.

## Invocation

```
/autospec-harmonize [URL] [--source-only] [--pages <route,route,…>]
    [--variants minimal,high-contrast,dense,bold,linear-blend,stripe-blend,…]
    [--num-variants N] [--no-live-preview]
```

- `URL` — running app to inspect. Omitted or unreachable → source fallback.
- `--source-only` — skip runtime extraction even if a URL is given.
- `--pages` — explicit route list; otherwise routes are discovered from sitemap
  / nav links.
- `--variants` — operator-chosen axes (directional nudges and/or vendor blends).
  Default: `minimal,high-contrast`.
- `--num-variants N` — cap on generated options (default 4, incl. baseline).
- `--no-live-preview` — gallery only; never offer the live shortlist preview.

**Model tier:** Tier A for the generalize synthesis and any variant judgment;
Tier B / deterministic for extraction, transforms, and gallery rendering.

## Pipeline

### Stage 1 — Inspect (runtime-first, source fallback)

One normalized extractor interface with two backends; downstream stages never
learn which ran.

- **Runtime** (`extract-runtime.mjs`, Playwright): enumerate routes (`--pages`,
  else sitemap/nav crawl), read *computed* CSS per page, capture a "before"
  screenshot per page.
- **Source** (`extract-source.mjs`): static parse of CSS/SCSS, Tailwind config,
  and component files.

Both emit:

- `.autospec/design/discovered-tokens.json` — normalized profile: palette +
  semantic roles, type scale, spacing scale, radii, shadows, component inventory
  (buttons, inputs, cards, nav). Schema:
  `schemas/autospec-harmonize-token-profile.schema.json`.
- `.autospec/design/inventory.md` — pages × components matrix that **names the
  inconsistencies** ("7 button styles, 5 blues, 3 type scales") and flags UX /
  flow / a11y smells. This file is the generalize evidence.

### Stage 2 — Generalize

A Tier-A pass collapses the discovered tokens into one coherent system
(role-based palette, single component specs), grounded strictly in the
deterministic profile. Output: **variant 1, the faithful baseline** — minimal
visual change, consistency only, the safest pick.

### Stage 3 — Variants (operator-chosen)

Baseline + N options:

- **Directional nudges** — deterministic transforms on the token set:
  `minimal`, `high-contrast` (raise to WCAG-AA+), `dense`, `bold`.
- **Vendor blends** — `linear-blend`, `stripe-blend`, … fetch `<vendor>/DESIGN.md`
  from the `berlinguyinca/awesome-design-md` catalog (reusing `autospec-design`'s
  fetch+cache) and blend toward it.

Each variant = a full token set + a `DESIGN.md` draft. Schema:
`schemas/autospec-harmonize-variant.schema.json`.

### Stage 4 — Preview gallery

Render each variant's tokens onto 2–3 representative pages, screenshot via
Playwright, assemble one side-by-side `.autospec/design/preview/index.html`
with per-variant WCAG contrast scores annotated. Opens in the operator's browser.

### Stage 5 — Pick

`AskUserQuestion` records the choice. *Live on demand:* unless
`--no-live-preview`, offer to spin up a real server for the 1–2 shortlisted
variants before the final commit.

### Stage 6 — Apply (spec-first)

1. Write the chosen system to `DESIGN.md` at the project root (reuse
   `autospec-design apply`'s writer).
2. Generate a **per-page migration spec** at
   `docs/specs/<date>-harmonize-<slug>-design.md` (finishing the stubbed
   `autospec-design migrate`, #580), with `- [ ]` acceptance checkboxes per page
   and a dedicated **UX findings** section drawn from Stage 1's smells.
3. Hand off to `/autospec-define <spec>`. The normal loop then decomposes →
   `/autospec-run` applies per page → `autospec-playwright` writes
   visual-regression + interaction tests → `autospec-qa` runs the UX / userflow /
   a11y pass. The operator reviews the issues and PRs.

## Components

Harness-neutral, under `skills/autospec-harmonize/scripts/`:

| Unit | Purpose | Depends on |
|---|---|---|
| `harmonize.sh` | Orchestrator: stage sequencing, flags, degradation. | the units below |
| `design-discover.sh` | Run the right extractor, normalize output. | `extract-runtime.mjs`, `extract-source.mjs` |
| `extract-runtime.mjs` | Playwright computed-CSS crawler + screenshots. | Playwright |
| `extract-source.mjs` | Static CSS/Tailwind/component parser. | — |
| `design-generalize` | Tier-A synthesis prompt + schema → baseline. | token profile |
| `design-variants.sh` | Baseline + variants (transforms + catalog blends). | catalog fetch |
| `design-preview.mjs` | Render + screenshot variants → gallery HTML. | Playwright |

Reused as-is: `autospec-design` (catalog fetch, `DESIGN.md` writer, `migrate`),
`autospec-define`, `autospec-run`, `autospec-playwright`, `autospec-qa`.

## Error handling / degradation (never stall)

| Condition | Behavior |
|---|---|
| No / unreachable URL | source-only fallback; `code_health:harmonize_runtime_unavailable` |
| Playwright absent | gallery degrades to token swatches + a component sheet (no page renders); warn |
| Catalog fetch fails (a blend) | drop that one variant; keep the rest |
| No CSS / components found at all | exit with a clear "nothing to discover" message |
| `migrate` / #580 not landed | write `DESIGN.md` + a raw migration-spec stub, still hand to `define` |
| No winner picked | keep all artifacts; apply nothing; exit 0 |

## Multi-harness & lock-step

Skill ships the trio (`SKILL.md` + `opencode/agent.md` + `codex/prompt.md`),
byte-identical below adapter headers, plus `install.sh` / `uninstall.sh` /
`README.md`. New scripts are harness-neutral. Trio prose and regenerated
test/fixture/skill-goldens land as **one atomic change** (lock-step rule). A
`check_autospec_harmonize_contract` gate is added to `scripts/validate.sh`.

## Testing

bats over a synthetic fixture web app (under `skills/autospec-harmonize/test-targets/`):

- token-profile extraction (runtime fixture + source fixture → same shape);
- deterministic variant transforms (snapshot);
- preview gallery HTML structure;
- fallback paths: no URL, no Playwright, catalog-fetch failure;
- the `/autospec-define` handoff (spec written, correct path, checkboxes present).

**Dogfood target:** autospec's own fleet GUI (`skills/autospec-fleet/gui/`) and
`docs/site/`.

## Acceptance criteria

- [ ] `/autospec-harmonize <url>` writes `discovered-tokens.json` + `inventory.md`.
- [ ] `--source-only` (and unreachable URL) produces the same token-profile shape via the source extractor.
- [ ] A faithful baseline plus the operator-chosen `--variants` are generated, each with a `DESIGN.md` draft.
- [ ] A side-by-side preview gallery HTML is produced with per-variant WCAG contrast annotations.
- [ ] An explicit pick gate records the choice; `--no-live-preview` suppresses the live shortlist offer.
- [ ] On pick, `DESIGN.md` is written and a per-page migration spec with `- [ ]` checkboxes is handed to `/autospec-define`.
- [ ] Every degradation row above is reachable and emits its `code_health:*` identifier without stalling.
- [ ] The trio is lock-step-clean and `check_autospec_harmonize_contract` passes in `validate.sh`.

## Open questions

1. Variant blend math for vendor blends — full token replace vs. weighted
   interpolation of palette/scale only. (Lean: interpolate palette + type/space
   scales; replace component recipes.)
2. Whether the UX/userflow pass should also *propose* flow changes (reorder
   steps, merge screens) or only audit. v1: audit + file findings; flow redesign
   is a follow-on.
