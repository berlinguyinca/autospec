# Autospec web UI generation: beautiful, animated, accessible, device-correct

**Date:** 2026-08-04
**Status:** design — approved for decomposition
**Scope:** how autospec produces user-facing web UI that is visually modern,
animated, WCAG-conformant, and correct on phones and tablets — both when
improving existing repos and when scaffolding greenfield sites.

## 1. Problem

Autospec's UI machinery today is entirely **audit-and-heal**. It can prove a UI
conforms to an adopted design language and passes automated accessibility
engines. It cannot make a UI *good*, it has no concept of motion at all, and its
responsive coverage is viewport widths rather than devices.

### 1.1 What already exists (verified 2026-08-04)

| Surface | Capability |
|---|---|
| `autospec-design` | Adopts a vendor `DESIGN.md` from the `berlinguyinca/awesome-design-md` catalog |
| `autospec-harmonize` | Token discovery → drift report → variants → migration spec |
| `scripts/lint-ui.sh` | Deterministic token-drift lint: raw hex, off-grid spacing, ad-hoc z-index, banned fonts |
| `scripts/accessibility-workstream.sh` | Gate on axe + pa11y + Lighthouse a11y 100 + IBM Equal Access, per commit, **light and dark themes both required** |
| `scripts/ux-ui-workstream.sh` | CWV budgets (LCP/INP/CLS), Lighthouse performance, token violations, visual diff, tap-target and horizontal-overflow counts |
| `autospec-qa` → `clusters/accessibility-and-responsive.md` | Viewport matrix 375/768/1280/1920, 8 responsive-cohesion verdict categories, vision-based visual-fidelity judge vs `DESIGN.md`, self-heal loop |
| `autospec-define` | `<!-- ui-feature -->` issues must carry **Design reference / Interaction states / UX flows**, enforced by the `UI_SECTIONS_INCOMPLETE` linter rule |
| `autospec-run` implementer contract | `DESIGN_DRIFT` directive pointing at `DESIGN.md` tokens + `lint-ui.sh` |
| `.autospec/design-gates.yml` + `scripts/autospec-design-gates.sh` | Opt-in baseline-pack rule-id → command mapper, with `check: auto` (executed) and `check: vlm\|review` (critic checklist) tiers |
| `autospec-ui-audit` | Deterministic route inventory (React Router only) |

### 1.2 The four gaps

**G1 — Motion does not exist at any layer.**
`grep -rn "prefers-reduced-motion\|@keyframes\|transition-duration\|motion-safe"`
across the entire repository returns **zero hits**. The cached vendor design doc
(`~/.autospec/design-cache/linear.app/DESIGN.md`, 547 lines, 30+ headings) has no
motion, animation, easing, duration, or transition section. This is not a missing
check — it is a missing vocabulary, absent from autospec *and* from the upstream
catalog.

**G2 — "Beautiful" is structurally unmeasurable.**
The visual-fidelity judge measures conformance to `DESIGN.md`, never the quality
of `DESIGN.md`. Adopt a bland design language and the pipeline enforces blandness
perfectly. Nothing in the pipeline distinguishes a well-composed screen from a
poorly-composed one that uses the right tokens.

**G3 — The WCAG claim is overstated.**
`accessibility-workstream.md` treats "0 axe violations + Lighthouse a11y 100 + 0
pa11y errors + 0 IBM violations" as WCAG 2.2 Level AA. Automated engines cover a
minority of AA success criteria. The runbook itself routes judgment-class
findings — meaningful alt text, reading and focus order, custom-widget keyboard
operability, screen-reader announcement correctness — to human review and
**rejects `auto_merged` judgment findings**. That is in direct conflict with
"automatically … completely WCAG compatible."

**G4 — Width is not device.**
The 375/768/1280/1920 matrix varies viewport width only. There are no device
profiles (UA, DPR, touch), no `pointer: coarse` / `any-hover: none`, no safe-area
insets, no orientation change, no 1.4.10 reflow at 320 CSS px, no 1.4.4 resize to
200%. Tap-target counts exist in `ux-ui-workstream.sh` and partially cover 2.5.8.

### 1.3 Prior art check

22 open issues at time of writing; none cover UI, UX, design, accessibility,
motion, responsive, or mobile work. `autospec-ui-audit`'s out-of-scope list
defers "page quality scoring, accessibility, IA, search, and visual review" to
later slices that were never filed. This is new ground.

## 2. Decisions

Four decisions were made by the operator and are treated as fixed constraints:

| Decision | Choice |
|---|---|
| Scope | **Both, existing-first.** Primary target is UI issues in existing repos; a greenfield scaffold path is secondary. |
| WCAG | **Build an AT-emulation tier** that converts much of today's judgment class into machine-verifiable assertions. |
| Source of "modern" | **Blessed stack plus accessible primitives**, not a purely stack-agnostic contract. |
| Motion vocabulary owner | **The `awesome-design-md` catalog schema**, not autospec. |

## 3. Architecture — six layers over one existing heal loop

