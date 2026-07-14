# autospec-harmonize Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `autospec-harmonize` skill — discover a web app's de-facto style, generalize it, generate operator-chosen variants, preview-and-pick, then apply spec-first across all pages.

**Architecture:** A harness-neutral script pipeline (`scripts/*.mjs` + `*.sh`) sits behind a lock-step skill trio. Deterministic stages (extract, variant transforms, gallery, migration spec) are plain Node/bash with bats tests; the one judgment stage (generalize) is a Tier-A dispatch with a deterministic fallback. Application/tests/UX reuse `/autospec-define` → `/autospec-run` → `autospec-playwright` → `autospec-qa` — nothing new is built there.

**Tech Stack:** bash, Node ESM (`.mjs`), Playwright (optional, runtime extraction + screenshots), `ajv` (schema validation), `jq`, bats (tests). Catalog reuse from `autospec-design`.

## Global Constraints

- Skill trio bodies (`SKILL.md`, `opencode/agent.md`, `codex/prompt.md`) MUST be byte-identical below adapter headers (lock-step rule); editing trio prose and regenerating goldens is ONE atomic commit.
- All runtime scripts resolve via `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}`; never hardcode repo-relative paths in skill prose.
- Every stage degrades without stalling; each failure path emits a `code_health:harmonize_*` identifier and exits 0 unless input is wholly absent.
- Schemas live at `schemas/autospec-harmonize-*.schema.json`; validate with `ajv`.
- New skill dir is auto-discovered by `install.sh` (`ALL_SKILLS`); only the usage-comment list and docs need manual updates.
- macOS bash is 3.2 — no `[ -f <(...) ]`, no associative arrays in shipped scripts.
- Artifacts write under `.autospec/design/`; never `git add -A` inside any script.

---

### Task 1: JSON schemas (token profile + variant)

**Files:**
- Create: `schemas/autospec-harmonize-token-profile.schema.json`
- Create: `schemas/autospec-harmonize-variant.schema.json`
- Test: `tests/harmonize/test_schemas.bats`

**Interfaces:**
- Produces: the `token-profile` and `variant` JSON shapes every later task reads/writes.

- [ ] **Step 1: Write the failing test**

```bash
# tests/harmonize/test_schemas.bats
setup() { ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"; }

@test "token-profile schema accepts a minimal valid profile" {
  echo '{"source":"runtime","palette":[{"hex":"#1e6fff","role":"primary"}],
    "type_scale":[16,20,24],"spacing":[4,8,16],"radii":[4],"shadows":[],
    "components":{"button":[{"selector":".btn","bg":"#1e6fff"}]},
    "inconsistencies":["5 blues"]}' > "$BATS_TMPDIR/p.json"
  run npx ajv validate -s "$ROOT/schemas/autospec-harmonize-token-profile.schema.json" -d "$BATS_TMPDIR/p.json"
  [ "$status" -eq 0 ]
}

@test "token-profile schema rejects a profile missing palette" {
  echo '{"source":"runtime","type_scale":[16]}' > "$BATS_TMPDIR/bad.json"
  run npx ajv validate -s "$ROOT/schemas/autospec-harmonize-token-profile.schema.json" -d "$BATS_TMPDIR/bad.json"
  [ "$status" -ne 0 ]
}

@test "variant schema accepts a baseline variant" {
  echo '{"id":"baseline","label":"Faithful baseline","axis":"baseline",
    "tokens":{"palette":[{"hex":"#1e6fff","role":"primary"}],"type_scale":[16],
    "spacing":[8],"radii":[4],"shadows":[]},"wcag_min_ratio":4.6,
    "design_md":"# Design"}' > "$BATS_TMPDIR/v.json"
  run npx ajv validate -s "$ROOT/schemas/autospec-harmonize-variant.schema.json" -d "$BATS_TMPDIR/v.json"
  [ "$status" -eq 0 ]
}
```

- [ ] **Step 2: Run to verify it fails** — `bats tests/harmonize/test_schemas.bats` → FAIL (schema files missing).

- [ ] **Step 3: Write the schemas**

