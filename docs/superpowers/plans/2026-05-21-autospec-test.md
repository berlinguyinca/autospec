# autospec-test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Decomposition:** This plan is structured as 10 sequential phases. Each phase = one GitHub issue for `/autospec-split`. Phase ordering is intentional — Phase N depends on Phase 1..N-1 being merged.

**Goal:** Ship a new top-level autospec skill (`autospec-test`) that gates every Phase 4 PR on unit + E2E coverage against an isolated environment, self-heals failures within a 60-min coding budget, blocks assertion-loosening rewrites, and supports an opt-in scoped-production mode with mandatory backup/restore.

**Architecture:** Bash-driven skill following autospec's existing pattern (sibling of `autospec-run`, `autospec-review`). Helper logic in Node (`scripts/`) where AST parsing is needed; bash for orchestration. Contract file is `.autospec/test.yml` in target repos. Per-PR JSON state in target-repo worktree; loop pauses while tests run.

**Tech Stack:** Bash 4+, Node 20+ (`@typescript-eslint/parser`, `tree-sitter` per language for AST classifier), `gh` CLI, Playwright ≥1.40 (in target repos only), `yq` for YAML parsing, language-native coverage collectors (Istanbul/c8, coverage.py, JaCoCo, `go test -cover`, `cargo llvm-cov`).

**Spec reference:** `docs/specs/2026-05-21-autospec-test-design.md`.

---

## File Structure (locked across phases)

```
skills/autospec-test/
  SKILL.md                              # Phase 10
  prompt.md                             # Phase 10 (codex/lockstep)
  scripts/
    load-contract.sh                    # Phase 1
    autodetect.sh                       # Phase 1
    validate-contract.sh                # Phase 1
    run-gate.sh                         # Phase 9 (top-level orchestrator)
    gate-stage-unit.sh                  # Phase 2
    gate-stage-e2e.sh                   # Phase 3
    coverage-collectors/
      istanbul.sh                       # Phase 2/3
      c8.sh                             # Phase 2/3
      coverage-py.sh                    # Phase 2/3
      jacoco.sh                         # Phase 2/3
      go-cover.sh                       # Phase 2/3
      cargo-llvm-cov.sh                 # Phase 2/3
    function-presence.mjs               # Phase 2 (AST: list product fns + test references)
    playwright-config-resolver.mjs      # Phase 3
    forbidden-url-check.mjs             # Phase 3 (Layer A)
    network-intercept-inject.mjs        # Phase 3 (Layer B)
    ui-crawler.mjs                      # Phase 3 (Metric B)
    behavior-taxonomy-check.mjs         # Phase 3 (Metric D)
    findings-generator.mjs              # Phase 3 (LLM finding writer)
    assertion-shift-classifier.mjs      # Phase 4 (AST: LOOSENING/SHIFTING/STRENGTHENING)
    loop-controller.sh                  # Phase 5
    loop-classifier.mjs                 # Phase 5 (failure → category)
    loop-budget.sh                      # Phase 5 (timer w/ pause-while-tests-run)
    mode-ii-preflight.sh                # Phase 6
    mode-ii-runtime-intercept.mjs       # Phase 6
    mode-ii-postcheck.mjs               # Phase 6
    backup-drivers/
      zfs.sh                            # Phase 6
      pgdump.sh                         # Phase 6
      mysqldump.sh                      # Phase 6
      custom.sh                         # Phase 6
    wizard.sh                           # Phase 7
    pr-report.sh                        # Phase 9 (writes marker-replaced PR comment)
  test-targets/                          # Phase 8
    target-clean-pass/
    target-failing-gap/
    target-greenwash-bait/
    target-mode-ii-fixture/
  tests/
    unit/                               # Per-phase tests added 1..7
    integration/                        # Phase 8
  validate.sh                           # Phase 10 (lock-step lint)

schemas/
  autospec-test-contract.schema.json    # Phase 1 (shared with future Skill C/D)

skills/autospec-run/SKILL.md            # Phase 9 (modify: invoke autospec-test in Phase 4)
```

