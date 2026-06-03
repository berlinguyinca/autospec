# autospec-doc — multi-audience, always-in-sync documentation (core)

- **Date:** 2026-06-03
- **Status:** Design (Phase 2)
- **Author:** berlinguyinca (brainstormed with Claude)
- **Tracker target:** `berlinguyinca/autospec`
- **Companion spec:** `2026-06-03-autospec-doc-evolution-design.md` (Spec B — builds on this)

## Problem statement

Documentation is the basis of all user interaction and understanding of the
software, yet today it is partial and fragmented: three flat generated files
(USER_MANUAL/API_REFERENCE/ARCHITECTURE), a drift gate that *detects* but does
not *generate* during runs, audience config that no generator fully honors, no
tutorials/onboarding/getting-started, no concrete verified examples, no diagram
styling, and a stub LLM manifest. We need a single front door —
**`/autospec-doc`** — that generates audience-tailored documentation
(default: **user, developer, admin, general**) completely automatically, keeps
it in sync with code and specs on every autospec-run PR, audits completeness on
every sweep, embeds screenshots and themed diagrams where applicable, renders
**concrete examples that are executed and therefore cannot lie**, and emits both
a markdown folder structure and a single large LLM-ingest document.

## Already-handled inventory (EXTEND these — do NOT duplicate)

Paths verified 2026-06-03. The "autospec-docs-amendment" subsystem
(`docs/specs/2026-05-22-autospec-docs-amendment-design.md`) provides:

| Mechanism | Location | Status |
|---|---|---|
| Doc generator orchestrator (USER_MANUAL/API_REFERENCE/ARCHITECTURE; section-level human-edit preservation via `generated: true` scope blocks; AI-review confidence annotations + `ai-review-low-confidence.json`) | `skills/autospec-shared/scripts/gen-docs-from-spec.mjs` + `gen-docs/*.mjs` + `ai-review-doc.mjs` | working |
| Docs-drift gate (modes `--diff/--pr/--working-tree`; exit 0 clean / 1 drift / 2 missing-scope; `visual_stale`, `ai_review_stale`) | `skills/autospec-shared/scripts/check-doc-drift.sh` + `scan-doc-scope.mjs` | working |
| Doc scopes — `<!-- autospec-doc-scope: src: ["glob"] reason: ... generated: true mismatch_action: warn\|hard_fail visual_glob: ... -->` | parsed by `scan-doc-scope.mjs` | working |
| Self-heal classifier (priority-ordered actions; anti-rubber-stamp LOOSENING/STRENGTHENING/SHIFTING buckets) | `skills/autospec-shared/scripts/loop-classifier-docs-extension.mjs` | working — gains a `regenerate` action (this spec) |
| Audience + scope config | `.autospec/autospec.yml` `documentation:` block | working — schema extended (this spec) |
| Screenshots + terminal casts (Playwright desktop/mobile → `docs/assets/screenshots/`; asciinema → `docs/assets/transcripts/`; Mode-II URL safety) | `skills/autospec-qa` `gen-screenshots.mjs` | working — reused as-is |
| Mermaid arch diagram generation (unthemed; `<!-- mermaid-graph-placeholder -->` injection point) | `gen-arch-diagram.mjs` | working — gains theming (this spec) |
| Sweep doc checks (docs-drift, spec-vs-code-drift, audience/scope gap detection) | `skills/autospec-sweep/SKILL.md` + `dogfood-adapter-doc-drift.sh` | working — upgraded to invoke `/autospec-doc --full` |
| `llms.txt` index + `.llm-manifest.json` | `gen-llms-txt.sh`, `gen-llm-manifest.mjs` | manifest content is stub — filled (this spec) |
| Auto-docs step after spec merges | `skills/autospec-define/SKILL.md` (Auto-docs step) | redirected to `/autospec-doc` (this spec) |

## Goals / non-goals

**Goals**
1. New top-level skill **`/autospec-doc`** (lock-step trio) unifying all doc generation.
2. Audience-tailored content for a **configurable** audience list, default `user / developer / admin / general`, in the approved folder contract.
3. **Docs-as-tests:** every concrete example/tutorial step is executed at generation time; real output embedded; failure blocks the doc PR.
4. Sync: per-PR incremental scope regeneration inside autospec-run; full regen + completeness audit on sweep / `--full`; docs-completeness dimension in Phase 5.5.
5. Tutorials, onboarding, getting-started per feature, per audience.
6. Light-blue themed mermaid diagrams + algorithm explainers with worked, verified examples.
7. `llms-full.txt` single-document ingest + filled `.llm-manifest.json`.

