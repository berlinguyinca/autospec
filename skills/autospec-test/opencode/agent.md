---
description: Use when you want to gate every Phase 4 PR on unit + E2E test coverage against an isolated environment, auto-heal coverage gaps within a 60-minute coding budget, and block assertion-loosening rewrites before auto-merge.
mode: primary
---

# autospec-test — Unit + E2E Coverage Gate with Self-Heal Loop

Enforce that every PR produced by `/autospec-run` ships with thorough, passing unit + E2E test coverage against an isolated environment, and that any gaps the gate finds get auto-healed by a bounded LLM loop within the same PR.

## Self-update

Decide this purely from the request text the harness handed you. Do NOT
shell out (no `grep`, `sed`, `[[ =~ ]]`, command substitution, etc.) to
test the user's free-form request — passing it through a shell is what
historically tripped harness permission engines. Read the request, normalize
it in your reasoning (collapse whitespace, trim, lowercase), and if the result is
exactly `update`, this skill enters self-update mode and does NOT run the
normal pipeline.

1. Detect harness by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-test/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-test.md`
   - Codex CLI:   `~/.codex/prompts/autospec-test.md`
2. Re-install from `main` by piping the canonical installer:
   ```
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. Show the diff between the prior installed file(s) and the freshly fetched copy.
4. Stop. Do not enter any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-test found; run install.sh first.` and exit.

## Model tier

`reasoning:standard, ctx:120k`

This skill runs inline inside the Phase 4 implementer worktree (same model tier as the outer implementer) or standalone against a specified PR branch. No Tier A dispatch required for normal operation; the coverage gate and self-heal loop are deterministic or LLM-light.

## When to use

- Automatically: `/autospec-run` Phase 4 invokes this skill after the build + lint gate passes and before admin auto-merge, for every PR that closes an `auto-implement` issue.
- Standalone: `/autospec-test [PR#]` for ad-hoc validation of any branch or PR against main — useful before filing an issue or after a manual hot-fix.
- First-time setup: `/autospec-test --init` to interactively configure `.autospec/test.yml` in a target repo.

## When not to use

- When the target repo has no test suite at all (no jest/vitest/mocha/pytest/go test/cargo test found, no Playwright). Use Skill D (suite bootstrapper) first.
- When clone provisioning is not yet in place and no `E2E_BASE_URL` / `PLAYWRIGHT_BASE_URL` / `BASE_URL` env var is set. The gate will refuse to run (exit 2) rather than silently hit production.
- When `~/.autospec/no-test.flag` exists (operator opt-out for repos where testing is handled by an external CI system).
- For repos in the autospec family itself (autospec is a bash/markdown skill repo with no Playwright suite). The skill self-detects and skips gracefully.

## How it works

Two-stage gate runs sequentially inside the Phase 4 worktree:

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

**Stage 1 — Unit (Metric E):** Code coverage thresholds (defaults 95/90/95 lines/branches/functions), function-presence check (every exported/public function has at least one unit test), and all unit tests pass with no `.only`, `.skip`, or `xit`.

**Stage 2A — disciplined authoring (opt-in):** Runs between Stage 1 and Stage 2 only when `e2e.authoring.enabled: true` in `.autospec/test.yml`; otherwise it is skipped with zero overhead. The orchestrator (`stage2a-orchestrator.mjs`) clusters crawler-discovered routes into author-assignable groups (`clusterRoutes` — auto by first path segment, capped at `fanout_max`, or an explicit `route_clusters` override), centralizes shared harness files into `helpers_dir` as a single writer that never clobbers divergent files (`centralizeHelpers`), runs a `playwright test --list` parse gate before the real run (`parseGate`), and writes `e2e/.autospec/coverage.json` as `{total, covered, pct}` from the crawler manifest denominator — not author self-report (`coverageReport`). Each authored spec passes `lint-playwright-author.mjs`, whose `PW_SELECTOR_UNVERIFIED` rule resolves every selector against app source via `selector-evidence.mjs` so invented `data-testid`s hard-fail; failures feed the MAX=5 adaptive-retry loop with corrective directives.

**Spec-compliance QA audit:** For UI/product features, Stage 2 must also use
the `autospec-qa` prompt shape when generating or healing E2E coverage: extract
the spec into a traceability matrix, interact with the running app like a user,
cover text boxes, textareas, selects, dropdowns, checkboxes, radio buttons,
buttons, links, forms, validation, state changes, API effects, accessibility,
responsive layouts, negative paths, and cross-feature flows, then regenerate
tests for every missing or weak behavior. Do not mark a spec requirement PASS
from code inspection alone.

**No-mock deployed smoke:** Mocked API tests are necessary but insufficient for
workflow buttons and dropdowns. For each workflow action, Stage 2 should include
a deterministic mocked happy path plus a no-mock smoke path against the
configured deployed/dev URL when the target contract permits it. The smoke path
must assert no live-failure banner, a visible domain result, request
payload/query changes from selected values, and recovery or actionable UI for
known backend failure signatures such as `502` bridge/proxy outages.

**Self-heal loop:** Up to 5 iterations, 60 minutes of coding time (paused while tests run). Each iteration classifies failures (missing_unit_test, missing_test, failing_unit_test, failing_test, flaky_test, selector_brittle, product_bug) and picks the highest-priority cluster that fits remaining budget. Loop state persisted in `.autospec/test-loop-state.json` in the worktree so monitor relaunches resume rather than reset.

**Assertion-shift guardrail:** AST + regex pass over all modified test files. LOOSENING changes block auto-merge (adds `needs-human-review` + `e2e:assertion-loosening`). SHIFTING is conditionally allowed if the same commit also modifies a non-test source file and includes `JUSTIFICATION:` in the commit message. STRENGTHENING always allowed.

## Spec-compliance QA prompt

`autospec-qa` is the canonical full prompt for spec-to-running-app QA. Do not
duplicate that full prompt here. When Stage 2 or the self-heal loop generates
UI/product E2E tests, import its intent and preserve these minimum anchors:

- **Spec traceability:** every spec requirement maps to expected behavior,
  evidence, and PASS/FAIL/PARTIAL/NOT TESTED status.
- **UI controls:** text inputs, textareas, selects, dropdowns, checkboxes,
  radio buttons, buttons, links, forms, validation, state changes, responsive
  layouts, and accessibility are exercised as user-visible behavior.
- **No-mock deployed smoke:** mocked happy paths are paired with no-mock
  deployed/dev smoke paths whenever the target contract permits it.
- **Outcome assertions:** tests prove visible domain results and absence of
  live-failure banners, not just clickability or request dispatch.
- **User input intent review:** for every input-like control, ask what it is
  for, why it exists, how users should use it, what request/state/result it
  affects, how invalid/rejected input behaves, and whether evidence from the
  running app proves it was implemented and tested correctly.
- **Live backend blocker triage:** before leaving no-mock smoke `NOT TESTED`,
  search safe config surfaces, avoid printing secret values, clean temporary
  credential material, probe health/status/fleet paths when available, and
  classify the blocker as credentials, infrastructure health, route mental
  model, or route-specific worker/handler failure.
- **Proof artifacts:** emit or draft `.autospec/proof-matrix.json`,
  `.autospec/reliability.yml`, `.autospec/control-intent-ledger.json`,
  `.autospec/mutation-proof.json`, and `.autospec/canary-results.json` so spec
  rows connect to implementation files, tests, live evidence, blockers, and
  follow-ups.
- **Reliability contract:** backend-backed features declare required live smoke
  workflows, representative inputs, seed/live data, allowed mocks, required
  no-mock paths, health probes, fallback behavior, forbidden URLs, and forbidden
  false-green patterns.
- **Control intent ledger:** every reachable control is classified as
  functional, read_only, decorative, or unclassified; unclassified controls keep
  the result PARTIAL or FAIL.
- **Duplicate-code guardian:** generated components, helpers, validators, API
  clients, request wrappers, error banners, fixtures, and test utilities are
  checked against existing repo patterns before acceptance.
- **Mutation/breakage proof:** at least one critical workflow per feature proves
  the test fails when request propagation, result rendering, fallback behavior,
  persistence, or backend responses are broken.
- **Post-merge deployed canary:** when a deployed/dev URL exists, representative
  no-mock workflows are defined for post-merge verification and failed canaries
  create regression issues with route classification.
- **New code intent gate:** every new component, module, function, endpoint,
  worker, hook, or test helper has a spec-linked reason, reuse check, public
  contract, proof test, and "what breaks if wrong" answer.
- **Artifact freshness:** proof artifacts record commit SHA, spec hash,
  environment/app URL, test command hash when practical, and generation time;
  stale artifacts cannot support PASS.
- **Evidence provenance:** every PASS cites inspectable evidence such as test
  file/name, command output, screenshot/video/trace/log path, network summary,
  deployed URL, representative input, or backend probe result.
- **Console/network gate:** unexpected frontend exceptions, failed network calls,
  runtime warnings, generic live-failure banners, swallowed errors, or timeouts
  fail the workflow unless they are the explicit negative path under test.
- **Spec contradiction detector:** conflicting, untestable, UI/API-mismatched,
  or data-missing requirements become AMBIGUOUS/PARTIAL with follow-up.
- **Data lifecycle proof:** stateful features prove create, refresh, edit,
  delete, rollback, idempotency, navigation, and relevant concurrency behavior.
- **Observability contract:** backend-backed workflows expose enough safe health,
  timing, correlation, and user-safe error information to triage failed canaries
  without leaking secrets or PII.
- **Flake quarantine:** retries, sleeps, widened timeouts, and reruns are
  classified as product race, selector fragility, backend instability,
  environment issue, or test oracle weakness before acceptance.
- **Reliability exhaustion loop:** repeatedly ask what else could make the app
  unreliable, duplicated, or falsely green; every actionable answer becomes a
  fix, stronger test, proof update, contract update, or follow-up issue.
- **Legacy removal and spec cleanup:** when a spec or accepted architecture
  deprecates a route, cache, bucket, storage backend, worker, feature flag,
  config key, UI path, or data source, generated tests and fixes must prove the
  replacement path and remove stale code/spec/docs references instead of
  repopulating or relying on the deprecated surface.
- **Backend fragility:** `502` bridge/proxy/cache failures require regression
  coverage plus either a documented fallback route or a clear actionable user
  state; mocked endpoint success is not proof of deployed workflow success.
- **Critical self-question:** before accepting green tests, ask: "What could
  still pass here while a real user workflow fails?" Add one test or issue for
  the highest-risk answer.

## Contract file

`.autospec/test.yml` in the target repo. Most fields autodetected; the file's primary purpose is declaring **forbidden URLs** (mandatory) and any Mode II opt-in.

Minimal valid contract:

```yaml
# .autospec/test.yml
mode: strict_isolation

e2e:
  clone_url_env: E2E_BASE_URL
  forbidden_url_patterns:
    - "^https?://app\\.acme\\.com"
    - "^https?://.*\\.prod\\.acme\\.internal"

budgets:
  coding_time_minutes: 60
  max_loop_iterations: 5
  same_error_halt_threshold: 3
```

**Fail-closed:** missing or empty `forbidden_url_patterns` (without `forbidden_url_patterns_intentionally_empty: true`) causes the skill to refuse to run (exit 2, operator-actionable).

**Loop-immutable fields:** the self-heal loop may not edit `.autospec/test.yml`, `.autospec/.scoped-prod-acked-*.lock`, or Playwright config safety-related fields. A pre-commit hook in the worktree enforces this.

Full schema at `schemas/autospec-test-contract.schema.json` in this repo.

## Modes I and II

**Mode I — strict isolation (default):** All test traffic is confined to a clone URL (`E2E_BASE_URL` or equivalent). Three enforcement layers:

1. Pre-flight URL check: every URL-shaped value in Playwright config is matched against `forbidden_url_patterns` before any test runs. Match → hard fail, no tests run.
2. Runtime network intercept: Playwright global setup injects `context.route('**/*', handler)` that aborts requests whose final URL (post-redirect) matches a forbidden pattern.
3. Egress allowlist (optional): if `egress_allowlist` declared, runner enforces at netns/container level.

**Mode II — scoped production (opt-in):** Requires `mode: scoped_production` plus `i_understand_this_writes_to_production: true` in the contract, plus `backup:` with a verified pre-suite snapshot and `restore_cmd`. Scope tokens constrain which rows, methods, and routes the test suite may touch. Scope violations trigger immediate restore and batch halt. Two consecutive scope violations across runs auto-quarantine Mode II and require manual re-acknowledgement.

Hard non-negotiables for Mode II (encoded in skill, not user-overridable): no `backup:` → refuse to run; no verified pre-suite snapshot → refuse to run; no `restore_cmd` → refuse to run; scope violation → restore + halt; 2 consecutive violations → quarantine.

## Safety rails

The skill enforces these rails regardless of operator configuration:

- **Fail-closed forbidden URL check:** no `forbidden_url_patterns` → refuse to run (exit 2).
- **Loop-immutable contract:** self-heal loop cannot weaken safety config files.
- **LOOSENING block:** any test assertion that becomes less strict blocks auto-merge.
- **Mode II backup invariants:** see Modes I and II above.
- **Stop flag respects existing convention:** if `~/.autospec/stop.flag` is present, the skill exits gracefully using the same sentinel path as autospec-stop.
- **Pre-commit hook:** installed in the worktree before the self-heal loop starts; blocks any loop commit that modifies `.autospec/test.yml` or Playwright safety fields.

## Self-heal loop

Loop anatomy for each iteration:

1. Read gate JSON + `.autospec/test-findings.md` + last 3 iterations' summaries.
2. Classify each failure: missing_unit_test, missing_test, failing_unit_test, failing_test, flaky_test, selector_brittle, product_bug.
3. Prioritize: product_bug > missing_unit_test > missing_test > selector_brittle > failing_unit_test > failing_test > flaky_test.
4. Pick the highest-priority cluster that fits remaining coding budget.
5. Edit allowed surfaces: unit test files, Playwright test files, product source code, `playwright.config.*` (timeouts/retries/projects only — not safety fields).
6. Commit with structured message: `test-heal(iter N): <one-line>` plus `CLASSIFICATION:` and `JUSTIFICATION:` lines (required for assertion-shift compliance).
7. Re-run gate. Green → exit. Red → next iteration.

**Termination conditions (any one exits the loop):**
- Gate passes → proceed to assertion-shift guardrail.
- 60 min coding time exhausted → PR blocked (`e2e:blocked`, `needs-human-review`).
- 5 iterations completed → PR blocked.
- Same error signature in 3 consecutive iterations → PR blocked + `e2e:stuck-error`.
- `~/.autospec/stop.flag` present → graceful exit.
- Loop classifier produces empty action 2 iterations in a row → PR blocked + `e2e:no-action`.

Loop state schema:
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

## Wizard

`/autospec-test --init` walks the operator through first-time setup:

1. Mode selection (strict isolation vs scoped production; strict is the default).
2. If scoped: scope-token kinds, identifiers, prod URL.
3. Backup driver detection (probe for `zfs`, `pg_dump`, `mysqldump`, `custom_cmd`). If none detected and Mode II selected, refuse to write `.autospec/test.yml`.
4. Dry-run preview — print the constraints that will apply; require operator to type `I UNDERSTAND` before writing any files.
5. Write `.autospec/test.yml` and, for Mode II, the initial `.autospec/.scoped-prod-acked-<sha>.lock`. Operator handles `git add` and push.

The wizard does not run tests. It only produces config. Run `/autospec-test [PR#]` or let `/autospec-run` invoke the gate to exercise the configuration.

## v2 — Stage 2.5 invariants

Stage 2.5 is an optional extension to Stage 2 that catches a specific regression class: Playwright suites pass green but the user-visible product still breaks — missing edit affordances on visible items, frontend-display windows wider than backend query windows, broken or unaffordable interactive elements, and UI claims the API cannot back.

Stage 2.5 runs four declarative gate metrics (F/G/H/I) after Stage 2 passes, only when `e2e.invariants_v2.enabled: true` in the target's `.autospec/test.yml`. If `enabled` is not `true`, the gate emits `{"skipped": true, "passed": true}` and exits 0 with zero overhead on v1-only targets.

```
Stage 2 (E2E)
     │ passes
     ▼
Stage 2.5 (Invariants & contracts) — skipped if invariants_v2.enabled != true
  Metric F — Structural invariants (edit affordances on all rows)
  Metric G — Window-contract symmetry (UI window == API window)
  Metric H — Extended crawler (all interactive elements affordable)
  Metric I — Data-source contract symmetry (UI claims ↔ API responses)
     │
     ▼
Auto-merge (all pass) or block (any fail)
```

**Stage 2.5 + docs drift-gate composition:** When the docs amendment is active (Phase 10c), the
drift-gate (`check-doc-drift.sh`) runs immediately after Stage 2.5 and shares its exit-code
routing. A `failing_doc_drift` result feeds the same self-heal loop as a Stage 2.5 metric
failure — the loop classifier routes it to `failing_doc_drift` → doc-section edit, not a
test rewrite. Both gates must pass before the fused guardian+LGTM step proceeds. If Stage 2.5
is skipped (`invariants_v2.enabled` absent), the drift gate still runs independently.

## Contract: invariants_v2

Add the `e2e.invariants_v2` namespace to `.autospec/test.yml` to enable Stage 2.5:

```yaml
e2e:
  clone_url_env: E2E_BASE_URL
  forbidden_url_patterns:
    - "^https?://app\\.example\\.com"

  invariants_v2:
    enabled: true
    structural_invariants:
      - name: dashboard-done-items-editable
        selector: "[data-done-item]"
        require_affordance: edit
        routes:
          - /dashboard
    window_contracts:
      - name: dashboard-streak-window
        ui_attribute: data-window-days
        api_param: from
        tolerance_days: 1
    contract_symmetry:
      - name: streak-task-must-be-editable
        ui_selector: "[data-task-id]"
        api_endpoint: /api/timeline
        require_field: editable
    edge_case_seeds:
      enforcement: refuse_to_run_if_missing
      require_shapes:
        - kind: done_item
          count_at_least: 3
        - kind: streak_gap
          count_at_least: 1
```

## Built-in invariant kinds

| Metric | Kind | What it checks | Block label |
|--------|------|----------------|-------------|
| F | `structural` | Every `selector` row has the required `affordance` button/link | `e2e:blocked-stage25` |
| G | `window_contract` | UI window attribute matches API query window (±`tolerance_days`) | `e2e:window-mismatch` |
| H | `crawler` | Every reachable interactive element is afforded (click → response within 5s) | `e2e:unaffordable-elements` |
| I | `contract_symmetry` | UI claim fields present in API response and semantically consistent | `e2e:contract-mismatch` |

## Edge-case seeds handshake

`edge_case_seeds` in the contract creates a hard handshake with Skill C (autospec-e2e-clone). When declared, Stage 2.5 verifies that the clone contains at least `count_at_least` rows matching each declared shape before running any metric.

- `enforcement: refuse_to_run_if_missing` (default) → gate exits 2 (`e2e:seed-missing`) if shapes absent; operator re-runs Skill C.
- Loop cannot fix missing seeds — Skill A explicitly does not edit clone-provisioner concerns.

## Helper library (@autospec/test)

The `@autospec/test` npm package ships Playwright helpers for writing the same invariant patterns imperatively inside target-repo test suites:

```typescript
import { checkStructuralInvariant, verifyWindowContract } from '@autospec/test';

test('dashboard done items are editable', async ({ page }) => {
  await checkStructuralInvariant(page, {
    selector: '[data-done-item]',
    requireAffordance: 'edit',
  });
});
```

Install: `npm install --save-dev @autospec/test` (wizard handles this during `--init`).

## Harness detection (run once at skill start)

Detect your harness by checking available tools before dispatching work:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same dispatch with `TIER_A`. Preserve the parent context on retry; for Codex native subagents, fork/inherit the current conversation context and use the latest top GPT model instead of moving the work into the main session. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run.

## Required capabilities & harness adapter

| Capability                  | Claude Code                             | OpenCode                              | Codex CLI                              | Fallback if missing                         |
|-----------------------------|-----------------------------------------|---------------------------------------|----------------------------------------|---------------------------------------------|
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session               | Run inline (more context cost)              |
| Shell execution             | `Bash` tool                             | shell tool                            | `apply_patch` + shell                  | Required; skill cannot run without shell    |
| Read-only codebase research | `Agent` (subagent_type=Explore)         | `task` agent in read-only mode        | shell `grep` / `rg`                    | In-thread grep                              |
| Ask the user a question     | `AskUserQuestion`                       | inline prompt                         | inline prompt                          | Ask in the response and wait               |
| Subagent model tier         | TIER_B: `sonnet`; TIER_A: `opus` + ultrathink | TIER_B: smaller task model       | TIER_B: `gpt-5.1-codex-spark`          | Fall back UP to TIER_A on unavailability    |
<!-- autospec-block:harness-adapter-core -->

## Stop mode

When the user's request (normalized: collapsed whitespace, trimmed, lowercased) is exactly `stop`, or `stop` followed by one or more `--<word>` flags, this skill enters stop mode and does NOT run the normal pipeline.

Dispatch to `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>`, print the helper's stdout, then stop.

Inside the self-heal loop, the skill checks `~/.autospec/stop.flag` after each iteration. If present, it writes the current loop state to `.autospec/test-loop-state.json`, comments on the PR with the current iteration summary, and exits without blocking or labeling. The next monitor relaunch will resume from the saved state.