---

## Phase 1 — Contract loader + JSON Schema

**Files:**
- Create: `skills/autospec-test/scripts/load-contract.sh`
- Create: `skills/autospec-test/scripts/autodetect.sh`
- Create: `skills/autospec-test/scripts/validate-contract.sh`
- Create: `schemas/autospec-test-contract.schema.json`
- Create: `skills/autospec-test/tests/unit/contract-loader.bats`
- Create: `skills/autospec-test/tests/fixtures/contracts/*.yml` (valid, invalid, partial, mode-ii)

### Tasks

- [ ] **1.1** Write JSON Schema (`schemas/autospec-test-contract.schema.json`) covering the full shape from spec §3. Required fields: `mode` (enum), `e2e.forbidden_url_patterns` (array; conditionally required), `unit`, `budgets`. Mode II adds `i_understand_this_writes_to_production`, `production_scoped_access`, `backup`. Validate with `ajv` in test.

- [ ] **1.2** Write bats test fixtures covering: minimal-valid, autodetect-only, Mode II valid, Mode II missing backup, empty `forbidden_url_patterns` without ack flag (must fail), unparseable YAML.

- [ ] **1.3** Write `load-contract.sh`:
  ```
  load_contract <repo_root> → emits resolved contract JSON to stdout
    1. Read .autospec/test.yml if present (yq)
    2. For each missing field, run autodetect.sh
    3. Pipe merged result through validate-contract.sh
    4. Exit non-zero with structured error JSON on validation failure
  ```
  Signature contract for all downstream scripts: stdin/stdout JSON, exit 0 = ok, exit 1 = fatal, exit 2 = refuse-to-run (operator-actionable).

- [ ] **1.4** Write `autodetect.sh`: probes `package.json` scripts, scans for `playwright.config.*`, checks env vars (`E2E_BASE_URL`, etc.), detects language (`go.mod`/`Cargo.toml`/`pyproject.toml`/`pom.xml`/`build.gradle`). Outputs per-field defaults as JSON.

- [ ] **1.5** Write `validate-contract.sh`: invokes `ajv validate -s schema.json -d /dev/stdin`. Adds the higher-level rule: `mode=scoped_production` requires `i_understand_this_writes_to_production=true` AND `backup.driver` AND `backup.restore_cmd`. Fail-closed rule: `forbidden_url_patterns` empty without `forbidden_url_patterns_intentionally_empty=true` → refuse (exit 2).

- [ ] **1.6** Bats tests for all fixtures; assert exit code + stderr structure.

**Acceptance criteria:**
- All bats tests pass
- Schema rejects every invalid fixture; accepts every valid fixture
- `ajv` available in CI (add to autospec dev deps if absent)
- Commit message: `feat(autospec-test): contract loader + JSON schema (phase 1)`

---

## Phase 2 — Stage 1 gate: unit tests + unit coverage

**Files:**
- Create: `skills/autospec-test/scripts/gate-stage-unit.sh`
- Create: `skills/autospec-test/scripts/coverage-collectors/{istanbul,c8,coverage-py,jacoco,go-cover,cargo-llvm-cov}.sh`
- Create: `skills/autospec-test/scripts/function-presence.mjs`
- Create: `skills/autospec-test/tests/unit/gate-stage-unit.bats`
- Create: `skills/autospec-test/tests/unit/function-presence.test.mjs`

### Tasks

- [ ] **2.1** Write coverage collector adapters. Each takes lcov-or-equivalent input from running the test command, emits normalized lcov to stdout. Test each with a static lcov fixture.

- [ ] **2.2** Write `function-presence.mjs`: per-language AST walker. For JS/TS use `@typescript-eslint/parser`; for Python use `ast` via a child Python process; for Go use `go/parser` via a small Go helper; for Rust use `syn` via a small Rust helper; for JVM use `JavaParser`. Output JSON: `{ exported_functions: [{file, name, signature}], test_references: [{file, references_name}] }`. Test with synthetic source trees.