**Non-goals**
- External doc frameworks (Docusaurus/MkDocs) — plain markdown only.
- A second generation engine — autospec-define's auto-docs step *redirects* here.
- Evolution timelines / Marp presentations — Spec B.
- Blocking code shipping on doc-generation failures (only the doc PR itself blocks on its own failed example verification).

## Design

### D1 — `/autospec-doc` skill (trio + scripts)

```
skills/autospec-doc/
  SKILL.md  codex/prompt.md  opencode/agent.md   # lock-step bodies
  README.md  install.sh  uninstall.sh
  scripts/
    doc-orchestrator.mjs      # subcommand router; plans scope→generator work
    gen-audience-docs.mjs     # per-audience page/tutorial/getting-started generators
    verify-examples.mjs       # docs-as-tests engine (D3)
    gen-llms-full.mjs         # llms-full.txt concatenator (D5)
    doc-style.mjs             # palette resolution + mermaid theme injection (D4)
```

Subcommands (body prose, dispatching to `doc-orchestrator.mjs`):
`/autospec-doc` (incremental — scopes affected since last generation),
`--full` (everything + completeness audit), `--audit` (read-only report),
`--audience <name>`, `init` (scaffold the `documentation:` config + starter doc
scopes by scanning the repo). Standard structural sections (Startup
self-update with `SKILL_NAME=autospec-doc`, Self-update mode, Model tier +
adapter row) per repo convention.

### D2 — Audience & content model

`.autospec/autospec.yml` (extended schema; existing `audiences:` entries keep
working — `name/path/focus/require_scope` unchanged, new optional keys):

```yaml
documentation:
  audiences:
    - {name: user,      path: docs/user,      focus: "tasks, workflows, how to use features"}
    - {name: developer, path: docs/developer, focus: "architecture, APIs, extending"}
    - {name: admin,     path: docs/admin,     focus: "install, configure, operate, troubleshoot"}
    - {name: general,   path: docs/general,   focus: "what it is, why it matters, plain language"}
  style:
    palette: light-blue        # named preset; see D4 for the resolved values
  examples:
    verify: true               # docs-as-tests on/off (default true)
    sandbox: worktree          # execution isolation
```

Folder contract per audience (the approved tree): `index.md`,
`getting-started.md`, `tutorials/<feature>.md`, `features/<feature>.md`;
developer additionally `architecture/` + `api/`; admin links `docs/runbooks/`;
shared `docs/assets/{screenshots,diagrams,transcripts}/`. Every generated
section carries an `autospec-doc-scope` comment (existing format) so the
existing drift gate governs it; `generated: true` sections are overwritten,
anything else is human-owned and preserved (existing `mergeWithExisting`
behavior).

Content tailoring: one feature → four renderings. The generator stages the
feature's spec sections + code entry points, then produces audience prose at
**Tier A** with the existing AI-review pass; every generator output runs the
repo's **validator + 5-attempt retry** discipline (findings fed back as
directives).

### D3 — Docs-as-tests example engine (`verify-examples.mjs`)

The core quality invariant: **if it's in the docs, it ran.**

1. Scan generated pages for fenced ` ```bash ` / ` ```console ` blocks tagged
   `<!-- example -->` and tutorial step sequences.
2. Execute each in an isolated sandbox (fresh worktree off origin/main;
   network-restricted; per-example timeout, default 60s).
3. Embed the real captured output in an adjacent ` ```output ` block; stamp
   `<!-- example-verified: <head-sha> <ISO-date> -->`.
4. A failing example **fails generation** → the doc PR is blocked exactly like
   a failing test. (Code PRs are never blocked by doc generation — see Error
   handling.)
5. Web walkthroughs replay through `gen-screenshots.mjs`'s Playwright path;
   CLI tutorials capture asciinema casts. Both reuse existing scripts.
6. `check-doc-drift.sh` gains an `example_stale` key: a verified-marker SHA
   older than the newest commit touching that scope's `src_globs` counts as
   drift (same self-heal path).

### D4 — Visual layer

- **Palette preset `light-blue`** resolved by `doc-style.mjs`:
  background `#E3F2FD`, primary `#90CAF9`, secondary `#BBDEFB`, accent
  `#1E88E5`, line `#64B5F6`, text `#0D2C45`. Injected as mermaid
  `%%{init: {'theme':'base','themeVariables':{...}}}%%` into every generated
  diagram (`gen-arch-diagram.mjs` gains a `--style` flag), and exported for
  Spec B's Marp theme and screenshot frame accents. One palette, everywhere.
- **Algorithm explainers:** for each spec section describing logic/flows
  (heuristic: sections with ordered steps, decision rules, or state machines),
  generate a themed mermaid flowchart/sequence diagram plus a worked example
  (real input → real output, verified via D3) on the developer feature page,
  with a simplified single-diagram version on the user page.