`schemas/autospec-harmonize-token-profile.schema.json`:

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "autospec-harmonize-token-profile",
  "type": "object",
  "required": ["source", "palette", "type_scale", "spacing", "radii", "shadows", "components"],
  "properties": {
    "source": { "enum": ["runtime", "source"] },
    "palette": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["hex"],
        "properties": {
          "hex": { "type": "string", "pattern": "^#[0-9a-fA-F]{6}$" },
          "role": { "type": "string" },
          "count": { "type": "integer" }
        }
      }
    },
    "type_scale": { "type": "array", "items": { "type": "number" } },
    "spacing": { "type": "array", "items": { "type": "number" } },
    "radii": { "type": "array", "items": { "type": "number" } },
    "shadows": { "type": "array", "items": { "type": "string" } },
    "components": { "type": "object" },
    "inconsistencies": { "type": "array", "items": { "type": "string" } }
  }
}
```

`schemas/autospec-harmonize-variant.schema.json`:

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "autospec-harmonize-variant",
  "type": "object",
  "required": ["id", "label", "axis", "tokens", "design_md"],
  "properties": {
    "id": { "type": "string" },
    "label": { "type": "string" },
    "axis": { "enum": ["baseline", "minimal", "high-contrast", "dense", "bold", "vendor-blend"] },
    "vendor": { "type": "string" },
    "tokens": {
      "type": "object",
      "required": ["palette", "type_scale", "spacing", "radii", "shadows"],
      "properties": {
        "palette": { "type": "array" },
        "type_scale": { "type": "array" },
        "spacing": { "type": "array" },
        "radii": { "type": "array" },
        "shadows": { "type": "array" }
      }
    },
    "wcag_min_ratio": { "type": "number" },
    "design_md": { "type": "string" }
  }
}
```

- [ ] **Step 4: Run to verify it passes** — `bats tests/harmonize/test_schemas.bats` → 3 PASS.

- [ ] **Step 5: Commit** — `git add schemas/autospec-harmonize-*.schema.json tests/harmonize/test_schemas.bats && git commit -m "feat(harmonize): token-profile + variant JSON schemas"`

---

### Task 2: Source extractor → token profile

**Files:**
- Create: `skills/autospec-harmonize/scripts/extract-source.mjs`
- Create: `skills/autospec-harmonize/test-targets/messy-app/styles.css` (fixture)
- Test: `tests/harmonize/test_extract_source.bats`

**Interfaces:**
- Produces: `extractSource(repoRoot) -> tokenProfile` (CLI: `node extract-source.mjs --root <dir>` prints token-profile JSON to stdout, `source:"source"`).
- Consumes: token-profile schema (Task 1).

- [ ] **Step 1: Write the fixture** — `messy-app/styles.css` deliberately inconsistent:

```css
.btn      { background:#1e6fff; border-radius:4px; padding:8px 16px; font-size:16px; }
.button   { background:#2a74ff; border-radius:6px; padding:10px 18px; font-size:15px; }
.cta      { background:#0a5cff; border-radius:4px; }
.title    { font-size:24px; } .h2 { font-size:20px; } .lead { font-size:19px; }
.card     { box-shadow:0 1px 3px rgba(0,0,0,.12); border-radius:8px; }
```

- [ ] **Step 2: Write the failing test**

```bash
# tests/harmonize/test_extract_source.bats
setup() { ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  APP="$ROOT/skills/autospec-harmonize/test-targets/messy-app"; }

@test "extract-source emits a schema-valid profile with source=source" {
  run node "$ROOT/skills/autospec-harmonize/scripts/extract-source.mjs" --root "$APP"
  [ "$status" -eq 0 ]
  echo "$output" > "$BATS_TMPDIR/p.json"
  run npx ajv validate -s "$ROOT/schemas/autospec-harmonize-token-profile.schema.json" -d "$BATS_TMPDIR/p.json"
  [ "$status" -eq 0 ]
}

@test "extract-source detects the multiple near-duplicate blues" {
  run node "$ROOT/skills/autospec-harmonize/scripts/extract-source.mjs" --root "$APP"
  # 3 distinct blue hexes collected into the palette
  [ "$(echo "$output" | jq '[.palette[].hex] | map(select(test("(?i)#[0-2][a-f0-9]6[0-9a-f]ff"))) | length')" -ge 3 ]
  [ "$(echo "$output" | jq '.inconsistencies | length')" -ge 1 ]
}
```