- [ ] **2.3** Write `gate-stage-unit.sh`:
  ```
  Input: resolved contract JSON (stdin)
  Steps:
    1. Run unit.test_cmd; capture stdout/stderr + exit
    2. If non-zero exit → emit gate JSON with passed=false, reason=tests_red
    3. Collect coverage via per-language collector
    4. Compare to thresholds
    5. Run function-presence.mjs; check every exported fn has ≥1 test ref
    6. Emit gate JSON (see spec §4)
  ```

- [ ] **2.4** Unit tests:
  - All collectors handle missing-lcov gracefully
  - function-presence reports correct missing-test set on fixture trees
  - gate-stage-unit emits well-formed JSON for: pass, threshold-fail, function-presence-fail, tests-red

**Acceptance criteria:**
- Bats + Node tests pass
- Stage 1 JSON output validates against a sub-schema
- Commit message: `feat(autospec-test): stage 1 unit gate (phase 2)`

---

## Phase 3 — Stage 2 gate: E2E + Layers A/B safety

**Files:**
- Create: `skills/autospec-test/scripts/gate-stage-e2e.sh`
- Create: `skills/autospec-test/scripts/playwright-config-resolver.mjs`
- Create: `skills/autospec-test/scripts/forbidden-url-check.mjs`
- Create: `skills/autospec-test/scripts/network-intercept-inject.mjs`
- Create: `skills/autospec-test/scripts/ui-crawler.mjs`
- Create: `skills/autospec-test/scripts/behavior-taxonomy-check.mjs`
- Create: `skills/autospec-test/scripts/findings-generator.mjs`
- Create: tests/unit/*.test.mjs for each

### Tasks

- [ ] **3.1** `playwright-config-resolver.mjs`: parses TS/JS/MJS/CJS config via `tsx`-style require, returns `{ baseURL, useBaseURL, webServerURL, projects, testDir }`. Test with 5 fixture configs (TS, JS, JSConfig with webServer, multi-project, missing baseURL).

- [ ] **3.2** `forbidden-url-check.mjs` (Layer A): takes resolved config + `forbidden_url_patterns`; matches every URL-shaped value against every pattern as regex. Output: `{ violations: [{ field, value, pattern }] }`. Test with positive + negative fixtures.

- [ ] **3.3** `network-intercept-inject.mjs` (Layer B): writes `playwright/global-setup-autospec.ts` into the worktree that registers `context.route('**/*', handler)` aborting requests matching forbidden patterns. Inserts a one-line require into the target's `playwright.config.*` (idempotent — detects prior insert). Test with fixture playwright configs; verify global setup file content + config patch.

- [ ] **3.4** `ui-crawler.mjs`: headless Playwright BFS crawler. Visits root URL, extracts all `<a href>` for in-domain routes, recurses to ≤200 routes. On each route, dumps every element matching the locator set from spec §4 Metric B. Outputs manifest: `{ routes: [{ url, elements: [{ selector_strategy, selector, role, accessible_name }] }] }`. Selector strategy preference: data-testid > role+name > xpath. Cap crawl at 200 routes; respect sitemap.xml if present.

- [ ] **3.5** Playwright fixture for instrumenting interactions: wraps `click`, `fill`, `selectOption`, `keyboard.press`, drag/scroll/upload primitives. Records `(route, selector)` tuples to `.autospec/touched-elements.jsonl`. Shipped as `skills/autospec-test/scripts/playwright-fixtures/touched.ts` — autodetected and prepended to target's test imports via a thin codemod (idempotent).

- [ ] **3.6** `behavior-taxonomy-check.mjs`: post-run analyzer. Reads Playwright trace files from `test-results/`, parses event stream, maps to category primitives (sort = click on `[role=columnheader]`, etc.). Output: `{ categories: { sort: { tests: [...], passed: true }, ... } }`.

- [ ] **3.7** `findings-generator.mjs`: invokes LLM (codex CLI) with trace summary + manifest + coverage; emits `.autospec/test-findings.md`. Non-blocking. Idempotent (uses content hash to avoid duplicate runs).