- Screenshot generation auto-detects applicability (web app → Playwright; CLI
  → asciinema; library → skip with INFO).

### D5 — LLM ingest

- `llms-full.txt` at repo root: deterministic concatenation of every generated
  page wrapped in `<!-- llms: audience=<a> feature=<f> -->` delimiters with
  token-budget chunk markers every ~30k tokens. Regenerated by
  `gen-llms-full.mjs` whenever any page changes (pure concatenation — cheap).
- `.llm-manifest.json` filled for real: modules, CLI entry points, concepts,
  FAQ (sourced from generated pages + specs), replacing the stub.

### D6 — Pipeline integration

| Point | Change |
|---|---|
| autospec-run Phase 4 (per-PR) | `loop-classifier-docs-extension.mjs` gains a `regenerate` action: on `drift`/`missing_scope`/`example_stale`, invoke `/autospec-doc` scoped to the affected scopes and commit the regenerated sections **into the same PR**. |
| autospec-run Phase 5.5 | New docs-completeness dimension: every feature shipped in the batch window has pages for every configured audience; no `visual_stale`/`example_stale`; gaps filed via the existing gap-remediation loop. |
| autospec-sweep | The docs-drift sweep check upgrades to `/autospec-doc --full` (regenerate + audit missing parts repo-wide). |
| autospec-define | The Auto-docs step body replaced with an invocation of `/autospec-doc` (delete the parallel path). |

### Error handling

- Generation failure during a code run: warn + `docs:failed` label + issue
  comment; **never blocks code shipping**.
- Example-verification failure: blocks only the doc PR; the failing
  command/output is the error message.
- No `documentation:` config: `/autospec-doc init` offered; other subcommands
  exit 2 with a pointer to `init`.
- Sandbox/timeout failures are reported per-example with the captured stderr.

### Testing & validation

- bats: skill CLI routing, `init` scaffolding, incremental scope selection.
- `.mjs` unit tests mirroring `gen-docs.test.mjs` for `gen-audience-docs`,
  `verify-examples` (incl. a deliberately-failing example fixture proving the
  block), `gen-llms-full`, `doc-style` (palette injection).
- Fixture repo `test-targets/target-doc-bait/` extended end-to-end: change
  code → drift detected → scoped regen → example executed → llms-full updated.
- `validate.sh`: register the new trio (lock-step + structural sections) and a
  named-content check that the light-blue palette constants live ONLY in
  `doc-style.mjs` (single source).

## Team personality

**Docs-platform engineering** — technical writer, platform/tooling engineer,
DX engineer, test engineer, accessibility/plain-language reviewer. Fits because
the product *is* documentation infrastructure; the team must notice silent
content drift, broken examples, audience-tone mismatch, and token-cost
explosions.

**Review counter-team:** maintainer + skeptical end-user + small-LLM-cost
auditor — challenge: "would a non-author actually succeed following this
tutorial?", "does incremental regen really stay cheap?", "does any generator
bypass the verify engine?"

## Risks

| Risk | Mitigation |
|---|---|
| Token cost of full regen | incremental-by-scope default; `--full` only sweep/manual |
| LLM prose drifts from truth | docs-as-tests + AI-review confidence + drift gate |
| Clobbering human edits | existing `generated: true` preservation |
| Two engines diverge | define's auto-docs step deleted/redirected — one engine |
| Example sandbox runs mutate state | fresh worktree + network restriction + timeout |

## Decomposition hint for /autospec-define

1. **Scaffold `/autospec-doc` skill trio + packaging + `init`** (structural
   first issue: Self-update, Model tier + adapter row, subcommand contract).
2. **Config schema extension + folder contract** (autospec.yml `style`/
   `examples` keys; audience defaults; migration note for operators/security).
3. **`gen-audience-docs.mjs`** — per-audience pages, tutorials,
   getting-started, onboarding (+ unit tests).
4. **`verify-examples.mjs`** — docs-as-tests engine (+ failing-example fixture).
5. **`doc-style.mjs` + mermaid theming + algorithm explainers.**
6. **`gen-llms-full.mjs` + manifest fill.**
7. **Pipeline wiring** — self-heal `regenerate` action, Phase 5.5 completeness,
   sweep `--full`, define redirect (touches autospec-run/sweep/define trios —
   serialize via deps, one trio per issue if caps require).
8. **Phase 5.5 audit issue** (standard).

> Decomposer notes: the first issue MUST carry the structural sections
> verbatim; codex/prompt.md needs the leading blank line; do NOT apply
> needs-autospec-template; issues editing OTHER skill trios (#7) must be
> serialized and lock-step-guarded like any trio edit.