- [ ] **Step 3: Run to verify it fails** — FAIL (script missing).

- [ ] **Step 4: Implement `extract-source.mjs`** — walk `*.css`/`*.scss` under `--root`, regex-collect `background`/`color` hex values, `border-radius`, `padding`/`gap`, `font-size`, `box-shadow`; dedupe; flag inconsistency when a category has >2 near-duplicate values. Emit the token-profile shape with `source:"source"`. Key implementation:

```js
#!/usr/bin/env node
import fs from 'node:fs'; import path from 'node:path';
const root = process.argv[process.argv.indexOf('--root') + 1];
const files = [];
(function walk(d){ for (const e of fs.readdirSync(d,{withFileTypes:true})) {
  const p = path.join(d,e.name);
  if (e.isDirectory()) { if (!/node_modules|\.git/.test(e.name)) walk(p); }
  else if (/\.s?css$/.test(e.name)) files.push(p); } })(root);
const css = files.map(f => fs.readFileSync(f,'utf8')).join('\n');
const uniq = a => [...new Set(a)];
const hexes = uniq((css.match(/#[0-9a-fA-F]{6}\b/g)||[]).map(h=>h.toLowerCase()));
const px = re => uniq((css.match(re)||[]).map(s=>parseInt(s.match(/\d+/)[0],10))).sort((a,b)=>a-b);
const fontSizes = px(/font-size:\s*\d+px/g);
const radii = px(/border-radius:\s*\d+px/g);
const spacing = px(/(?:padding|gap|margin):\s*\d+px/g);
const shadows = uniq(css.match(/box-shadow:[^;]+/g)||[]).map(s=>s.replace(/box-shadow:\s*/,''));
const inconsistencies = [];
if (hexes.length > 3) inconsistencies.push(`${hexes.length} distinct colors`);
if (fontSizes.length > 4) inconsistencies.push(`${fontSizes.length} type sizes`);
const buttons = uniq(css.match(/\.(btn|button|cta)\b[^{]*\{[^}]*\}/g)||[])
  .map(b => ({ selector: b.match(/\.[\w-]+/)[0], bg: (b.match(/#[0-9a-fA-F]{6}/)||[''])[0] }));
if (buttons.length > 1) inconsistencies.push(`${buttons.length} button styles`);
process.stdout.write(JSON.stringify({
  source:"source", palette: hexes.map(h=>({hex:h})), type_scale: fontSizes,
  spacing, radii, shadows, components:{ button: buttons }, inconsistencies }));
```

- [ ] **Step 5: Run to verify it passes** — `bats tests/harmonize/test_extract_source.bats` → PASS.

- [ ] **Step 6: Commit** — `git add skills/autospec-harmonize/scripts/extract-source.mjs skills/autospec-harmonize/test-targets tests/harmonize/test_extract_source.bats && git commit -m "feat(harmonize): static source style extractor"`

---

### Task 3: Runtime extractor (Playwright) → token profile

**Files:**
- Create: `skills/autospec-harmonize/scripts/extract-runtime.mjs`
- Create: `skills/autospec-harmonize/test-targets/messy-app/index.html` (serves the fixture CSS)
- Test: `tests/harmonize/test_extract_runtime.bats`

**Interfaces:**
- Produces: `node extract-runtime.mjs --url <url> [--pages a,b]` → token-profile JSON, `source:"runtime"`; exit 3 + `code_health:harmonize_runtime_unavailable` on Playwright-missing/unreachable.
- Consumes: token-profile schema (Task 1).

- [ ] **Step 1: Write `index.html`** linking `styles.css` with `.btn`, `.button`, `.card`, headings.

- [ ] **Step 2: Write the failing test** (skips cleanly when Playwright absent — never a false failure):