- [ ] **3.8** `gate-stage-e2e.sh`:
  ```
  1. Run forbidden-url-check.mjs (Layer A); refuse if violations
  2. Inject network-intercept (Layer B)
  3. If contract has egress_allowlist, log declared list (runner enforces)
  4. Run ui-crawler.mjs → manifest
  5. Run playwright_cmd with coverage
  6. Read touched-elements.jsonl, compute UI coverage
  7. Read traces, run behavior-taxonomy-check
  8. Run coverage collector on E2E lcov
  9. Run findings-generator.mjs (non-blocking)
  10. Emit gate JSON
  ```

- [ ] **3.9** Tests for each script with fixture inputs; happy-path + edge cases (no sitemap, malformed config, no traces).

**Acceptance criteria:**
- Forbidden-URL check unit tests cover every URL field in spec §5a Layer A
- Crawler caps at 200 routes; respects sitemap; uses preferred selector strategy
- Behavior-taxonomy maps every declared category to at least one trace primitive
- Commit message: `feat(autospec-test): stage 2 e2e gate + safety layers (phase 3)`

---

## Phase 4 — Assertion-shift guardrail (§5c)

**Files:**
- Create: `skills/autospec-test/scripts/assertion-shift-classifier.mjs`
- Create: per-framework AST adapter modules (`adapters/{playwright,jest,vitest,mocha,pytest,go-test,cargo-test}.mjs`)
- Create: `skills/autospec-test/tests/unit/assertion-shift/*.fixture.diff` + `expected.json`

### Tasks

- [ ] **4.1** Define the bucket schema:
  ```ts
  type Bucket = 'LOOSENING' | 'SHIFTING' | 'STRENGTHENING';
  type Verdict = { file: string; line: number; bucket: Bucket; before: string; after: string; reason: string };
  ```

- [ ] **4.2** Write adapters per framework. Each takes a single test file's before+after AST; returns `Verdict[]`. Cases per adapter:
  - Removed assertion → LOOSENING
  - `==` → `>=` / `<=` / `toContain` / `expect.anything` → LOOSENING
  - `toBeCloseTo(x, N)` where N decreased → LOOSENING
  - `toBe(N)` → `toBe(M)` same operator → SHIFTING
  - Added assertion / tightened tolerance / strengthened operator → STRENGTHENING

- [ ] **4.3** Write `assertion-shift-classifier.mjs`:
  ```
  Input: { repo_root, base_ref, head_ref }
  1. git diff --name-only base..head → list test files
  2. For each test file, detect framework (by path/extension/heuristic)
  3. Parse before+after via adapter
  4. Aggregate verdicts
  5. Decide gate result:
       any LOOSENING → bucket=block, reason=loosening
       any SHIFTING without (i) co-edited non-test file in same commit AND (ii) JUSTIFICATION: in commit msg → block, reason=unjustified-shift
       else → pass
  6. Emit JSON
  ```

- [ ] **4.4** Build fixture diff corpus: ≥30 diffs across all supported frameworks; each fixture pinned to expected verdicts JSON. Run as parameterized test.

- [ ] **4.5** Edge cases to cover with explicit tests:
  - Pure selector fix (no assertion changed) → no verdict
  - Mass-delete of a `.skip`-ed test → no verdict (skipped test removal is fine)
  - Assertion moved between files → matched by structural fingerprint, not file location
  - SHIFTING with `JUSTIFICATION:` present in commit but no co-edited product file → block
  - SHIFTING with co-edited product file but no `JUSTIFICATION:` line → block
  - SHIFTING with both → pass

**Acceptance criteria:**
- All fixture diffs produce expected verdicts
- Co-presence rule for SHIFTING tested in both directions
- Commit: `feat(autospec-test): assertion-shift AST classifier (phase 4)`

---

## Phase 5 — Self-heal loop

**Files:**
- Create: `skills/autospec-test/scripts/loop-controller.sh`
- Create: `skills/autospec-test/scripts/loop-classifier.mjs`
- Create: `skills/autospec-test/scripts/loop-budget.sh`
- Create: `.autospec/test-loop-state.json` schema doc in `schemas/`

