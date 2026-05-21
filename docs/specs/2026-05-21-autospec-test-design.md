# autospec-test — Unit + E2E Coverage Gate with Self-Heal Loop

**Status:** Draft design (2026-05-21)
**Author:** berlinguyinca + brainstorm
**Skill name:** `autospec-test` (new top-level autospec skill)
**Family position:** Skill A of three (A=this, C=clone provisioner follow-on, D=suite bootstrap future)

## 1. Goal & non-goals

### Goal
Enforce that every PR produced by `/autospec-run` ships with thorough, passing unit + E2E test coverage against an isolated environment (clone of production), and that any gaps the gate finds get auto-healed by a bounded LLM loop within the same PR. Coverage spans code lines/branches/functions, every reachable UI element, and a declared behavior taxonomy (sort, scroll, upload, download, filter, paginate, bulk-select, keyboard-nav, drag-drop). Tests **may** target production under tightly scoped, operator-acknowledged opt-in (Mode II) with mandatory backup/restore guardrails.

### Non-goals
- Clone provisioning (Skill C, separate spec)
- Bootstrapping a Playwright or unit suite into a repo that has none (Skill D)
- Cross-PR flake DB / historical test analytics
- Multi-browser matrix policy (target repo's `playwright.config` is authoritative)
- CI choice (GitHub Actions vs other) — skill runs inside autospec-run worktree

## 2. Architecture & integration

Two invocation contexts:

- **Inline (primary):** `/autospec-run` Phase 4, after the existing build+lint gate passes and before admin auto-merge. Same per-PR worktree, same model tier (`reasoning:standard, ctx:120k`).
- **Standalone:** `/autospec-test [PR#]` for ad-hoc validation against a branch / PR / main.

Two-stage gate inside the skill:

```
Phase 4 existing: build + lint
        │
        ▼
Stage 1: unit tests + unit-coverage gate
        │ (must pass before Stage 2 starts)
        ▼
Stage 2: E2E tests + E2E coverage gate
        │
        ▼
Assertion-shift guardrail (covers both stages' test diffs)
        │
        ▼
Auto-merge or block
```

Both stages share one 60-minute coding-time budget for the self-heal loop. Test runtime itself is unbounded (suites can legitimately run hours to days).

## 3. Contract

**File:** `.autospec/test.yml` in target repo. Most fields autodetected; the file's primary purpose is declaring **forbidden URLs** (mandatory) and any Mode-II opt-in.

Resolution order per field: explicit yml → autodetect → fail-closed if neither.

```yaml
# .autospec/test.yml
mode: strict_isolation                          # or: scoped_production

unit:
  test_cmd: "npm test -- --coverage"            # auto: pkg script test / pytest / go test / cargo test / mvn test
  coverage_collector: istanbul                   # auto: per language
  coverage_thresholds: { lines: 95, branches: 90, functions: 95 }
  function_presence_check: true                  # every public/exported function must have ≥1 unit test
  coverage_exclude_globs: ["**/*.gen.ts", "migrations/**"]

e2e:
  clone_url_env: E2E_BASE_URL                   # auto: E2E_BASE_URL → PLAYWRIGHT_BASE_URL → BASE_URL
  start_cmd: "npm run start:e2e"                # auto: pkg scripts start:e2e → dev → (none)
  playwright_cmd: "npx playwright test"         # auto: pkg scripts test:e2e → e2e → npx playwright test
  playwright_config: "playwright.config.ts"     # auto: glob playwright.config.{ts,js,mjs,cjs}
  coverage_cmd: "npx playwright test"           # auto: same + COVERAGE=1 env
  coverage_collector: istanbul
  coverage_thresholds:
    lines: 90
    branches: 85
    functions: 90
    ui_elements: 100                            # every reachable element must be touched
    behavior_categories:
      - sort
      - scroll
      - upload
      - download
      - filter
      - paginate
      - bulk_select
      - keyboard_nav
      - drag_drop

  # MANDATORY — fail-closed if missing or empty (without explicit ack flag)
  forbidden_url_patterns:
    - "^https?://app\\.acme\\.com"
    - "^https?://.*\\.prod\\.acme\\.internal"
  # forbidden_url_patterns_intentionally_empty: true   # required to use []

  egress_allowlist:                             # optional, Layer C
    - "clone.staging.acme.internal"
    - "10.0.0.0/8"

  # Mode II only — see §5 Safety
  # i_understand_this_writes_to_production: true
  # production_scoped_access: { ... }
  # backup: { ... }

budgets:
  coding_time_minutes: 60
  max_loop_iterations: 5
  same_error_halt_threshold: 3
```

**Schema:** JSON Schema at `schemas/autospec-test-contract.schema.json` in autospec repo; shared by future Skills C and D.

**Loop-immutable fields:** the self-heal loop is forbidden from editing `.autospec/test.yml`, `.autospec/.scoped-prod-acked-*.lock`, or `playwright.config.*` safety-related fields. Pre-commit hook in worktree enforces.

## 4. Coverage gate (three E2E metrics + one unit metric)

### Stage 1 — Unit (Metric E)
Three sub-checks, all must pass:
- Code coverage ≥ thresholds (defaults 95/90/95 lines/branches/functions)
- Function-presence: every exported/public function in product code has ≥1 unit test
- All unit tests pass; no `.only`, no `.skip`, no `xit`

Frameworks autodetected: jest, vitest, mocha, pytest, unittest, `go test`, `cargo test`, JUnit.

### Stage 2 — E2E

**Metric A — code coverage from E2E run:** Istanbul/c8/coverage.py/JaCoCo per language; lcov consumed; thresholds 90/85/90 lines/branches/functions; excludes per `coverage_exclude_globs`.

**Metric B — UI element coverage:**
- Crawler pass (BFS from root URL, ≤200 routes, or sitemap.xml if present) builds manifest of every `[role=button], button, a, input, select, textarea, [contenteditable], [tabindex], [onclick]` keyed by `route + stable_selector` (data-testid > role+text > xpath).
- Playwright fixture wraps interaction APIs (`click`, `fill`, `selectOption`, keyboard, drag, scroll, upload) and records `(route, selector)` touched.
- Gate: `crawled − touched = 0` (or ≤ `ui_element_tolerance`).

**Metric D — behavior taxonomy:**
- Tests tag themselves via `test.info().annotations.push({ type: 'category', description: 'sort' })` or `@category(sort)` JSDoc.
- For each declared category: ≥1 test exists AND passes AND its Playwright trace contains the expected primitive (e.g., `sort` requires ≥1 click on a `[role=columnheader]`-like element; rule table ships with skill, overridable).

**LLM finding generator (NOT a gate):** after every run, LLM reviews trace + manifest + coverage and emits `.autospec/test-findings.md` with suggestions. Feeds the self-heal loop's next iteration as candidate work and is posted as a PR comment regardless of pass/fail.

### Gate result schema

```json
{
  "passed": false,
  "stage": "e2e",
  "metrics": {
    "unit": { "passed": true, "lines": 96.2, "branches": 91.4, "functions": 97.0,
              "missing_function_tests": [] },
    "code_coverage": { "passed": true, "lines": 92.1, "branches": 86.4, "functions": 91.0 },
    "ui_element_coverage": { "passed": false,
      "missing": [{ "route": "/dashboard", "selector": "button[data-testid=export-csv]" }] },
    "behavior_categories": { "passed": false, "missing": ["drag_drop"],
      "passing": ["sort","scroll","upload","download","filter","paginate","bulk_select","keyboard_nav"] }
  },
  "findings_md_path": ".autospec/test-findings.md",
  "test_run_summary": { "total": 412, "passed": 408, "failed": 4, "duration_ms": 11820300 }
}
```

## 5. Safety rails

### 5a. Mode I (strict isolation — default)
- **Layer A — Pre-flight URL check:** before any test, every URL-shaped value in effective Playwright config (`baseURL`, `use.baseURL`, `webServer.url`, `E2E_BASE_URL`, etc.) matched against `forbidden_url_patterns`. Any match → hard fail, no tests run.
- **Layer B — Runtime network intercept:** Playwright global setup injects `context.route('**/*', handler)` that aborts requests whose final URL (post-redirect) matches a forbidden pattern. Suite aborted on first hit.
- **Layer C — Egress allowlist (optional):** if `egress_allowlist` declared, runner enforces at netns/container level. Skill verifies list presence but trusts runner.
- **Fail-closed:** missing or `[]` `forbidden_url_patterns` (without the explicit ack flag) → refuse to run.

### 5b. Mode II (scoped production — opt-in)

Contract shape:

```yaml
mode: scoped_production
i_understand_this_writes_to_production: true     # required literal

e2e:
  production_scoped_access:
    scope_tokens:
      - kind: row_filter
        table: families
        column: id
        allowed_values: ["test-family-7a3f9c"]
        out_of_scope_action: hard_fail
      - kind: method_allowlist
        module: data_processor
        allowed_methods: ["test_method"]
        others_must_be: read_only
      - kind: route_filter
        methods: [POST, PUT, PATCH, DELETE]
        allowed_path_patterns:
          - "^/api/families/test-family-7a3f9c(/.*)?$"
        action_on_violation: hard_fail

  backup:
    driver: zfs                                  # or: pgdump | mysqldump | custom_cmd
    dataset: "tank/db/prod"
    pre_test_snapshot: true
    verify_snapshot_cmd: "zfs list tank/db/prod@e2e-pre"
    restore_cmd: "zfs rollback tank/db/prod@e2e-pre"
    on_test_catastrophe: restore_and_halt
    refuse_to_run_without_backup: true
```

**Three-layer runtime enforcement:**
1. Pre-suite gate — verify snapshot taken, scope tokens parsed, prod URL reachable, scope identifiers exist. Fail-closed on any miss.
2. Network intercept — every mutating request must match an `allowed_path_patterns` rule AND include the expected scope identifier.
3. Post-test verifier — query DB driver for rows/methods touched in suite window; out-of-scope mutation → `restore_cmd` immediately + halt the entire batch (overrides §6 Mode-I per-PR-block default).

**Hard non-negotiable invariants (encoded in skill code, not user-overridable):**
- No `backup:` → refuse to run
- No verified pre-suite snapshot → refuse to run
- No `restore_cmd` → refuse to run
- Scope violation → restore + halt batch
- 2 consecutive scope violations across runs → auto-quarantine (`mode: scoped_production_quarantined`); manual re-ack required

**One-time ack:** first Mode II activation writes `.autospec/.scoped-prod-acked-<contract-sha>.lock`. Any change to `production_scoped_access` re-requires interactive acknowledgement (or `--ack-scoped-prod-change` for headless).

**Auto-labels:** every Mode II PR run gets `e2e:scoped-prod` + a banner comment summarizing scope tokens + backup driver.

### 5c. Assertion-shift guardrail (covers both unit + E2E test diffs)

After the loop reports green, before auto-merge, the skill runs an AST + regex pass over every modified test file (loop commits + original author commits) and buckets each assertion change:

```
For each modified line in a test file:
  removed assertion                                    → LOOSENING
  changed assertion:
    tolerance widened / check type weakened            → LOOSENING
    tolerance tightened / type strengthened            → STRENGTHENING
    value-only shift, same operator and type           → SHIFTING
  added assertion                                      → STRENGTHENING
```

Per-bucket policy:

| Bucket | Auto-merge | Conditions |
|---|---|---|
| LOOSENING | ❌ | Label `needs-human-review` + `e2e:assertion-loosening`; report comment lists file:line of every loosen |
| SHIFTING | ⚠️ conditional | Auto-merges iff (i) same loop iteration ALSO modified a non-test source file, AND (ii) loop commit message contains `JUSTIFICATION: <text>` for that test file; otherwise blocked + `e2e:unjustified-shift` |
| STRENGTHENING | ✅ | Always allowed |
| No assertion change | ✅ | Loop only added new tests or fixed selectors |

The classifier is pure AST/regex — no LLM judgment in the gate. LLM only writes the `JUSTIFICATION:` line; the gate just checks structure + co-presence.

### 5d. Wizard for first-time setup

`/autospec-test --init` (alias: `npx autospec-test init` when packaged) walks operator through:
1. Mode selection (strict vs scoped; strict is default)
2. If scoped: scope-token kinds, identifiers, prod URL
3. Backup driver detection (probe for `zfs`, `pg_dump`, `mysqldump`, `custom_cmd`); if none and Mode II selected, refuse to write yml
4. Dry-run preview — print the constraints that will apply; require operator to type `I UNDERSTAND` before writing
5. Write `.autospec/test.yml` + initial `.autospec/.scoped-prod-acked-<sha>.lock` if Mode II; operator handles git add + push

**CLI flags (wizard.sh init):**

| Flag | Description |
|---|---|
| `--config <yaml>` | Headless mode: read preset answers from YAML fragment instead of prompting |
| `--ack-i-understand` | Headless acknowledgement flag (replaces interactive `I UNDERSTAND` prompt) |
| `--dry-run` | Print resolved contract preview without writing any files |
| `--output-dir <dir>` | Write `.autospec/test.yml` under this directory (default: `$PWD`) |

**Helper scripts:**
- `wizard-probe-backup.sh` — probes PATH for `zfs`, `pg_dump`, `mysqldump`; prints first detected driver name; exits 1 if none found.
- `wizard-preview.sh <config.yml>` — prints resolved contract YAML + constraint summary to stdout; does not write files.

## 6. Self-heal loop

### Iteration anatomy

```
1. Read gate JSON + findings.md + last 3 iterations' summaries
2. Classify each failure:
     - missing_unit_test
     - missing_test (E2E gap)
     - failing_unit_test
     - failing_test (E2E red)
     - flaky_test       (passes ≥1 of last 3, fails others)
     - selector_brittle (timing / wait / selector resolution)
     - product_bug      (test correct, product wrong)
3. Prioritize:
     product_bug > missing_unit_test > missing_test (E2E)
     > selector_brittle > failing_unit_test > failing_test (E2E) > flaky_test
4. Pick highest-priority cluster that fits remaining budget
5. Edit files. Allowed surfaces:
     - unit test files
     - tests under repo's playwright_test_dir
     - product code (any non-test source file)
     - playwright.config.* (timeouts, retries, projects)
6. Commit with structured message:
     test-heal(iter N): <one-line>
     CLASSIFICATION: <category>
     JUSTIFICATION: <why this change>
     (assertion-shift commits require JUSTIFICATION per §5c)
7. Re-run gate. Green → exit. Red → next iteration.
```

### Budget accounting
- 60-min wall-clock timer for **coding time only** — pauses while tests run (test runtime is unbounded, your stated allowance: hours to days).
- Timer persisted in `.autospec/test-loop-state.json` in worktree so monitor relaunch (per existing Phase 4 silent-exit + relaunch workflow) resumes rather than resets.

### Termination (any one → loop exits)

| Condition | Outcome |
|---|---|
| Gate passes | ✅ Proceed to §5c guardrail |
| 60 min coding time exhausted | 🛑 PR blocked (§6 below) |
| 5 iterations completed | 🛑 PR blocked |
| Same error signature in 3 consecutive iterations | 🛑 PR blocked + `e2e-stuck-error` |
| `~/.autospec/stop.flag` present | 🛑 Graceful exit (reuse autospec-stop convention) |
| Loop classifier produces empty action 2 iterations in a row | 🛑 PR blocked + `e2e-no-action` |

### Loop-state JSON

```json
{
  "pr_number": 1234,
  "started_at": "2026-05-21T12:00:00Z",
  "coding_time_used_seconds": 1842,
  "iterations": [
    { "n": 1, "started_at": "...", "ended_at": "...",
      "classification": "missing_test",
      "files_changed": ["tests/dashboard.spec.ts"],
      "gate_passed": false,
      "error_signature": "sha256:abc..." }
  ],
  "last_error_signature": "sha256:abc...",
  "same_error_consecutive": 2
}
```

**Error signature normalization:** stack frames stripped of line numbers, test names stripped of `[chromium]`-style browser tags, error messages tokenized. Two errors hash equal iff same defect.

## 7. Failure semantics & reporting

### 7a. Per-PR outcomes — Mode I

| Final state | Labels | Auto-merge | Pipeline impact |
|---|---|---|---|
| Gate green, no LOOSENING/unjustified SHIFTING | `e2e:passed` | ✅ | none |
| Gate green, LOOSENING | `e2e:passed`, `needs-human-review`, `e2e:assertion-loosening` | ❌ | monitor moves on |
| Gate green, SHIFTING missing justification | `e2e:passed`, `needs-human-review`, `e2e:unjustified-shift` | ❌ | monitor moves on |
| Gate red, loop healed | `e2e:healed` | ✅ (subject to §5c) | none |
| Gate red, budget exhausted | `e2e:blocked`, `needs-human-review` | ❌ | **monitor proceeds to next issue** |
| Gate red, stuck error | `e2e:blocked`, `e2e:stuck-error`, `needs-human-review` | ❌ | monitor moves on |
| Skill refused (fail-closed) | `e2e:refused`, `e2e:contract-error` | ❌ | monitor moves on |

### 7b. Per-PR outcomes — Mode II (additional)

| State | Labels | Pipeline impact |
|---|---|---|
| Scope violation, restore succeeded | `e2e:scoped-prod`, `e2e:scope-violation`, `e2e:restored`, `needs-human-review` | 🛑 HALT batch (overrides Mode I "move on") |
| Scope violation, restore FAILED | `e2e:scoped-prod`, `e2e:scope-violation`, `e2e:restore-failed`, `CRITICAL` | 🛑 HALT + page operator (`.autospec/.CRITICAL` sentinel; monitor refuses start until removed) |
| 2 consecutive scope violations | `e2e:scoped-prod-quarantined` | Mode II auto-disabled; re-ack required |

### 7c. PR comment (marker-replaced on subsequent runs)

```markdown
<!-- autospec-test-report-marker -->
## autospec-test — ❌ Blocked

**Mode:** strict-isolation
**Clone URL:** https://clone.staging.acme.internal (verified)
**Coding time used:** 58m 12s / 60m   **Iterations:** 5 / 5

### Why blocked
- ui_element_coverage: 7 untouched elements
- behavior_categories: missing drag_drop
- code_coverage: 88.2% lines (threshold 90%)

### Self-heal log
| Iter | Classification | Outcome | Files |
| --- | --- | --- | --- |
| 1 | missing_test (drag_drop) | still red | tests/board.spec.ts |
| 2 | selector_brittle | still red | tests/board.spec.ts, playwright.config.ts |
| ... |

### Untouched UI elements
| Route | Selector |
| --- | --- |
| /dashboard | button[data-testid=export-csv] |

### Findings (LLM, not blocking)
See `.autospec/test-findings.md` (workflow artifact).

### Next steps for human reviewer
1. ...
```

### 7d. Artifacts uploaded
- `playwright-report/`
- `coverage/lcov.info` (unit + E2E merged)
- `.autospec/test-findings.md`
- `.autospec/test-loop-state.json`
- `traces/` (Playwright traces of any failed test)

Comment links to all artifacts.

### 7e. Resume semantics

Loop state JSON in worktree + workflow artifact upload after every iteration lets a relaunched monitor (per known Phase 4 silent-exit + relaunch pattern) resume with same coding-time accumulator, iteration counter, error-signature history.

## 8. Dependencies & scope boundaries

| Dependency | Status | Failure mode |
|---|---|---|
| Skill C — clone provisioner | follow-on spec | clone URL unreachable → `e2e:contract-error`; operator can hand-roll clone in interim |
| Phase 4 existing build/lint gate | live | runs first; failures block before Stage 1 |
| `gh` CLI + GitHub PR labels API | live | required for report + labels |
| Playwright ≥ 1.40 in target repo | per-repo | autodetect; missing → `e2e:contract-error` with bootstrap message |
| Codex CLI (Phase 4 peer-review) | optional | skill independent of peer-review |

### Out-of-scope (explicit non-goals)
- Clone provisioning (Skill C)
- Bootstrapping suites into empty repos (Skill D)
- Cross-PR flake DB
- Multi-browser policy
- CI choice

### Skill family map

```
autospec-test           (Skill A — this spec)            unit + E2E gate + self-heal loop
autospec-e2e-clone      (Skill C — follow-on)            provisions clones, exposes URL
autospec-test-bootstrap (Skill D — future)               bootstraps unit + Playwright suites
```

## 9. Testing the skill itself

### 9a. Unit tests (in autospec repo)
- Assertion-shift classifier — fixture diffs per supported framework (Playwright JS/TS, jest, vitest, pytest, go test) → assert bucket
- Contract loader — fixture `.autospec/test.yml` (valid/invalid/partial/autodetect-only) → assert resolved shape + fail-closed paths
- Forbidden-URL pre-flight check — (config, patterns, expected) table
- Error signature normalization table
- Scope token enforcement — synthetic request streams + tokens
- Loop classifier — synthetic failures → assert category + priority

### 9b. Integration tests — synthetic target repos under `skills/autospec-test/test-targets/`

| Target | Purpose |
|---|---|
| `target-clean-pass/` | minimal app + 100% covered suite → gate passes |
| `target-failing-gap/` | deliberately untouched button + missing drag_drop → loop fills gap |
| `target-greenwash-bait/` | real regression + tempting weakening test → guardrail blocks |
| `target-mode-ii-fixture/` | tiny SQLite app + scope tokens + cp-style backup → scope violation triggers restore + halt |

Each ships `.autospec/test.yml` + golden gate JSON + golden report comment; CI diffs actual vs golden.

### 9c. Lock-step lint
SKILL.md must include `## Self-update`, `## Model tier` (declaring `reasoning:standard, ctx:120k`), and an adapter row in skills inventory. `validate.sh` checks structural sections + presence of `forbidden_url_patterns` example block.

### 9d. Dry-run mode
`/autospec-test --dry-run` runs classifier without editing files; emits proposed iteration plan. Used by unit tests (9a) and operators sanity-checking.

### 9e. Mode II destructive-test guardrail
`target-mode-ii-fixture` uses SQLite + `cp`-based backup driver — exercises Mode II contract surface without ever touching real prod-shape DB.

### 9f. Codex peer-review on PRs to this skill
Existing Phase 4 Codex peer-review applies when skill itself is being modified via autospec-run (dogfooding).

### 9g. Language matrix
`validate.sh` integration matrix covers autodetect for: Node/jest+vitest, Python/pytest, Go/`go test`, Rust/`cargo test`, JVM/JUnit. Tiny synthetic target per language.

## 10. Open follow-ups (separate specs)

1. **Skill C — autospec-e2e-clone:** snapshot/anonymize/scale-down for multi-TB datasets, exposes URL+creds via `.autospec/test.yml`-readable contract.
2. **Skill D — autospec-test-bootstrap:** auto-generate initial unit + Playwright suite for repos that have none.
3. **Cross-PR flake DB:** quarantine known-flaky tests across PRs.

## 11. Decision log (for future readers)

| Q | Decision | Rationale |
|---|---|---|
| Single skill vs split | Split into 3 (A/C/D); this spec is A only | Single SKILL.md would exceed small-LLM context (60-120k target) |
| Integration point | Inline Phase 4 gate (not on-demand) | User priority: quality of code + docs |
| Contract shape | Autodetect + optional `.autospec/test.yml` overrides | Lower friction; safety rails always explicit |
| Coverage metric | A (code cov) + B (UI elements) + D (taxonomy); LLM findings non-blocking | Multi-dimensional catches more gaps; LLM judge too non-deterministic for a merge gate |
| Loop fix scope | Both tests + product code | User accepts the risk with §5c guardrail |
| Budget definition | 60 min coding time; test runtime unbounded | Tests can legitimately run hours to days |
| Loop commits | Land on same Phase 4 PR branch | One PR = one audit trail |
| Assertion-shift policy | LOOSENING blocked / STRENGTHENING allowed / SHIFTING conditional on co-edit + JUSTIFICATION | Peak-detection-improvement case shouldn't be blocked, but raw assertion rewrites should |
| Safety rails | Layer A + B mandatory; C optional; fail-closed | A+B is portable; C requires infra cooperation |
| Mode II opt-in | Allowed with one-time ack + mandatory backup driver + restore-on-violation + batch halt | User explicitly requested for household-style / data-processor-style scoped prod cases |
| Mode I budget exhaust | Block this PR only; monitor moves on | Keeps pipeline flowing |
| Mode II violation | Halt entire batch | Catastrophic class of failure |
| Unit tests | In-scope (Stage 1 of this skill, not separate) | User correction during design |