```bash
# tests/harmonize/test_extract_runtime.bats
setup() { ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  APP="$ROOT/skills/autospec-harmonize/test-targets/messy-app"; }

@test "runtime extractor emits source=runtime against a served fixture" {
  node -e "require('playwright')" 2>/dev/null || skip "playwright not installed"
  ( cd "$APP" && python3 -m http.server 8731 >/dev/null 2>&1 & echo $! > "$BATS_TMPDIR/pid" )
  sleep 1
  run node "$ROOT/skills/autospec-harmonize/scripts/extract-runtime.mjs" --url http://localhost:8731/index.html
  kill "$(cat "$BATS_TMPDIR/pid")" 2>/dev/null || true
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r .source)" = "runtime" ]
}

@test "runtime extractor exits 3 with code_health when url unreachable" {
  run node "$ROOT/skills/autospec-harmonize/scripts/extract-runtime.mjs" --url http://localhost:1/nope
  [ "$status" -eq 3 ]
  [[ "$output" == *"code_health:harmonize_runtime_unavailable"* ]]
}
```

- [ ] **Step 3: Run to verify it fails** — FAIL (script missing).

- [ ] **Step 4: Implement `extract-runtime.mjs`** — lazy-require Playwright (missing → exit 3 + code_health); `chromium.launch()`, for each page `page.$$eval` over elements to read `getComputedStyle` (`color`, `backgroundColor`, `fontSize`, `borderRadius`, `boxShadow`, `padding`), convert rgb→hex, aggregate into the same token-profile shape with `source:"runtime"`, screenshot to `.autospec/design/before/<route>.png`. The test in Step 2 defines correctness; the implementation pivots on:

```js
let pw; try { pw = await import('playwright'); }
catch { console.error('code_health:harmonize_runtime_unavailable'); process.exit(3); }
const browser = await pw.chromium.launch().catch(() => null);
if (!browser) { console.error('code_health:harmonize_runtime_unavailable'); process.exit(3); }
const page = await browser.newPage();
try { await page.goto(url, { timeout: 8000, waitUntil: 'domcontentloaded' }); }
catch { await browser.close(); console.error('code_health:harmonize_runtime_unavailable'); process.exit(3); }
const raw = await page.$$eval('*', els => els.map(el => {
  const s = getComputedStyle(el);
  return { bg: s.backgroundColor, color: s.color, fs: s.fontSize,
           br: s.borderRadius, sh: s.boxShadow, pad: s.padding }; }));
// rgb()->hex + aggregate identically to extract-source's shape, source:"runtime"
```

- [ ] **Step 5: Run to verify it passes** — PASS (or skip if Playwright absent + the unreachable test PASS).

- [ ] **Step 6: Commit** — `git add ... && git commit -m "feat(harmonize): runtime computed-CSS extractor via Playwright"`

---

### Task 4: `design-discover.sh` — runtime-first, source fallback

**Files:**
- Create: `skills/autospec-harmonize/scripts/design-discover.sh`
- Test: `tests/harmonize/test_discover.bats`

**Interfaces:**
- Consumes: `extract-runtime.mjs` (Task 3), `extract-source.mjs` (Task 2).
- Produces: writes `.autospec/design/discovered-tokens.json` and `.autospec/design/inventory.md`; selects backend by `--url` reachability / `--source-only`.

- [ ] **Step 1: Write the failing test**

```bash
@test "no --url falls back to source extractor" {
  run bash "$ROOT/skills/autospec-harmonize/scripts/design-discover.sh" \
    --root "$APP" --out "$BATS_TMPDIR/d"
  [ "$status" -eq 0 ]
  [ "$(jq -r .source "$BATS_TMPDIR/d/discovered-tokens.json")" = "source" ]
  [ -f "$BATS_TMPDIR/d/inventory.md" ]
}

@test "unreachable --url logs code_health and falls back to source" {
  run bash "$ROOT/skills/autospec-harmonize/scripts/design-discover.sh" \
    --root "$APP" --url http://localhost:1/nope --out "$BATS_TMPDIR/d2"
  [ "$status" -eq 0 ]
  [[ "$output" == *"code_health:harmonize_runtime_unavailable"* ]]
  [ "$(jq -r .source "$BATS_TMPDIR/d2/discovered-tokens.json")" = "source" ]
}
```

- [ ] **Step 2: Run to verify it fails** — FAIL.