### L0 · Catalog owns motion (`berlinguyinca/awesome-design-md`)

Every vendor `DESIGN.md` gains two sections:

- **`## Motion`** — duration scale, easing curves, choreography patterns
  (enter, exit, stagger, layout shift), an explicit statement of what animates
  and what must never animate, and a reduced-motion fallback per pattern.
- **`## Device & Input`** — `pointer: coarse` vs `fine` behavior, hover
  availability, safe-area insets, orientation handling, DPR asset density.

The catalog also gains a **`_baseline/`** entry carrying schema-conformant
defaults. Un-migrated vendor docs resolve motion from the catalog's own baseline,
so design-language ownership stays in the catalog while autospec is never blocked
waiting for every vendor doc to migrate.

### L1 · Define-time — the `ui-feature` contract grows from three sections to five

Existing: **Design reference**, **Interaction states**, **UX flows**.

Added:

- **Motion & feedback** — which catalog motion patterns this screen uses, and its
  reduced-motion fallback.
- **Device & viewport** — which device profiles must pass, plus reflow-at-320 and
  200%-zoom expectations.

Both are enforced by extending the existing `UI_SECTIONS_INCOMPLETE` rule as a
group. No new enforcement machinery.

`UX flows` additionally gains a named task and a success criterion, so a flow
becomes end-to-end testable rather than merely described.

**New define-time gate:** no `ui-feature` issue may be filed against a repo with
no adopted design language. `/autospec-define` blocks on a root `DESIGN.md`
containing a `## Motion` section and offers `/autospec-design suggest` inline.
This is the structural fix for G2 — fidelity cannot be judged against nothing.

#### L1a · Word-cap resolution (normative)

Adding two required sections to a ≤400-word issue body would break every
classified UI child: Phase 3.5/3.75 append Model-fit and Shared-contracts blocks
*after* the ≤400-word trim, so classified `ui-feature` children would
systematically trip `needs-quality-bar`.

**Decision:** the five `ui-feature` sections are **excluded from the ≤400-word
body count**. They are additionally required to use terse one-line-per-item form,
e.g. `Motion: fade-in + 40ms stagger; reduced: opacity-only`. The word cap
continues to apply to the rest of the body unchanged.

### L2 · Build-time — blessed stack and accessible primitives

**Blessed default stack:** React + Vite/Next + Tailwind + shadcn/Radix + a motion
library, with catalog tokens (motion included) emitted as CSS custom properties
and a global `prefers-reduced-motion` reset. Greenfield scaffolding always uses
this stack.

**Stack capability probe** runs at PR start for existing repos: does the repo
carry accessible component primitives and a motion library? If not, the
implementer files an adoption issue rather than hand-rolling the widget.

This is the highest-leverage layer in the design. Most judgment-class
accessibility findings — custom-widget keyboard operability, focus management,
screen-reader announcements — originate in hand-rolled widgets. Primitives make
those defects not exist, which is also what makes L4's AT-emulation residual
small enough to be worth building.

**New implementer directive `MOTION_DRIFT`**, alongside `DESIGN_DRIFT`: use the
`DESIGN.md` motion scale rather than ad-hoc durations and easings, and ship a
reduced-motion fallback with every animation.

### L3 · Deterministic lint — extend `scripts/lint-ui.sh`

Five new rules, `file:line`, run before any vision call:

| Rule | Catches | Needs motion scale? |
|---|---|---|
| `UI_NO_REDUCED_MOTION` | Module animates with no `prefers-reduced-motion` guard | No |
| `UI_INFINITE_ANIMATION` | `animation-iteration-count: infinite` over 5s with no pause control (WCAG 2.2.2) | No |
| `UI_FIXED_VIEWPORT` | `user-scalable=no` / `maximum-scale=1` (defeats WCAG 1.4.4) | No |
| `UI_HOVER_ONLY_AFFORDANCE` | Interaction available only on `:hover` with no focus or tap equivalent | No |
| `UI_RAW_DURATION` / `UI_RAW_EASING` | Hardcoded ms or `cubic-bezier(...)` off the motion scale | **Yes** |

Four of the five have **no catalog dependency** and ship immediately. Only the
raw-duration and raw-easing rules require L0's motion scale. This split is what
keeps the catalog on a parallel track rather than a blocking prefix.

### L4 · Runtime evidence

#### L4a · Device profiles replace width sweeps

Playwright device descriptors — iPhone, iPad **portrait and landscape**, Pixel,
desktop, wide — carrying UA, DPR, and touch, plus `pointer: coarse` /
`any-hover: none` assertions, safe-area inset handling, and an orientation-change
check. Two dedicated WCAG runs are added:

- **1.4.10 reflow** — 320 CSS px wide, no two-dimensional scrolling.
- **1.4.4 resize** — 200% zoom with no loss of content or function.

#### L4b · Motion becomes a second ledger axis

The accessibility ledger already requires a `light` and a `dark` row per commit.
Motion becomes a second axis: every route runs **default** and
**`prefers-reduced-motion: reduce`**, asserting:

1. **Motion actually happens by default.**
2. Motion is suppressed or reduced under the media query.
3. No flash exceeding three per second (WCAG 2.3.1).
4. Nothing auto-plays longer than 5s without a pause/stop/hide control (2.2.2).

Assertion 1 is load-bearing and must not be dropped. It is the **only check in
the entire design that fails when a UI is inert rather than wrong**. Lint, the
fidelity judge, axe, and the aesthetic rubric all pass a static, animation-free
page cleanly. It is the pipeline's sole anti-blandness gate.

#### L4c · AT-emulation tier

Three machine-verifiable assertion families that convert today's judgment class:

- **Accessibility-tree snapshot** per route, diffed against a committed baseline.
  Catches accessible-name, role, state, and reading-order regressions.
- **Keyboard robot** — tab-traverse every route asserting: no keyboard traps,
  visible focus at every stop, focus order matching visual order, focus not
  obscured (2.4.11), modal focus containment and restoration.
- **Live-region assertions** on loading, error, success, and route-change state
  transitions.

Results record into `accessibility-workstream.sh` as new counters. The runbook's
auto-remediate-vs-judgment split is then rewritten: whatever the AT tier proves
moves from judgment class to machine-verifiable. The honest residual that remains
human-reviewed is meaningful *alt semantics*, link-text meaningfulness, and plain
language.

#### L4d · Aesthetic scorecard — deferred, not designed in

A fixed vision rubric (visual hierarchy, spacing rhythm, legibility, alignment,
density, state completeness, motion coherence) recorded in the `ux-ui-workstream`
ledger and gated on **regression, not absolute score** — absolute beauty
thresholds would wedge the heal loop.

**Deferred.** With regression-only gating and vision-model noise, it cannot
distinguish "got uglier" from "the judge drifted." Before it is built, run a
stability measurement: score the same screenshot N times and measure variance.
If variance approaches the regression threshold, it is not worth building.

### L5 · Heal

All findings flow into the existing `autospec-qa` self-heal loop unchanged. No
new loop is introduced. This is deliberate — the value is in better inputs and
better evidence, not in new orchestration.

## 4. Delivery waves

| Wave | Contents | Blocking dependency |
|---|---|---|
| **1** | Four no-scale lint rules (L3); rename the accessibility gate to "WCAG 2.2 AA machine-verifiable" | None |
| **1-parallel** | Catalog `## Motion` + `## Device & Input` schema and `_baseline/` defaults (L0); then `UI_RAW_DURATION`/`UI_RAW_EASING` | External repo |
| **2** | Stack capability probe, accessible primitives, greenfield scaffold, `MOTION_DRIFT` directive (L2) | None |
| **3** | `ui-feature` sections 4-5 + word-cap fix (L1, L1a); device profiles (L4a); motion dual-run (L4b) | **Named pilot repo** |
| **4** | AT-emulation phased: keyboard robot → live regions → a11y-tree baselines (L4c); runbook judgment-split rewrite | Wave 2 |
| *deferred* | Aesthetic scorecard (L4d), gated on a stability measurement | — |

### 4.1 The interim WCAG naming fix

The operator chose to build the AT-emulation tier, which lands in Wave 4. Until
then, `accessibility-workstream.md` claims WCAG 2.2 Level AA on an
axe-plus-Lighthouse gate. Wave 1 renames it to **"WCAG 2.2 AA
machine-verifiable"**. This is an interim honesty correction covering the ~3
waves until the stronger claim is earned, not a substitute for the AT tier.

## 5. Risks

1. **Blessed stack versus "any repo."** Repos on Angular, Vue, or Svelte receive
   the contract and the gates but not the by-construction benefit. Mitigated by
   probe-then-adopt-or-file; the asymmetry is permanent and accepted.
2. **Accessibility-tree baselines create churn.** Every legitimate UI change
   updates them. Without snapshot-test review discipline, baseline updates become
   a rubber stamp and the tier stops catching regressions.
3. **The catalog is a second repository.** A genuine prerequisite for two lint
   rules and for motion content. Mitigated — not eliminated — by shipping the
   schema before the vendor migrations and by splitting the no-scale lint rules
   into Wave 1.
4. **Wave 3 has no pilot subject.** `marketing/` in this repository is seven
   markdown files, not a web surface. Wave 3's device and motion evidence
   requires a real site with real routes. **Wave 3 does not start until the
   operator names a target repo.**
5. **Vision-judge noise** across L4d and the existing fidelity judge. Existing
   convention — default to `PARTIAL` when unsure — is retained so borderline
   calls never wedge the heal loop.

## 6. Out of scope

- Native mobile applications; this covers responsive web only.
- WCAG Level AAA. The target remains 2.2 Level AA, consistent with the existing
  workstream.
- Design-language *authoring*. Autospec adopts languages from the catalog; it
  does not invent them.
- Replacing the `autospec-qa` heal loop or the existing route inventory's React
  Router limitation. Non-React router adapters remain separately scoped.