### Tasks

- [ ] **5.1** Define loop-state JSON schema (subset of spec §6); add to `schemas/`.

- [ ] **5.2** Write `loop-budget.sh`:
  ```
  budget_start <state_file>        # init timer
  budget_pause <state_file>        # called before test run
  budget_resume <state_file>       # called after test run
  budget_remaining <state_file>    # echoes seconds left
  budget_exhausted <state_file>    # exit 0 if exhausted, 1 otherwise
  ```
  Wall clock minus paused intervals. Persist to JSON.

- [ ] **5.3** Write `loop-classifier.mjs`:
  ```
  Input: { gate_json, findings_md, last_3_iterations }
  Output: {
    classification: 'product_bug'|'missing_unit_test'|'missing_test'|
                    'selector_brittle'|'failing_unit_test'|'failing_test'|'flaky_test',
    target_failures: [...],
    suggested_files: [...],
    estimated_minutes: number
  }
  ```
  Uses priority order from spec §6. Conservative on `product_bug` (require strong evidence — e.g., test was passing on previous commit but failing now).

- [ ] **5.4** Write `loop-controller.sh`:
  ```
  Loop:
    budget_exhausted → exit, mark blocked
    iteration N >= max_loop_iterations → exit
    same_error_consecutive >= 3 → exit
    stop.flag present → exit
    Read gate JSON; if passed → exit success
    Invoke loop-classifier → action plan
    Spawn implementer subagent with action plan + allowed file set
    budget_pause
    Re-run gate
    budget_resume
    Compute new error signature; update state
    Detect empty-action 2x → exit
  ```

- [ ] **5.5** Resume semantics:
  - On controller start, if state file exists in worktree, resume
  - Otherwise pull from PR artifact (`gh run download`)
  - Otherwise init fresh

- [ ] **5.6** Error signature normalization: strip line numbers, browser tags, randomized identifiers; SHA-256 the result. Test with fixture error strings.

- [ ] **5.7** Pre-commit hook in worktree blocks edits to immutable paths during loop (`.autospec/test.yml`, `.autospec/.scoped-prod-acked-*.lock`, certain `playwright.config.*` keys).

**Acceptance criteria:**
- Budget timer correctly pauses across simulated test runs (table-driven test)
- Classifier picks correct category from fixture gate JSONs
- Loop controller exits on every documented termination condition (one test per condition)
- Pre-commit hook rejects edits to immutable paths
- Commit: `feat(autospec-test): self-heal loop (phase 5)`

---

## Phase 6 — Mode II scoped-production