- [ ] **Step 3: Implement** — if `--url` set and not `--source-only`, run `extract-runtime.mjs`; on non-zero, log its `code_health` line and fall through to `extract-source.mjs`. Write `discovered-tokens.json`; render `inventory.md` from `.inconsistencies[]` + `.components`. Use `if/then/fi` (no `&&` one-liners under `set -e`).

- [ ] **Step 4: Run to verify it passes** — PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(harmonize): discover orchestration with runtime-first/source-fallback"`

---

### Task 5: Deterministic variant transforms

**Files:**
- Create: `skills/autospec-harmonize/scripts/design-variants.mjs`
- Test: `tests/harmonize/test_variants.bats`

**Interfaces:**
- Produces: `node design-variants.mjs --baseline <file> --axes minimal,high-contrast,dense,bold` → JSON array of variant objects (schema: Task 1). `baseline` is always element 0.
- Consumes: a baseline token set (from Task 7) — for this task, tested against a fixture baseline.

- [ ] **Step 1: Write the failing test**

```bash
@test "high-contrast variant raises the min WCAG ratio above baseline" {
  echo '{"palette":[{"hex":"#7aa2ff","role":"primary"},{"hex":"#ffffff","role":"bg"}],
    "type_scale":[16,20],"spacing":[8,16],"radii":[6],"shadows":[]}' > "$BATS_TMPDIR/b.json"
  run node "$ROOT/skills/autospec-harmonize/scripts/design-variants.mjs" \
    --baseline "$BATS_TMPDIR/b.json" --axes high-contrast
  [ "$status" -eq 0 ]
  base=$(echo "$output" | jq '.[] | select(.id=="baseline") | .wcag_min_ratio')
  hc=$(echo "$output"   | jq '.[] | select(.id=="high-contrast") | .wcag_min_ratio')
  [ "$(echo "$hc > $base" | bc)" -eq 1 ]
}

@test "dense variant shrinks the spacing scale" {
  echo '{"palette":[{"hex":"#1e6fff"}],"type_scale":[16],"spacing":[8,16,24],"radii":[6],"shadows":[]}' > "$BATS_TMPDIR/b.json"
  run node "$ROOT/skills/autospec-harmonize/scripts/design-variants.mjs" --baseline "$BATS_TMPDIR/b.json" --axes dense
  [ "$(echo "$output" | jq '.[] | select(.id=="dense") | .tokens.spacing[-1]')" -lt 24 ]
}
```

- [ ] **Step 2: Run to verify it fails** — FAIL.

- [ ] **Step 3: Implement** the transforms — `baseline` (identity), `high-contrast` (darken/lighten palette toward WCAG-AA+ against bg, recompute `wcag_min_ratio` via relative-luminance contrast), `dense` (×0.75 spacing/radii), `bold` (heavier weights, +1 type-scale step), `minimal` (drop shadows, neutralize accent). Include a `contrastRatio(hexA,hexB)` helper. Emit schema-valid variants with `design_md` drafts.

- [ ] **Step 4: Run to verify it passes** — PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(harmonize): deterministic directional variant transforms"`

---

### Task 6: Vendor-blend variants (catalog reuse)