**Files:**
- Create: `skills/autospec-test/scripts/mode-ii-preflight.sh`
- Create: `skills/autospec-test/scripts/mode-ii-runtime-intercept.mjs`
- Create: `skills/autospec-test/scripts/mode-ii-postcheck.mjs`
- Create: `skills/autospec-test/scripts/backup-drivers/{zfs,pgdump,mysqldump,custom}.sh`
- Create: tests/unit/mode-ii/*.test.mjs

### Tasks

- [ ] **6.1** Backup driver interface (each `.sh` exports):
  ```
  snapshot   → exit 0 on success; echoes snapshot id
  verify <id> → exit 0 if snapshot exists + verified
  restore <id> → exit 0 on success
  ```
  Implement zfs (`zfs snapshot/list/rollback`), pgdump (dump-restore), mysqldump, custom (operator-provided commands from contract).

- [ ] **6.2** `mode-ii-preflight.sh`:
  ```
  1. Verify i_understand_this_writes_to_production=true
  2. Verify backup driver present + driver self-test passes
  3. Call <driver> snapshot; record id
  4. Verify scope tokens parseable; resolve identifiers exist (DB driver probe)
  5. Verify ack lock file matches contract sha; if mismatch → refuse (require re-ack)
  6. Emit preflight JSON
  ```

- [ ] **6.3** `mode-ii-runtime-intercept.mjs`: extends Layer-B fixture. Adds scope-token check on every mutating request (POST/PUT/PATCH/DELETE). Violation → abort test + write `.autospec/.scope-violation` sentinel.

- [ ] **6.4** `mode-ii-postcheck.mjs`: queries DB driver for rows/methods touched during suite window (timestamp-bracketed query). Out-of-scope rows → invoke `<driver> restore` immediately + emit `e2e:scope-violation` + write `.autospec/.CRITICAL` sentinel if restore fails.

- [ ] **6.5** Quarantine logic: track scope-violation count in `.autospec/scoped-prod-violations.json`; on 2 consecutive → patch contract in-place setting `mode: scoped_production_quarantined` (note: this is the ONE exception to loop-immutability for `.autospec/test.yml` — only the quarantine bit can be set, by the post-check, never by the loop).

- [ ] **6.6** Tests with fixture SQLite DB (matches synthetic target from Phase 8): scope-pass, scope-violation, restore-success, restore-fail, ack-mismatch, quarantine-trigger.

**Acceptance criteria:**
- Every "refuse-to-run" rule from spec §5b enforced in test
- Restore is called on scope violation, verified
- Quarantine path tested
- Commit: `feat(autospec-test): mode-ii scoped-prod runtime (phase 6)`

---

## Phase 7 — Wizard

**Files:**
- Create: `skills/autospec-test/scripts/wizard.sh`
- Create: tests/unit/wizard.bats

### Tasks

- [ ] **7.1** Wizard prompt flow (spec §5d):
  1. Mode selection
  2. If scoped: scope token kinds + identifiers
  3. Backup driver detection (probe each driver's binary; refuse Mode II if none)
  4. Dry-run preview — print resolved constraints
  5. Require operator to type `I UNDERSTAND` literally before writing yml
  6. Write `.autospec/test.yml` + initial ack lock if Mode II

- [ ] **7.2** Non-interactive mode: `--config <yaml-fragment>` accepts preset answers for headless CI use. Same dry-run preview; ack via explicit `--ack-i-understand` flag.

- [ ] **7.3** Bats tests use expect-like input feeding; test happy paths + abort paths (refuses I-UNDERSTAND, missing backup driver, etc.).

**Acceptance criteria:**
- Wizard refuses Mode II if no backup driver detected
- Wizard refuses to proceed without literal `I UNDERSTAND`
- Headless mode works without TTY
- Commit: `feat(autospec-test): operator wizard (phase 7)`

---

## Phase 8 — Synthetic target repos + language matrix

**Files:**
- Create: `skills/autospec-test/test-targets/target-clean-pass/`
- Create: `skills/autospec-test/test-targets/target-failing-gap/`
- Create: `skills/autospec-test/test-targets/target-greenwash-bait/`
- Create: `skills/autospec-test/test-targets/target-mode-ii-fixture/`
- Create: `skills/autospec-test/test-targets/lang-matrix/{node,python,go,rust,jvm}/`
- Create: `skills/autospec-test/tests/integration/run-against-target.bats`
- Create: golden output files per target

### Tasks

- [ ] **8.1** `target-clean-pass`: tiny Vite + React app, 100% unit covered (jest), 100% E2E covered (Playwright), all 9 behavior categories represented. Golden gate JSON shows `passed: true` for both stages.

- [ ] **8.2** `target-failing-gap`: same app shape, deliberately omits drag_drop test + one button. Golden gate JSON shows specific missing entries. After loop runs (with `--dry-run` to avoid LLM expense in unit tests), expected classifier output committed as golden.

- [ ] **8.3** `target-greenwash-bait`: a real product regression in `peak_detector.ts` (returns 8 instead of 10) + a tempting test rewrite. Run assertion-shift classifier on the diff `(rewrite tests)` → asserts SHIFTING without justification → block. Then with `JUSTIFICATION:` and co-edit → pass.

- [ ] **8.4** `target-mode-ii-fixture`: SQLite-backed Express app + Playwright tests; scope token = single `family_id`. Backup driver = `custom_cmd` with `cp source backup; cp backup source`. Includes one test that deliberately mutates an out-of-scope row → assert restore + halt sentinel.

- [ ] **8.5** Language matrix subdirs: tiny "hello world + 1 test" projects in each language confirming `unit.test_cmd` autodetection + coverage collector adapter for that language.

- [ ] **8.6** Integration test harness (`run-against-target.bats`): for each target, runs the full skill (`run-gate.sh`), diffs actual gate JSON + PR comment markdown against golden. Goldens are version-controlled.

**Acceptance criteria:**
- All 4 targets produce expected golden outputs
- Language matrix passes Stage 1 for each language
- Goldens are checked in; CI diff stays clean
- Commit: `test(autospec-test): synthetic targets + language matrix (phase 8)`

---

## Phase 9 — autospec-run Phase 4 wiring + PR report

**Files:**
- Modify: `skills/autospec-run/SKILL.md` (add invocation of autospec-test in Phase 4)
- Modify: `skills/autospec-run/scripts/phase4-implementer.sh` (or equivalent — locate exact path during task)
- Create: `skills/autospec-test/scripts/run-gate.sh` (top-level orchestrator)
- Create: `skills/autospec-test/scripts/pr-report.sh`
- Create: tests/integration/phase4-integration.bats

### Tasks

- [ ] **9.1** Write `run-gate.sh` orchestrator:
  ```
  1. load-contract.sh → contract JSON
  2. gate-stage-unit.sh → unit JSON
  3. If unit failed AND mode=strict: invoke loop-controller (Stage 1 only)
  4. gate-stage-e2e.sh → e2e JSON
  5. If e2e failed: invoke loop-controller (Stage 2)
  6. assertion-shift-classifier.mjs against PR diff
  7. Compose final gate result
  8. pr-report.sh → marker-replaced PR comment
  9. Apply labels via gh
  10. Upload artifacts via gh run upload
  11. Exit 0 (proceed to merge) / 1 (block PR) / 2 (halt batch — Mode II violation)
  ```

- [ ] **9.2** `pr-report.sh`: composes the markdown comment from gate JSON (spec §7c shape). Idempotent — uses `<!-- autospec-test-report-marker -->` to replace prior comment via `gh issue comment --edit-last` or equivalent.

- [ ] **9.3** Modify autospec-run SKILL.md Phase 4 section to invoke `skills/autospec-test/scripts/run-gate.sh` after existing build/lint/unit gate. Handle the three exit codes (0/1/2). Exit-2 halt is the new batch-halt behavior for Mode II.

- [ ] **9.4** Integration test: launch autospec-run in dry-run mode against `target-failing-gap`; assert PR comment + labels + exit code shape.

- [ ] **9.5** Label set added once per skill family: `e2e:passed`, `e2e:healed`, `e2e:blocked`, `e2e:refused`, `e2e:contract-error`, `e2e:assertion-loosening`, `e2e:unjustified-shift`, `e2e:stuck-error`, `e2e:no-action`, `e2e:scoped-prod`, `e2e:scoped-prod-quarantined`, `e2e:scope-violation`, `e2e:restored`, `e2e:restore-failed`, `CRITICAL`, `needs-human-review`. Provide a `bootstrap-labels.sh` in `skills/autospec-test/scripts/` that creates them all idempotently via `gh label create --force`.

**Acceptance criteria:**
- autospec-run invokes autospec-test inline in Phase 4
- All three exit codes handled correctly (proceed / block / halt)
- PR comment is marker-replaced on subsequent runs
- All labels exist after bootstrap
- Commit: `feat(autospec-run): wire autospec-test into phase 4 (phase 9)`

---

## Phase 10 — SKILL.md + lockstep validation + docs

**Files:**
- Create: `skills/autospec-test/SKILL.md`
- Create: `skills/autospec-test/codex/prompt.md` (per autospec lockstep convention)
- Create: `skills/autospec-test/validate.sh`
- Modify: top-level `validate.sh` (add autospec-test checks)
- Modify: skills inventory adapter row (locate in autospec-define/decomposer)

### Tasks

- [ ] **10.1** Write `SKILL.md` with required structural sections (per saved memory on decomposer gotchas):
  - Frontmatter (`name`, `description`, trigger keywords)
  - `## Self-update`
  - `## Model tier` → `reasoning:standard, ctx:120k`
  - `## When to use`, `## When not to use`
  - `## How it works` (high-level — refer to spec for details)
  - `## Contract file`
  - `## Modes I and II`
  - `## Safety rails`
  - `## Self-heal loop`
  - `## Wizard`
  - Adapter row block
  - `## Stop mode` (pure prose — no `{FEATURE_DESCRIPTION}` heredocs per saved memory on no-shell-user-text)
  - **Leading blank line at top** (saved memory: codex/prompt.md needs leading blank line for lockstep)

- [ ] **10.2** Write `codex/prompt.md` for peer-review. Leading blank line. Mirrors SKILL.md structure with codex-specific tone.

- [ ] **10.3** Write per-skill `validate.sh` checks: presence of structural sections, `forbidden_url_patterns` example block, adapter row syntactically valid YAML. (Per saved memory: validate.sh has named-content checks that must be updated when section names change.)

- [ ] **10.4** Add autospec-test to top-level `validate.sh` invocation list. Add to skills inventory in autospec-define decomposer (the row block downstream skills auto-discover).

- [ ] **10.5** Add adapter row in the same first-issue structural sections that autospec-define decomposer reads.

- [ ] **10.6** Update `docs/target-repo-setup.md` (or create) with a "How to enable autospec-test in your repo" section: install Playwright, write `.autospec/test.yml`, run wizard, what labels mean.

- [ ] **10.7** Update top-level autospec README skills list.

- [ ] **10.8** End-to-end smoke: run `./validate.sh`; everything green.

**Acceptance criteria:**
- `validate.sh` passes
- All saved-memory lockstep gotchas avoided (leading blank line; no shell-out of user text; named-content sections present; adapter row, Model tier, Self-update sections present in first issue)
- Documentation entry created
- Commit: `feat(autospec-test): SKILL.md + lockstep validation + docs (phase 10)`

---

## Cross-cutting acceptance (final gate before declaring done)

- [ ] Every saved-memory feedback applied: pre-pipeline sync done before each issue; small-LLM context kept ≤120k; lockstep rules honored; `--admin` merges configured per existing settings.json pattern; validate.sh named-content checks present.
- [ ] No commit edits `.autospec/test.yml` outside the wizard or the quarantine post-check (enforced by pre-commit hook).
- [ ] All 10 phases merged to main via autospec-run.
- [ ] `target-clean-pass`, `target-failing-gap`, `target-greenwash-bait`, `target-mode-ii-fixture` golden diffs all clean.

---

## Self-review

**Spec coverage:**
- §1 goal/non-goals — covered by phase boundaries + acceptance
- §2 architecture — Phase 9 wiring
- §3 contract — Phase 1
- §4 coverage gate — Phase 2 (Stage 1) + Phase 3 (Stage 2)
- §5a Mode I safety — Phase 3 (Layers A/B)
- §5b Mode II — Phase 6
- §5c assertion-shift — Phase 4
- §5d wizard — Phase 7
- §6 self-heal loop — Phase 5
- §7 failure semantics/reporting — Phase 9
- §8 dependencies — Phase 9 (autospec-run wiring); §10 follow-ups stay out of plan
- §9 testing the skill — Phase 8 (synthetic targets) + per-phase unit tests
- §11 decision log — referenced; no implementation tasks

**Placeholder scan:** clean — no TBD/TODO; every task has a file path + acceptance criterion.

**Type consistency:** signatures and JSON shapes (`gate JSON`, `loop-state JSON`, `Verdict`) named consistently across phases. Exit-code contract (0/1/2) consistent across all scripts.

**Open follow-ups (NOT in this plan, per spec §10):**
- Skill C clone provisioner — separate spec
- Skill D suite bootstrap — separate spec
- Cross-PR flake DB — separate spec