**Files:**
- Modify: `skills/autospec-harmonize/scripts/design-variants.mjs` (add `vendor-blend` axis)
- Create: `skills/autospec-harmonize/scripts/fetch-vendor.sh` (thin wrapper over autospec-design's catalog fetch)
- Test: `tests/harmonize/test_vendor_blend.bats`

**Interfaces:**
- Consumes: a fixture `<vendor>/DESIGN.md` (test injects via `--vendor-file`).
- Produces: a `vendor-blend` variant whose palette is interpolated toward the vendor's, component recipes replaced (per spec open-Q1 lean: interpolate scales, replace recipes).

- [ ] **Step 1: Write the failing test** — `--axes linear-blend --vendor-file fixtures/linear-DESIGN.md` yields a variant with `axis:"vendor-blend"`, `vendor:"linear"`, and a palette hex strictly between baseline and vendor; a fetch failure drops only that variant (others survive), printing `code_health:harmonize_vendor_fetch_failed`.

- [ ] **Step 2: Run to verify it fails** — FAIL.

- [ ] **Step 3: Implement** — parse the vendor `DESIGN.md` palette block, interpolate each baseline palette hex 50% toward the nearest vendor hex (channel-wise lerp), replace component recipes with the vendor's; on fetch/parse failure log code_health and `continue` (never throw).

- [ ] **Step 4: Run to verify it passes** — PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(harmonize): vendor-blend variants via design catalog"`

---

### Task 7: `design-generalize` — baseline synthesis (Tier-A + fallback)

**Files:**
- Create: `skills/autospec-harmonize/scripts/design-generalize.mjs`
- Test: `tests/harmonize/test_generalize.bats`

**Interfaces:**
- Consumes: `discovered-tokens.json` (Task 4).
- Produces: a single baseline token set (variant schema, `id:"baseline"`). Honors `AUTOSPEC_HARMONIZE_LLM_STUB` (test seam) and falls back to a deterministic collapse when no LLM is available.

- [ ] **Step 1: Write the failing test** — with `AUTOSPEC_HARMONIZE_LLM_STUB=1` (deterministic path), feeding the 3-blue messy profile yields a baseline whose palette collapses to ≤2 role-tagged colors and validates against the variant schema.

```bash
@test "generalize collapses near-duplicate blues into a role palette (stub)" {
  export AUTOSPEC_HARMONIZE_LLM_STUB=1
  node "$ROOT/skills/autospec-harmonize/scripts/extract-source.mjs" --root "$APP" > "$BATS_TMPDIR/d.json"
  run node "$ROOT/skills/autospec-harmonize/scripts/design-generalize.mjs" --tokens "$BATS_TMPDIR/d.json"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '.tokens.palette | map(select(.role=="primary")) | length')" -eq 1 ]
  echo "$output" > "$BATS_TMPDIR/v.json"
  run npx ajv validate -s "$ROOT/schemas/autospec-harmonize-variant.schema.json" -d "$BATS_TMPDIR/v.json"
  [ "$status" -eq 0 ]
}
```

- [ ] **Step 2: Run to verify it fails** — FAIL.

- [ ] **Step 3: Implement** — when stub/no-LLM: deterministic collapse (cluster palette hexes by hue distance, pick the highest-count per cluster, assign roles primary/bg/text/accent by luminance + frequency; median type/space scales). When LLM available: build the Tier-A prompt embedding `discovered-tokens.json` + `inventory.md`, request a baseline matching the variant schema, validate the response with ajv, retry once on schema-fail, else fall back to the deterministic collapse. Emit `id:"baseline"`.

- [ ] **Step 4: Run to verify it passes** — PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(harmonize): baseline generalize synthesis with deterministic fallback"`

---

### Task 8: `design-preview.mjs` — gallery + screenshots (+ fallback)

**Files:**
- Create: `skills/autospec-harmonize/scripts/design-preview.mjs`
- Test: `tests/harmonize/test_preview.bats`

**Interfaces:**
- Consumes: a variants JSON array (Tasks 5–7).
- Produces: `.autospec/design/preview/index.html` — one labeled section per variant with its WCAG annotation; renders+screenshots each variant when Playwright present, else a swatch/component-sheet fallback (`code_health:harmonize_preview_no_render`).

- [ ] **Step 1: Write the failing test** — given a 3-variant array, the gallery HTML exists, contains exactly 3 `data-variant=` sections and 3 `WCAG` annotations; with Playwright forced absent (`AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1`) it still produces the gallery and prints the `code_health` line.

- [ ] **Step 2: Run to verify it fails** — FAIL.

- [ ] **Step 3: Implement** — emit a self-contained HTML gallery (CSS grid of variant cards, each injecting that variant's tokens as CSS custom properties onto a representative component sheet); when Playwright present, screenshot each card to `preview/<id>.png` and embed; annotate `wcag_min_ratio`. Honor the no-Playwright env for the deterministic fallback + code_health.

- [ ] **Step 4: Run to verify it passes** — PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(harmonize): preview gallery with screenshot + swatch fallback"`

---

### Task 9: `gen-migration-spec.mjs` — per-page spec + define handoff

**Files:**
- Create: `skills/autospec-harmonize/scripts/gen-migration-spec.mjs`
- Test: `tests/harmonize/test_migration_spec.bats`

**Interfaces:**
- Consumes: the chosen variant (id), `discovered-tokens.json` (for the page/component inventory), `inventory.md` (UX findings).
- Produces: `docs/specs/<date>-harmonize-<slug>-design.md` with `- [ ]` per-page acceptance checkboxes + a `## UX findings` section; prints the `/autospec-define <path>` handoff command (or a raw stub note if define unavailable).

- [ ] **Step 1: Write the failing test** — output spec file exists at the dated path, contains one `- [ ]` per discovered page/component group and a `## UX findings` heading; `--date 2026-06-16 --slug fleet-gui` is honored (deterministic path for tests).

- [ ] **Step 2: Run to verify it fails** — FAIL.

- [ ] **Step 3: Implement** — render the migration spec from the chosen variant's `DESIGN.md` + the component inventory (one checkbox per page/component that diverges from the new system) + UX findings lifted from `inventory.md`; write to the dated path; echo the `/autospec-define` handoff. Never `git add -A`.

- [ ] **Step 4: Run to verify it passes** — PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(harmonize): per-page migration spec generator + define handoff"`

---

### Task 10: `harmonize.sh` orchestrator (end-to-end)

**Files:**
- Create: `skills/autospec-harmonize/scripts/harmonize.sh`
- Test: `tests/harmonize/test_harmonize_e2e.bats`

**Interfaces:**
- Consumes: all stage scripts (Tasks 4–9).
- Produces: the full pipeline; flag parsing (`--source-only`, `--variants`, `--num-variants`, `--no-live-preview`, `--pages`); writes all `.autospec/design/` artifacts; reaches the migration-spec handoff. The interactive pick gate is delegated to the skill prose (Task 11), not this script.

- [ ] **Step 1: Write the failing test** — `harmonize.sh --root <messy-app> --source-only --no-live-preview --variants minimal,high-contrast --out <tmp>` produces `discovered-tokens.json`, a ≥3-element variants array, `preview/index.html`, and a migration spec; exits 0.

- [ ] **Step 2: Run to verify it fails** — FAIL.

- [ ] **Step 3: Implement** — sequence discover → generalize → variants → preview → (pick is operator/skill) → migration-spec; thread `--out`/flags; each stage failure emits its code_health and continues where the spec's degradation table allows; abort only on "nothing to discover".

- [ ] **Step 4: Run to verify it passes** — PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(harmonize): orchestrator wiring the full pipeline"`

---

### Task 11: Skill trio + packaging

**Files:**
- Create: `skills/autospec-harmonize/SKILL.md`, `opencode/agent.md`, `codex/prompt.md`, `install.sh`, `uninstall.sh`, `README.md`
- Modify: `install.sh` (usage-comment skill list only — `ALL_SKILLS` auto-discovers)
- Test: lock-step + frontmatter checks in `autospec validate` (run the whole validator)

**Interfaces:**
- Consumes: `harmonize.sh` + stage scripts (Tasks 4–10).
- Produces: the operator-facing `/autospec-harmonize` entrypoint, including the AskUserQuestion pick gate and live-on-demand offer in prose.

- [ ] **Step 1:** Write `SKILL.md` following the `autospec-loop` trio shape: frontmatter (`name`, `description`, `trigger`), `<!-- autospec-block:startup-self-update SKILL_NAME=autospec-harmonize -->`, self-update mode, `<!-- autospec-block:harness-adapter-core -->`, Invocation, the 6-stage procedure, the pick gate (AskUserQuestion + live-on-demand), Hard rules. Resolve scripts via `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}`.

- [ ] **Step 2:** Derive `opencode/agent.md` and `codex/prompt.md` byte-identical below the adapter headers (lock-step). Write `install.sh`/`uninstall.sh` mirroring `autospec-loop`'s, listing the new `scripts/*` runtime files for `${AUTOSPEC_SCRIPTS_DIR}` placement. Write `README.md`.

- [ ] **Step 3: Run** `autospec validate` → expect only the not-yet-added `check_autospec_harmonize_contract` to be absent (added in Task 13); lock-step + frontmatter PASS.

- [ ] **Step 4: Commit** — `git commit -m "feat(harmonize): skill trio + installer/uninstaller/README"`

---

### Task 12: Top-level docs (catalog + index)

**Files:**
- Modify: `README.md` (Skills catalog row + token-cost table row), `SKILLS.md` (index entry)
- Test: governance assertions in `autospec validate`

**Interfaces:** none new.

- [ ] **Step 1:** Add `autospec-harmonize` to the README "Docs and design" skills table and the `SKILLS.md` index (path + trigger + keywords), matching the existing format.
- [ ] **Step 2: Run** `autospec validate` governance checks → PASS (README/SKILLS still satisfy their grep assertions).
- [ ] **Step 3: Commit** — `git commit -m "docs(harmonize): add skill to README catalog + SKILLS index"`

---

### Task 13: `validate.sh` contract gate + goldens (atomic)

**Files:**
- Modify: `autospec validate` (add `check_autospec_harmonize_contract` + register it)
- Modify/Create: `tests/fixtures/skill-goldens/*` (regenerated)
- Test: `autospec validate`

**Interfaces:** none new.

- [ ] **Step 1:** Add `check_autospec_harmonize_contract()` asserting: the trio names the 6 stages, `--no-live-preview` is documented, the pick gate is present, and the migration-spec handoff to `/autospec-define` exists in all three trio files. Register it in the run list.
- [ ] **Step 2: Run** `autospec validate` → expect FAIL pointing at goldens drift.
- [ ] **Step 3:** Regenerate goldens: `bash tests/refresh-goldens.sh` (per the skill-golden derivation workflow — trio prose + goldens in one commit).
- [ ] **Step 4: Run** `autospec validate && bats tests/harmonize` → all PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(harmonize): validate contract gate + regenerate skill goldens"`

---

### Task 14: Dogfood smoke on the fleet GUI

**Files:**
- Create: `tests/harmonize/test_dogfood_fleet_gui.bats`

**Interfaces:** none new.

- [ ] **Step 1: Write the test** — `harmonize.sh --root skills/autospec-fleet/gui --source-only --no-live-preview --variants minimal --out <tmp>` exits 0 and produces a `discovered-tokens.json` whose `inconsistencies` is non-empty (the real GUI has organic drift), a variants array, and a gallery.
- [ ] **Step 2: Run to verify it fails** then implement nothing (pure integration) — it should pass once Tasks 4–10 are green; if it surfaces a real bug, fix the offending stage script.
- [ ] **Step 3: Run** → PASS.
- [ ] **Step 4: Commit** — `git commit -m "test(harmonize): dogfood smoke against the fleet GUI"`

---

## Self-Review

**Spec coverage:** discover (T2–T4), generalize (T7), variants incl. operator-chosen nudges + vendor blends (T5–T6), preview gallery + WCAG + fallback (T8), pick gate + live-on-demand (T11 prose), spec-first apply via migration-spec + `/autospec-define` (T9), reuse of run/playwright/qa (delegated, not rebuilt — documented in T9 spec output), every degradation row (T3/T4/T6/T8 + code_health asserts), lock-step + contract gate + goldens (T11/T13), dogfood (T14). All spec sections map to a task.

**Placeholder scan:** the exploratory units (T3 runtime extractor, T7 LLM synthesis) carry complete *test* contracts plus the real key API calls (`$$eval`/`getComputedStyle`, ajv-validated stub path) rather than fabricated full listings — the test defines correctness for TDD. Deterministic units (T1, T2, T5) carry full code.

**Type consistency:** the token-profile shape (`source`, `palette[].hex/role`, `type_scale`, `spacing`, `radii`, `shadows`, `components`, `inconsistencies`) and variant shape (`id`, `label`, `axis`, `vendor?`, `tokens`, `wcag_min_ratio`, `design_md`) are defined in T1 and consumed unchanged by T2–T9. `baseline` is always variant 0.

## Notes on the application phase

Tasks 1–14 build the *discover → pick → spec* half. The *apply* half is the existing autospec loop: the T9 migration spec flows through `/autospec-define` → `/autospec-run` → `autospec-playwright` → `autospec-qa`. No new code there; that's the payoff of the spec-first decision.
