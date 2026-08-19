---
name: autospec-qa
description: Use when the user wants to revalidate a running app against a spec, regenerate missing or weak tests, audit UI controls/forms/validation/dropdowns/API behavior/accessibility, or prove implemented features actually work after autospec-run.
mode: primary
---

# autospec-qa workflow

Run a spec-to-running-app QA audit, then regenerate missing or weak tests until
the application behavior is covered by executable evidence. This skill is the
explicit revalidation companion to `autospec-test`: `autospec-test` gates PRs
with deterministic coverage and E2E checks; `autospec-qa` performs a broad
human-style audit from the spec and turns gaps into stronger tests or follow-up
issues.

## Cluster dispatch

The per-area prose for this skill is also available as 8 cluster files under
`skills/autospec-qa/clusters/<name>.md`, dispatched by
`qa-cluster-dispatch.sh` (issue #730). The 8 canonical clusters are:

1. `spec-traceability` — extract spec requirements + traceability matrix.
2. `functional-coverage` — UI controls, forms, validation, dropdowns, buttons.
3. `backend-integration` — API contracts + no-mock smoke + live backend triage.
4. `reliability-contract` — proof matrix, control ledger, mutation, canary,
   no-mock coverage, observability, data lifecycle.
5. `legacy-and-cleanup` — legacy removal gate + spec/docs cleanup +
   deprecated-surface scan.
6. `benchmark-and-outsourcing` — benchmark-overfit gate + outsourced-
   implementation gate.
7. `accessibility-and-responsive` — a11y + viewport + browser variance.
8. `production-incidents` — incident registry regression check (PR #661).

Each cluster carries only its area's prose plus the spec under audit. The
orchestrator (`qa-cluster-dispatch.sh`) is harness-aware (via
`lib/autospec-harness-detect.sh`, PR #725), honors
`qa-cluster-coverage.sh` for cross-cluster dedup, and honors
`qa-verify-finding.sh` for verify-first filtering. Aggregated findings
land in `.autospec/qa-verdict.json`, which the existing heal loop
(PR #666/#713) consumes unchanged.

The full per-area prose in the sections below remains canonical for now;
cluster files reference it and will be backfilled as the dispatch model
matures. Operators can opt out individual clusters via
`qa-cluster-dispatch.sh --skip-cluster <name>`.

## Deployment contract

`/autospec-qa` assumes the app under audit is already running at the URL the
operator supplies. For repos that need a fresh deploy plus a scoped test-data
clone before each QA cycle, the optional declarative contract
`.autospec/qa-deploy.yml` stands the app up **first** and records what/when/where
into the verdict — so a failed deploy surfaces as a single
`code_health:qa_deploy_*` finding instead of a downstream `502` triage trail.

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/qa-deploy-runner.sh" \
    --repo-dir . --verdict .autospec/qa-verdict.json
```

is invoked at the **very start** of the orchestration — **before any cluster
fan-out** — and its exit code gates the audit. The Live backend blocker triage
prompt above stays as-is: it covers the deploy-succeeds-but-app-unhealthy-mid-
audit case; the deploy runner covers the never-deployed case.

**Contract file `.autospec/qa-deploy.yml`** mirrors the declarative style of
`.autospec/test.yml`, parsed with `yq -o=json` and validated against
`schemas/autospec-qa-deploy.schema.json` with `ajv` (the same yq → jq → ajv
toolchain `skills/autospec-test/scripts/load-contract.sh` uses). Its presence
makes deploy mandatory. Key fields:

- `stages[]` (required) — ordered deploy stages run sequentially under each
  stage's `timeout` (default 300s); first failure aborts the rest. Each stage
  has a `name`, a `command` run via `bash -c`, optional `env`, and an optional
  `health_check` (`url`, `expect_status` default 200, `retry` default 30,
  `retry_interval` default 10s) polled after the command.
- `stages[].safety.max_records` — **required** for any data-clone stage (name or
  command matching `clone|copy|replicate|sync`).
- `teardown[]` — post-verdict cleanup entries (`name`, `command`, `on_failure`
  `warn`|`fail`).
- `target_envs.forbidden` (required, ≥1) — the absolute safety floor.
- `target_envs.allowed` / `safety.notes` — advisory, recorded.

**Safety floor (non-negotiable, enforced regardless of operator config),** each
violation aborts immediately, runs no further stages, and exits **3**:

1. **Forbidden-target match** — each `target_envs.forbidden` token (literal,
   case-insensitive substring) matched against each stage `command`,
   `health_check.url`, and captured stdout → `qa_deploy_forbidden_target`.
2. **Production-pattern rejection** — each deploy/teardown `command` matched
   word-boundary case-insensitively against `--prod`, `--production`, ` prod `,
   ` production `, `production-only`, `live-prod` → `qa_deploy_prod_pattern`.
3. **`max_records` required for data-clone stages** — a `clone|copy|replicate|
   sync` stage without `safety.max_records` → `qa_deploy_missing_records_cap`.

**Exit-code gating:**

| Exit | Meaning | Verdict category |
| --- | --- | --- |
| 0 | No `.autospec/qa-deploy.yml` (no-op), or all stages + probes succeeded | — / `deploy` block |
| 1 | A stage failed/timed out, or a `health_check` exhausted retries | `code_health:qa_deploy_failed` |
| 2 | Usage / contract invalid (ajv fail, malformed YAML, missing required) | `code_health:qa_deploy_invalid_contract` |
| 3 | Safety-floor violation | `qa_deploy_forbidden_target` \| `qa_deploy_prod_pattern` \| `qa_deploy_missing_records_cap` |

A failed deploy (exit 1/2/3) never reaches the cluster fan-out: the operator
sees a single `code_health:qa_deploy_*` finding naming the failing stage and a
stdout tail, not a `502`.

**`deploy:` verdict block.** On success the runner writes a `deploy:` block into
`.autospec/qa-verdict.json` (atomically via the sibling `.tmp` + `mv` pattern
from `qa-finding-filter.sh`; a minimal `{"deploy": {...}}` is created
when no verdict exists yet, which the later audit merges into):

```json
{
  "deploy": {
    "stages_run": ["deploy-to-test-family", "clone-production-data"],
    "stage_durations_ms": {"deploy-to-test-family": 145000},
    "target_env": "https://test.tidyboard.org",
    "head_sha_deployed": "abc1234",
    "data_clone": {"source": "prod-readonly", "records_copied": 8500, "max_records": 10000},
    "teardown_run": true,
    "teardown_failures": []
  }
}
```

`head_sha_deployed` = `git rev-parse --short HEAD` at deploy time.
`data_clone.records_copied` is parsed from a `records_copied=<int>` stdout line
when present, else omitted. The block is **additive** — every existing verdict
consumer (heal loop, `/autospec-release`, `/autospec-sweep`) is unaffected.

**Teardown.** `teardown[]` entries run after the verdict is final, unless
`--skip-teardown` is passed (which sets `deploy.teardown_run: false` and leaves
the env intact for repeated manual re-QA). `on_failure: warn` logs only;
`on_failure: fail` downgrades a `PASS` verdict to `PARTIAL` (never touches
`FAIL`) and appends the entry to `deploy.teardown_failures`.

**Graceful absent-file behavior.** When `.autospec/qa-deploy.yml` is absent the
runner exits `0` immediately, writes nothing, and QA proceeds exactly as today —
the verdict is byte-identical with no `deploy:` key. This compatibility contract
is the most important one: behavior is byte-for-byte today's behavior when the
file is not present. Secret env **values** are never logged or recorded (only
key names). The runner uses `set -u` (not `set -e`) around stage execution so
failures are caught explicitly and recorded in the verdict.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-qa -->

## Self-update mode

If the feature-request argument matches `update` after trimming and lowercasing,
re-install the full autospec suite from `main`, show the before/after diff if the
harness exposes it, then stop. Do not run the QA audit.

## Required capabilities & harness adapter

| Capability | Claude Code | OpenCode | Codex CLI | Fallback if missing |
| --- | --- | --- | --- | --- |
| Subagent model tier | Tier A: `opus` + ultrathink | Tier A: top-tier `task` + max reasoning | Tier A: current top GPT + `reasoning_effort=high` | Run inline, but keep the same report contract |
<!-- autospec-block:harness-adapter-core -->
| Browser/E2E execution | Playwright/browser tool or shell | Playwright/browser task | shell + browser when available | Use `browser-verified` / `fallback-smoke-only` / `not-run`; mark UI-only gaps with blocker |
| Shell execution | Bash tool | shell tool | shell/apply_patch | Required for regeneration |

**Model tier:** TIER_A for the spec audit and test-regeneration plan because
false PASS results are more expensive than extra review tokens.

## Harness detection

Detect the harness once at skill start:

1. Claude Code: `Agent` with `subagent_type` is available.
   - `TIER_A` = `opus` + ultrathink.
   - `TIER_B` = `sonnet`.
2. OpenCode: `task` tool is available.
   - `TIER_A` = top-tier task model + high reasoning.
   - `TIER_B` = smaller-tier task model + medium reasoning.
3. Codex CLI: `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT + `reasoning_effort=high`.
   - `TIER_B` = current cost-optimized Codex model + `reasoning_effort=medium`.

Prefer a Tier A reviewer/QA subagent for the first traceability audit when the
harness supports it. If `TIER_A` is unavailable, silently fall back to the next
available top-tier model. If delegation is unavailable, run the audit inline.

## When to use

- After `/autospec-run` finishes and the operator wants proof that the app
  actually satisfies the source spec.
- When a UI-heavy feature needs text boxes, validation, selects, dropdowns,
  buttons, navigation, modals, keyboard behavior, accessibility, and API effects
  verified from multiple directions.
- When existing tests pass but the spec may still be under-tested.
- When the user asks to regenerate tests from a spec or strengthen weak tests.

## When not to use

- Do not use against production unless the target repo's `autospec-test`
  contract explicitly allows scoped production and backup/restore is verified.
- Do not mark a feature PASS from code inspection alone.
- Do not loosen assertions to make regenerated tests pass.
- Do not treat mocked API tests as proof that a deployed workflow works.

## No-mock deployed smoke rule

For every user workflow button or dropdown-backed action, QA must cover both the
contract and the deployed user-visible result:

1. Verify the control is enabled and changing it changes the actual request
   payload, query, route, or command parameters.
2. Run a mocked happy-path test for deterministic UI state coverage.
3. Run a no-mock smoke test against the configured deployed/dev URL using a
   representative input from the spec or domain fixtures.
4. Assert that no generic live-failure banner appears.
5. Assert that the result panel contains a real domain result, not only that a
   request was sent.
6. Include regression coverage for known fragile backend responses, such as
   `502` bridge/proxy failures, and require either a working fallback route or a
   clear actionable user state.
7. Re-check the endpoint mental model against the app and production behavior:
   if a mocked route differs from the route that actually supports the workflow,
   tests must follow the user-visible workflow and document the stable backend
   contract they prove.

## No-mock deployed smoke blocker handling

If a required no-mock deployed/dev smoke path is `NOT TESTED`, the final verdict
must be `PARTIAL` unless the target repo's explicit test contract says that
workflow intentionally has no deployed smoke requirement.

Do not leave a no-mock smoke gap as a prose-only "remaining risk." For every
`NOT TESTED` no-mock smoke row, create or draft one of:

1. An environment setup issue/task that names the missing dev URL or env var,
   representative seed/live data, credential requirements, backup/restore
   constraints, and exact smoke workflow command or Playwright scenario.
2. A test contract update in `.autospec/test.yml`, `docs/**`, or the repo's
   equivalent QA contract explaining why deployed smoke is intentionally
   unavailable for that workflow.
3. An operator-blocker line with the exact missing configuration and the next
   command to rerun once it exists.

If a dev URL and representative data are supplied, run the no-mock smoke test
instead of drafting setup work. If credentials or data are missing, keep the
traceability row `NOT TESTED`, keep the overall result `PARTIAL`, and make the
setup artifact the required next action.

## Presentational control classification

Example chips, demo shortcuts, taxonomy tokens, conversion presets, and similar
controls must be classified before the report is finalized:

- `decorative/presentational`: assert that the element has no action contract,
  no request side effect, and is either documented or clearly inert.
- `workflow control`: verify request contract, mocked happy path, no-mock smoke
  path, success-state result, failure-banner absence, and known-fragile-backend
  fallback behavior.

Do not report presentational chips as a vague risk. Either prove they are
intentionally decorative or file/draft the issue that wires and tests them as
real controls.

## User input element intent prompt

Use this prompt every time the app creates, changes, tests, or audits any
user-input element, including text boxes, textareas, selects, dropdowns,
comboboxes, checkboxes, radio buttons, switches, date/time inputs, file inputs,
search boxes, filters, sliders, segmented controls, editable tables, chips, or
other controls that accept or change user intent:

```text
For this user input element, answer before accepting the implementation or test:

1. What is this control for?
2. Which user goal or spec requirement does it serve?
3. How should a real user discover and use it?
4. What valid, invalid, empty, boundary, and recovery inputs must work?
5. What request payload, query, route, command, state change, persistence effect,
   or visible result should change when the user changes this input?
6. What should happen when the backend rejects, times out, or partially accepts
   this input?
7. Is this control intentionally read-only, disabled, decorative, or
   presentational? If yes, where is that contract documented?
8. Did I implement and test this correctly, with evidence from the running app
   rather than only DOM presence or mocked request dispatch?

Required outcome:
- If the control is functional, tests must cover discoverability, enabled/disabled
  state, valid input, invalid input, boundary input, request/output effect,
  success state, error state, and recovery.
- If the control is decorative or read-only, the QA report must classify it that
  way and cite the spec, UI copy, or test contract that makes this intentional.
- If the answers are unclear, mark the traceability row AMBIGUOUS or PARTIAL and
  draft the smallest spec/test/implementation follow-up needed to settle it.
```

## Live backend blocker triage prompt

When a no-mock deployed/dev smoke path is blocked by missing config, unknown
credentials, `502`, `504`, timeout, bridge, queue, worker, proxy, or backend
health errors, do not stop at "credentials unavailable" or "live backend not
tested" until you have made a bounded, non-secret-leaking config search.

```text
For this live backend blocker, answer before finalizing the QA report:

1. Where does the app define its deployed/dev base URL, API endpoint, worker
   endpoint, queue/bridge URL, and environment-specific service names?
2. Are these values available through safe config surfaces such as Terraform
   outputs, deployment manifests, CI variables, documented env vars, cloud
   resource names, or local `.env.example` files?
3. If credentials are required, which secret names or credential sources are
   referenced, and can the smoke test use them without printing secret values?
4. If temporary credential files or tokens are created for probing, were they
   stored outside the repo, restricted, and removed after the probe?
5. Is the failure caused by missing credentials, stopped infrastructure,
   unhealthy bridge/proxy, missing worker, wrong route mental model, or a live
   handler/index path that times out?
6. Can a direct health/status/fleet/ping probe distinguish infrastructure
   outage from a route-specific worker failure?
7. Which no-mock smoke paths now pass with real domain data, and which exact
   route/subject still fails?
8. What operator action, infrastructure fix, worker fix, or fallback behavior is
   required next?

Required outcome:
- Never print secret values in the QA report, logs, issue body, or test output.
- If credentials/config are discoverable, run the no-mock smoke instead of
  leaving it `NOT TESTED`.
- If infrastructure was stopped or unhealthy and can be safely started by the
  target repo's documented dev procedure, record the action and rerun the smoke.
- If broad health probes pass but one route/subject times out, classify the
  blocker as a route-specific live worker/handler failure, not missing
  credentials.
- Report passed live paths and the remaining failing path separately.
```

## Proof artifact requirements

The QA pass should produce machine-readable proof artifacts whenever the target
repo permits writing under `.autospec/`. If the repo is read-only, draft the
artifact contents in the report.

Required artifacts:

- `.autospec/proof-matrix.json`: one row per spec requirement with fields for
  `requirement_id`, `spec_reference`, `expected_behavior`,
  `implementation_files`, `automated_tests`, `live_evidence`, `status`,
  `blocker`, and `follow_up`.
- `.autospec/reliability.yml`: the generated app reliability contract,
  including required live smoke workflows, representative inputs, seed/live data
  sources, allowed mocks, required no-mock paths, health probes, expected
  fallback behavior, forbidden production URLs, and forbidden false-green
  conditions such as "PASS from DOM presence only."
- `.autospec/control-intent-ledger.json`: every discovered user input, button,
  link, chip, dropdown, form, modal trigger, table action, and workflow control
  classified as `functional`, `read_only`, `decorative`, or `unclassified`, with
  intent, owning spec reference, selector/locator, expected effect, tests, and
  evidence.
- `.autospec/mutation-proof.json`: critical workflow assertions that were
  intentionally broken or simulated and proved that the test suite fails when
  the implementation regresses.
- `.autospec/canary-results.json`: post-merge or deployed/dev no-mock canary
  results for representative workflows when a deployed/dev environment exists.

Validate written artifacts against the matching repository schemas when tooling
is available:

- `schemas/autospec-proof-matrix.schema.json`
- `schemas/autospec-reliability.schema.json`
- `schemas/autospec-control-intent-ledger.schema.json`
- `schemas/autospec-mutation-proof.schema.json`
- `schemas/autospec-canary-results.schema.json`

Use `validate-qa-artifacts.sh --repo <target-repo>` when the autospec
repository helper is available. It validates present `.autospec/*` proof files
and fails malformed artifacts while warning about missing draft-only artifacts.

Hard gate: a QA result cannot be `PASS` if `.autospec/proof-matrix.json` has a
`FAIL`, `PARTIAL`, `NOT_TESTED`, `AMBIGUOUS`, or unblocked `unclassified` row.

## Artifact freshness gate

Every proof artifact must carry freshness metadata:

- repository commit SHA,
- spec file path,
- spec hash,
- app URL or environment,
- test command hash when practical,
- generation timestamp.

Before accepting an artifact, compare its freshness metadata to the current
checkout and tested environment. If the spec, implementation, app URL, or test
command changed after the artifact was generated, mark the artifact stale and
the affected rows `PARTIAL` until regenerated.

## Documentation gate

Every QA run invokes `/autospec-doc --full` (via
`skills/autospec-doc/scripts/doc-orchestrator.mjs --full`), then runs
`verify-examples.mjs` across the regenerated pages, then runs
`check-doc-drift.sh --working-tree` expecting a clean working tree.

`documentation.auto_regenerate` is NOT consulted here. QA is the guaranteed
sync point for documentation correctness, independent of the operator-controlled
auto-regeneration switch.

Each of the following failure classes is a **QA finding** in the standard QA
report format, subject to the same severity discipline as the no-mock smoke rule
(never silently skipped):

- **Generation error** — `doc-orchestrator.mjs --full` exits non-zero or
  produces no output pages.
- **Failing example** — `verify-examples.mjs` reports one or more example
  failures across the regenerated audience pages.
- **Residual drift** — `check-doc-drift.sh --working-tree` exits non-zero,
  indicating that generated pages differ from what is committed.
- **Missing audience page** — a required audience page is absent from the
  output after generation.

Failures are reported as findings with `release_blocking: true` and `category:
documentation_gate`, mirroring the no-mock smoke handling.

**Graceful skip** (logged as INFO, not a finding): when the target repo has no
`documentation:` config (orchestrator exits 2), the gate records an INFO log
line — `docs: no documentation config found (exit 2), gate skipped` — and
continues without emitting a finding. No other skip condition is permitted.

## Repo quality audit

After QA proof artifacts are written, run the shared read-only quality audit so
the QA report carries repo-wide code-quality, verification, dependency, route,
security, and maintainability gaps discovered during revalidation:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-quality-audit.sh" \
  --repo . \
  --json ".autospec/repo-quality-audit.json" \
  --markdown ".autospec/repo-quality-audit.md" \
  ${AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES:+--file-issues}
```

Include the audit `status`, filed issue links, suppressed findings, and unfiled
residual risks in the final QA verdict. The helper is read-only with respect to
source files; it only writes audit artifacts and optional deduplicated GitHub
issues when operator policy permits issue creation.

## Evidence provenance gate

Every `PASS` must cite provenance that another reviewer can inspect:

- test file and test name,
- command and result,
- screenshot, video, trace, or log path when relevant,
- network request/response summary,
- deployed/dev URL and representative input for no-mock smoke,
- backend health/probe result when backend behavior is claimed.

No provenance means no `PASS`. Request dispatch alone, DOM presence alone, a
generic result panel change, or a mocked-only response is not sufficient
provenance for backend-backed behavior.

## Console and network error gate

For each browser or deployed/dev workflow, inspect console and network signals.
The workflow fails unless the error is the explicit negative path under test:

- uncaught frontend exceptions,
- failed network calls,
- hydration/runtime warnings that affect the route,
- generic live-failure banners,
- unexpected retries, timeouts, or aborted requests,
- swallowed errors that leave stale UI.

Record expected negative-path failures separately from unexpected errors.

## Spec contradiction detector

Before implementation, test generation, or final QA, scan the spec and current
UI/API behavior for contradictions:

- two requirements conflict,
- an acceptance criterion is not testable,
- UI copy implies behavior the spec does not mention,
- backend/API contract contradicts product wording,
- error/fallback behavior is unspecified for a required workflow,
- required seed/live data is missing for a no-mock path.

Contradictions become `AMBIGUOUS` rows with a proposed resolution and a
spec/config follow-up before implementation is called complete.

## Spec supersession (recency)

When two specs in `docs/specs/` overlap on a behavior, the spec whose
last-modifying commit on the current branch is most recent wins (issue #635:
implicit-by-recency supersession; untracked specs fall back to filesystem
mtime). Operators do NOT write `Supersedes:` frontmatter — recency alone
decides.

QA validation MUST resolve each behavior to its authoritative spec before
classifying a row as `FAIL`. Behavior present only in an older, superseded
spec is not a QA failure — it has been replaced by the newer spec. Use the
shared helper to look up the authoritative spec for each behavior key:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/resolve-spec-supersession.sh" "<behavior-key>"
```

For each proof-matrix row whose expected behavior overlaps multiple specs:

1. Run the resolver against the behavior key (e.g. heading text or
   acceptance-criterion fragment).
2. If the resolver returns a spec path different from the row's
   `spec_reference`, the row is being validated against a superseded
   behavior. Re-derive the expected behavior from the resolver's winning
   spec and update the row's `spec_reference` accordingly.
3. If the row's expected behavior no longer appears in the winning spec,
   mark the row `superseded` with a note pointing at the winning spec; do
   NOT classify it `FAIL`.

This rule applies symmetrically to control-intent ledger entries and
live-evidence checks: a control whose intent is only described by a
superseded spec is not a regression.

## Data lifecycle proof

For CRUD, stateful, or workflow apps, prove the data lifecycle:

- create persists after refresh,
- edit propagates to list/detail/search/export views,
- delete/remove is reflected everywhere without ghost UI state,
- failed writes roll back optimistic UI,
- retries are idempotent or visibly safe,
- browser back/forward and deep links preserve expected state,
- concurrent or rapid repeated actions do not corrupt state.

If the app has persistence but lacks lifecycle evidence, keep the affected
feature `PARTIAL`.

## Observability contract

Generated apps must expose enough safe diagnostics to debug failed QA and
canaries:

- request correlation IDs across frontend/backend where practical,
- frontend error boundary logging,
- backend route/worker timing,
- health/status endpoints or probes,
- user-safe actionable error messages,
- logs that identify route/worker/cache/database class without exposing
  secrets or PII.

Missing observability does not automatically fail a pure UI feature, but it is a
blocking gap for backend-backed workflows whose failures cannot otherwise be
triaged.

## Flake quarantine rule

Do not accept "green after retry" as reliable without classification. If a test
uses retries, sleeps, widened timeouts, or repeated reruns to pass, classify the
flake as one of:

- product race,
- selector fragility,
- backend instability,
- environment issue,
- test oracle weakness.

Then either fix it, quarantine it with a follow-up issue and owner, or keep the
affected proof row `PARTIAL`.

## Additional reliability surfaces

Keep looking for reliability gaps beyond the obvious UI happy path:

- route/screen inventory: every spec screen and reachable route has coverage or
  a documented exclusion,
- API contract replay: frontend requests match OpenAPI/schema/server contracts
  when available,
- auth and permission matrix: each role can and cannot perform the expected
  actions,
- timezone, locale, browser, viewport, and device variance for date/format/UI
  sensitive features,
- performance budgets for primary workflows,
- destructive action safety: confirmation, undo/restore, scoped production
  protections, and backup/restore where relevant,
- dead-code and generated-artifact cleanup after implementation,
- dependency/security scan for newly added packages or SDK use,
- migration/backward-compatibility proof for schema or persisted-data changes,
- test oracle specificity: assertions name the domain result, not generic text
  or "something changed."

## Reliability exhaustion loop

Before finalizing, ask: "What else could make this generated app unreliable,
duplicated, or falsely green against the spec?"

Repeat until the next answer is only:

- already covered by a proof artifact, test, canary, or follow-up issue,
- impossible to verify from available safe environments and recorded as an
  operator blocker,
- outside the accepted scope and filed as a separate spec issue,
- lower value than the documented risk tolerance in `.autospec/reliability.yml`.

Each actionable answer must become a strengthened test, implementation fix,
proof-artifact update, reliability-contract update, or follow-up issue.

## Generated app reliability contract prompt

Before declaring a generated app reliable, create or validate
`.autospec/reliability.yml`:

```text
For this app, define the reliability contract:

1. Which user workflows are required live smokes?
2. Which representative inputs and seed/live data prove each workflow?
3. Which tests may mock APIs, and which workflows require no-mock deployed/dev
   execution?
4. Which health probes distinguish app, API, queue, bridge, worker, cache,
   database, and external-service failures?
5. Which backend failures require fallback behavior, and what visible state
   proves the fallback worked?
6. Which production or unsafe URLs are forbidden during tests?
7. Which false-green patterns are forbidden, such as DOM-only PASS, request-only
   PASS, mocked-only PASS, or screenshot-only PASS?

Required outcome:
- If the contract is missing, draft it.
- If the contract conflicts with the spec, mark the affected rows PARTIAL and
  file a spec/config follow-up.
- If no deployed/dev environment exists, require an explicit contract entry
  explaining the blocker and exact setup action.
```

## Control intent ledger prompt

The UI audit must not rely on a vague list of "controls covered." Build a
control intent ledger:

```text
For every reachable control, record:

- selector or stable locator
- visible label and accessible name
- control type
- classification: functional, read_only, decorative, or unclassified
- spec requirement served
- expected request/state/persistence/result effect
- validation and error behavior
- tests proving the effect
- live no-mock evidence when required
- follow-up issue or blocker if not fully proven

Required outcome:
- Functional controls need request/output/result/failure coverage.
- Read-only and decorative controls need an explicit documented reason.
- Any unclassified control makes the QA result PARTIAL or FAIL.
```

## Duplicate-code guardian prompt

Before accepting generated code or regenerated tests, challenge duplication:

```text
For each new component, function, module, API client, validator, form helper,
error handler, backend wrapper, fixture, or test utility:

1. Search the repo for existing helpers or patterns that already solve this.
2. If a duplicate exists, reuse it or explain why it is insufficient.
3. If the new code remains, identify its owning spec requirement and public
   contract.
4. Block duplicate API clients, duplicate validators, duplicate form state
   logic, duplicate error banners, duplicate request wrappers, and duplicated
   test fixtures unless the issue/spec explicitly requires a fork.

Required outcome:
- Duplication findings become implementation fixes or `DUPLICATE_CODE`
  follow-up issues with file references.
```

## Mutation and breakage proof prompt

For every critical user workflow, prove that the test would fail if the product
were broken:

```text
For this workflow test, simulate or reason through one minimal break:

1. Break request payload/query propagation.
2. Break the visible result rendering.
3. Return a known backend failure such as `502`, `504`, timeout, empty data, or
   validation rejection.
4. Disable the fallback route or error banner.
5. Remove persistence or state update.

Required outcome:
- At least one critical workflow per feature has mutation/breakage evidence.
- If a test still passes after the behavior is broken, strengthen the assertion
  before accepting the QA result.
- Record the result in `.autospec/mutation-proof.json` or the QA report.
```

## Benchmark leak and overfit gate

For every test that consumes a public dataset, benchmark, leaderboard, or
externally-curated answer key (e.g., ClassyFire, ChEMBL, MassBank, MNIST,
GLUE, SuperGLUE, ImageNet, BIG-bench, HELM, GSM8K, MMLU, fixtures derived
from a named corpus), audit for benchmark leak and overfit-shaped assertions:

```text
For each test fixture and assertion derived from a public benchmark or
externally-curated dataset:

1. Identifier hard-coding — does the test pin specific benchmark IDs
   (InChIKeys, accession numbers, row hashes, sample indices, image filenames,
   prompt IDs) and assert exact downstream values?
2. First-N slice — is the fixture the first N rows / top-N records / a fixed
   prefix of a known benchmark, rather than a randomized or rotated sample?
3. Answer-key shape — do the assertions match the benchmark's published
   answer key (labels, class names, expected outputs) such that the system
   under test could pass by memorizing those rows?
4. Distributional sibling — is there a randomized or property-based test
   covering the same code path on a fresh draw from the source distribution?
5. Refresh path — can the fixture be regenerated on demand from the upstream
   source (API, query, seeded sampler), or is it frozen indefinitely?
6. Training-on-test — is the same fixture also used to train, tune, select,
   or prompt-engineer the code under test? If yes, the fixture is leaked.

Required outcome:
- Replace fixed slices with a seeded random sample drawn from the upstream
  source per run (or per release), and rotate the seed at a defined cadence.
- Replace exact-value assertions on benchmark identifiers with property-based
  or invariant assertions that hold for any valid draw from the distribution.
- Keep at most one tiny "shape smoke" test with hard-coded IDs, clearly named
  as a smoke test, never as the primary correctness signal.
- A test that asserts exact benchmark answers without a randomized sibling is
  a PARTIAL finding. If the same fixture is also used as the training or
  selection signal for the code under test, it is FAIL (training-on-test
  leak).
- Record each finding in the QA report under "Benchmark Overfit and Test
  Leak" with the test path, the leaked dataset, the failure mode (1-6
  above), and the required remediation.
```

## Outsourced implementation gate

When the spec uses an implementation verb (classify, resolve, score, parse,
transform, extract, generate, rank, detect, segment, embed, …) and the diff
or repo adds a runtime call to a third-party API/SDK that performs that same
verb, the LLM has outsourced the product to an external service instead of
implementing it. Audit every new or expanded external-service binding:

```text
For each new HTTP client, SDK call, or service binding introduced to satisfy
a spec acceptance criterion:

1. Verb alignment — does the spec's verb describe an action the product is
   supposed to PERFORM (build, classify, resolve, score, parse, transform,
   extract, generate, detect, segment, embed) or merely INTEGRATE WITH
   (fetch from, sync, import, mirror, publish to)? Implementation verbs
   satisfied by remote-call shortcuts are outsourced implementations.
2. Benchmark / comparator collision — does the third-party service's name
   appear elsewhere in `docs/specs/**`, README, or QA reports as a
   "benchmark", "comparator", "ground truth", or "baseline"? Using the
   benchmark at runtime contaminates evaluation and frequently violates
   ToS.
3. Missing local algorithm — does the spec or referenced paper describe an
   algorithm (rules, features, model, scoring formula) that the repo does
   NOT implement, while a remote call answers the same question?
4. Cached external payloads — is there a `cache/`, `fixtures/`, or
   `generated/` directory holding downloaded service responses that the
   runtime depends on, with no in-repo computation able to reproduce them?
   (Does NOT apply when the spec's verb is itself an integration verb
   from signal 1's integration list — e.g., a project whose stated job is
   to mirror/import/sync that external dataset.)
5. License and provenance — is there a provenance record (URL, fetch
   timestamp, ToS link, redistribution scope) for every cached external
   payload? Missing provenance on cached third-party data is a release
   blocker even when the runtime call is itself legitimate.
6. Reimplementation directive — does the spec or product mission state
   that the project is supposed to REPLACE or REIMPLEMENT the named
   service? If yes, runtime use of that service is a definitional bug.

Required outcome:
- For each finding, name the spec acceptance criterion, the offending
  client/URL/SDK call, and the signals (1-6) that matched.
- Remediation: replace the remote call with an in-repo implementation that
  matches the spec's described algorithm. The external service MUST be
  either (a) removed entirely, or (b) gated behind an off-by-default
  feature flag scoped to evaluation/benchmark comparison only, with cached
  payloads carrying explicit redistribution permission. No other
  configuration is acceptable for release PASS.
- A finding matching signal 1 (implementation verb satisfied by remote
  call), signal 2 (benchmark/comparator collision), or signal 6
  (reimplementation directive) is FAIL — these are product-identity
  violations and cannot be release-deferred.
- Signals 3, 4, or 5 alone default to PARTIAL until the in-repo
  implementation lands. Signal 3 paired with signal 1 escalates to FAIL.
- Record each finding in the QA report under "Outsourced Implementation"
  and in `.autospec/qa-verdict.json` with category
  `outsourced_implementation`, the matched signal numbers in the summary,
  and the remediation status.
```

## Exhaustiveness flags

The orchestrator soft-discipline sections above (the reliability exhaustion
loop, the 20-question Critical self-questioning checkpoint, "no-mock minimum
coverage", etc.) drive depth of coverage by prose. When clusters skip a step
the report can still say PASS while a production bug ships. The operator MAY
invoke `/autospec-qa` with one or more explicit machine-checkable flags that
convert that prose into a hard gate. With no flag, behavior is unchanged.

- `--every-spec-row` — fail the run with `code_health:incomplete_traceability`
  if any spec acceptance criterion lacks a non-`NOT_TESTED` row in
  `.autospec/proof-matrix.json`. Forces the orchestrator to extract every
  AC, not just the ones it judged "important".
- `--mutation-each` — require a mutation/breakage entry in
  `.autospec/mutation-proof.json` for every workflow listed in the spec,
  not just "critical" ones. Missing entries emit
  `code_health:incomplete_mutation_proof`.
- `--no-mock-each` — require a no-mock deployed/dev smoke path in
  `.autospec/canary-results.json` for every backend-backed feature.
  Exceptions must be declared explicitly under `no_mock_exempt` in
  `.autospec/reliability.yml`. Missing entries emit
  `code_health:incomplete_no_mock_coverage`.
- `--exhaustive` — turns the three above on.

Enforcement runs after the existing verdict computation but BEFORE writing
`.autospec/qa-verdict.json` as final. The helper appends one
`release_blocking: true` finding per gap and demotes the verdict from
PASS to PARTIAL (FAIL is preserved). The list of flags actually requested
is recorded under `requested_flags` on the verdict so downstream skills
(notably `/autospec-release`) can see what was enforced. Both the
orchestrator prompt and the regeneration loop call this helper:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/qa-exhaustiveness-check.sh" \
    --exhaustive \
    --verdict        .autospec/qa-verdict.json \
    --proof-matrix   .autospec/proof-matrix.json \
    --mutation-proof .autospec/mutation-proof.json \
    --canary         .autospec/canary-results.json \
    --reliability    .autospec/reliability.yml \
    --spec docs/specs/<feature>.md
```

Exit 0 means every requested gate passed. Exit 1 means at least one gate
failed and the verdict + findings have been updated atomically. Backwards-
compatible: invoking with no flag is a no-op.

## Self-healing loop

`/autospec-qa` is self-healing by default. After discovery, the orchestrator
enters a discover → file → fix → re-QA loop and runs until convergence or a
guardrail trips. The loop is driven by `qa-heal-loop.sh`, which calls
`qa-finding-to-issue.sh` once per release-blocking finding to file an
`auto-implement,autospec:v2-flow` GitHub issue, then dispatches `/autospec-run`
to drain the queue before re-running QA.

Builds on the verify-first filter (#647 / PR #650): the heal loop relies on
the filter to reject unverified findings before they become issues, otherwise
round 1 would file noise. Builds on the production-incident registry
(#659 / PR #661): findings tagged
`code_health:production_incident_regression_missing` follow the same heal
path. Companion to autonomous mode (#662): when
`~/.autospec/autonomous.flag` is set, heal mode is implicit.

Loop semantics:

1. Round N: run `/autospec-qa`, write `.autospec/qa-verdict.json` as today.
2. For each finding with `release_blocking: true` and
   `status ∈ {FAIL, PARTIAL, NOT_TESTED}`:
   - Translate via `qa-finding-to-issue.sh` into a structured issue
     body (category + summary + evidence + reproduction + remediation
     directive verbatim from the relevant `code_health:*` directive map).
   - File one `auto-implement,autospec:v2-flow` issue. Existing dedup:
     skip if an open issue with the same `(category, file, summary_hash)`
     already exists.
3. Launch `/autospec-run` (single-agent Phase 4) to drain the new + existing
   queue. One PR per issue per existing pipeline semantics.
4. After the run drains (`batch-done.json: ALL_DONE`), re-enter step 1.

Termination — loop exits when ANY of:

- **Convergence** — round N produces zero findings with
  `release_blocking: true`. (Non-blocking findings are filed once but do
  NOT keep the loop alive.) This is the canonical success state.
- **Oscillation detected** — round N+1's release-blocking finding set,
  hashed by `(category, file, summary)`, equals round N's. The auto-fix
  is going nowhere; the loop exits with `oscillation_detected` and
  surfaces the persistent findings for operator review.
- **Round cap** — `AUTOSPEC_HEAL_MAX_ROUNDS` reached (default 5).
- **Budget cap** — tokens used > `AUTOSPEC_HEAL_TOKEN_CAP` (default
  1.5M) or wall time > `AUTOSPEC_HEAL_TIME_CAP` (default 4h).
- **Verify-first violation persists** — if the post-cluster verify-first
  filter keeps dropping the same findings as `verified_dropped` round
  after round, those findings are likely flaky probes; the loop exits
  and asks the operator to triage.

At loop end, the orchestrator prints a Markdown table to the operator and
writes `.autospec/heal-summary.md` for the audit trail and for
`/autospec-release` to consume as a release-readiness signal:

```
## /autospec-qa heal loop summary

| Round | Findings | Issues filed | PRs merged | Time | Status                  |
|-------|---------:|-------------:|-----------:|------|-------------------------|
| 1     |        7 |            7 |          6 | 12m  | 1 finding persisted     |
| 2     |        1 |            0 |          0 |  4m  | oscillation_detected    |

Final verdict: PARTIAL (1 persistent finding)
```

Invocation:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/qa-heal-loop.sh" \
    --verdict .autospec/qa-verdict.json \
    --summary .autospec/heal-summary.md
```

Safety guardrails (NOT bypassed by heal mode) — inherit all existing
autonomy/release guardrails:

- Destructive remote actions surface for confirmation (handled by
  `autospec-autonomy-gate.sh`).
- Per-PR merge still goes through the rebase-and-retest gate.
- `~/.autospec/no-heal.flag` exists → heal globally disabled (operator
  escape hatch for runaway loops).
- `~/.autospec/stop.flag` (graceful / immediate) terminates the loop at
  the next round boundary.

## --no-heal opt-out

The heal loop runs by default (issue #711 locks the default-on contract;
issue #666 introduced it). Four opt-outs revert to discovery-only
behavior; all four are equivalent and only post-discovery behavior
differs:

1. `--no-heal` flag.
2. `--single-pass` flag (alias).
3. `~/.autospec/qa-no-heal.flag` operator preference.
4. `~/.autospec/no-heal.flag` legacy operator preference (still honored).

When any of the four is present, `/autospec-qa` writes
`.autospec/qa-verdict.json` and exits without filing issues or
dispatching `/autospec-run`. Triggers are identical for both modes.
Backwards compatibility: any caller that relied on `/autospec-qa`
exiting after discovery must pass `--no-heal`. The
`.autospec/qa-verdict.json` schema is unchanged. Existing
`/autospec-release` and `/autospec-sweep` integrations see the post-heal
verdict, not the pre-heal one — this is the intended improvement.

## Shared loop driver (issue #711)

`qa-heal-loop.sh` adopts the shared loop driver
`lib/autospec-loop.sh` (issue #708 / PR #712) for unified
termination naming. The four loop-enabled skills — `/autospec --loop`,
`/autospec-refine --continue`, `/autospec-continue --loop`, and
`/autospec-qa` (heal default-on) — share these termination conditions:

- `convergence_clean` — zero release-blocking findings (heal alias:
  `convergence`).
- `oscillation_detected` — finding-set hash repeats round-over-round.
- `round_cap_reached` — `AUTOSPEC_HEAL_MAX_ROUNDS` reached.
- `evidence_based_stop` — `stop_marker` / `STOP:` field in the QA
  verdict.
- `operator_stop` — `~/.autospec/stop.flag` or
  `~/.autospec/qa-heal-stop.flag` (heal alias: `stopped`).
- `budget_cap_reached` — token or wall-time cap exceeded.

Loop end produces `.autospec/qa-heal-summary.md` (mirrored from the
legacy `.autospec/heal-summary.md` path) with the same Markdown table
shape as `.autospec/loop-summary.md` (refine/continue/autospec --loop)
so `/autospec-release` and downstream consumers see structurally
identical summaries across all four loop-enabled skills.

## Production incident regression check

When QA passes but production breaks, the same bug class tends to leak again the
next cycle because nothing in the QA pipeline learns from the incident. The
registry `.autospec/production-incidents.json` records every production incident
and the regression test that proves it cannot reoccur silently. `/autospec-qa`
reads this registry on every run and asserts, for each `active` incident:

1. A named `regression_test` file exists.
2. That test actually runs.
3. The test is non-trivially assertive (no `expect(true).toBe(true)` or empty
   stub).
4. The test FAILS when the incident's recorded `mutation` is applied to the
   implementation — proving the test would catch a regression rather than
   passing for unrelated reasons.

Missing any of these four checks emits a finding under category
`code_health:production_incident_regression_missing` with
`release_blocking: true` in `.autospec/qa-verdict.json`.

Registry schema (one JSON array, one entry per incident):

```json
[
  {
    "id": "INC-2026-05-28-checkout-double-charge",
    "occurred_at": "2026-05-28T03:14:00Z",
    "failure_mode": "concurrent submit creates duplicate Stripe charge",
    "feature": "checkout/payment",
    "spec_anchor": "docs/specs/checkout.md#payment",
    "regression_test": "tests/regression/incident-2026-05-28-double-charge.test.ts",
    "mutation": "remove the `submitting` guard on the Pay button; test must FAIL",
    "added_by": "<operator handle>",
    "status": "active"
  }
]
```

`status` ∈ `active | superseded | wont_test`. Only `active` is enforced.
`superseded` means the underlying behavior was redesigned out of existence and
must cite the replacement spec section. `wont_test` requires explicit
justification (e.g., "infra-level only, no test can reach it") and emits a
warning, never a release blocker.

Never silently delete an incident entry. Operators move incidents to
`superseded` or `wont_test` with a citation/justification so the historical
record of why a regression test was added stays auditable. The orchestrator
runs the helper after the existing verdict computation, similar to other
post-verdict gates:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/qa-incident-check.sh" \
    --registry .autospec/production-incidents.json \
    --verdict  .autospec/qa-verdict.json
```

Exit 0 means every `active` incident is covered. Exit 1 means at least one
active incident is missing coverage and a release-blocking finding has been
appended to the verdict. The helper validates the registry against
`schemas/autospec-production-incidents.schema.json` and writes findings
atomically.

## Post-merge deployed canary prompt

When a deployed/dev URL exists, define the post-merge canary before release:

```text
For this feature, list the smallest deployed/dev workflow set that must run
after merge or deploy:

- route or screen
- representative input
- expected live API/backend path
- expected visible domain result
- failure banners that must not appear
- health probes to run if the canary fails
- issue label or escalation path for a failed canary

Required outcome:
- A missing canary for a backend-backed feature is PARTIAL unless the reliability
  contract explicitly exempts it.
- A failed canary creates a regression issue with logs, route classification,
  and live evidence.
```

## No-mock minimum coverage prompt

Mocked tests are not enough for backend-backed features:

```text
For every spec feature that depends on backend behavior, identify the minimum
no-mock path that proves real integration:

- one representative happy path,
- one known fragile backend path or fallback path,
- one visible success result,
- one absence-of-generic-failure assertion.

Required outcome:
- If no no-mock path exists, update `.autospec/reliability.yml` with the blocker
  and setup action.
- If the app has only mocked coverage, do not mark the backend-backed feature
  PASS.
```

## New code intent gate

Every new component, module, function, endpoint, worker, hook, or test helper
must answer why it exists:

```text
For this new code unit:

1. Which spec requirement or issue acceptance criterion owns it?
2. Why is an existing helper, component, service, or test utility not enough?
3. What is the smallest public contract this unit exposes?
4. What automated test and, when applicable, live workflow proves it?
5. What would break if this unit were wrong?

Required outcome:
- If there is no spec-linked reason, remove the code or file a scope/spec issue.
- If there is an existing helper, prefer reuse over a new abstraction.
```

## Legacy removal and spec cleanup gate

When the current spec, operator direction, or accepted architecture deprecates a
route, cache, bucket, storage backend, worker, feature flag, config key, UI path,
or data source, autospec must remove the legacy surface and its spec/docs
references instead of making the deprecated path work again.

```text
For each legacy or deprecated surface:

1. What replaced it, and where is that replacement specified?
2. Which code paths, feature flags, env vars, infrastructure resources, docs,
   tests, fixtures, runbooks, and specs still reference the legacy surface?
3. Did any smoke/test/repair step try to make the app pass by repopulating,
   restarting, or relying on the deprecated surface?
4. What data migration, cleanup, or compatibility window is required before
   removal?
5. Which tests prove the replacement path works and the legacy path is no longer
   used?
6. Which specs or docs were updated to remove or mark the legacy behavior as
   intentionally unsupported?

Required outcome:
- Do not populate deprecated caches, buckets, queues, indexes, or fallback stores
  just to make a live smoke pass.
- If a deployed environment still depends on a deprecated surface, mark the row
  PARTIAL/FAIL and file an implementation/infrastructure cleanup issue.
- Remove stale spec/docs references in the same change when implementation
  removes legacy behavior.
- If compatibility must remain temporarily, document the sunset condition,
  owner, and follow-up issue; tests must still prove the new canonical path.
```

## Brute-force string heuristics sweep

Sweep already-merged code for the LLM-tier RULE_IDs `STRING_MATCH_DOMAIN_LOGIC`
and `REPEATED_STRUCTURE_AS_CODE` (AGENTS.md `## Implementation-quality contract`).
This catches rot that pre-dates the PR-time guardian — substring-on-name
classifiers and ≥5-branch ladders that ship as a wall of `if`/`elif`/`switch`
instead of a table + dispatcher.

Supported languages: Python, JavaScript/TypeScript, Go, Java, Scala, Rust.

```bash
# Run from repo root. Emits findings into .autospec/qa-verdict.json under
# the category `code_health:brute_force_string_heuristics` and files one
# auto-implement issue per offender.
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/qa-brute-force-sweep.sh"
```

The sweep:

1. Walks every `*.py`, `*.js`, `*.ts`, `*.jsx`, `*.tsx`, `*.go`, `*.java`,
   `*.scala`, `*.rs` file outside `node_modules`, `.git`, and `dist`.
2. For `STRING_MATCH_DOMAIN_LOGIC`: counts substring-style checks
   (`.contains(...)`, `.includes(...)`, `in name`, `strings.Contains`,
   etc.) and confirms the file imports a proper-representation library for
   its language — Python `rdkit`/`ast`/`urllib.parse`/`datetime`/
   `ipaddress`/`lxml`/`jsonschema`; JS/TS `URL`/`Date`/`@babel/parser`/
   `acorn`/`ts-morph`/`zod`/`ajv`/`joi`; Go `net/url`/`time`/`go/ast`/
   `net.ParseIP`/`encoding/json`; Java `java.net.URI`/`java.time`/
   `JavaParser`/`com.github.javaparser`/`javax.validation`; Scala
   `java.net.URI`/`java.time`/`scalameta`/refined/circe; Rust `url::Url`/
   `chrono`/`time`/`syn`/`std::net::IpAddr`/`serde`.
3. For `REPEATED_STRUCTURE_AS_CODE`: counts repeated branch-shaped lines
   (`if`/`elif`/`case`/`match` arms) and flags any file with ≥5.
4. Writes one finding line per offender to `.autospec/qa-verdict.json`
   under category `code_health:brute_force_string_heuristics`, carrying
   `rule_id`, `language`, `file`, `function`, `line`.
5. Files one `auto-implement,autospec:v2-flow` GitHub issue per offender
   via `gh issue create`. Issue body cites the file, function/method, line,
   and the verbatim RULE_ID directive from AGENTS.md so the implementer
   retry loop has the corrective instruction. Where the offending file has
   no proper-representation library imported, the issue notes
   "no proper-rep library imported — investigate first" so the implementer
   does not blindly add a dependency.

Errors in the sweep MUST NOT block the QA verdict; emit findings and
continue. End-to-end regression: after the QA-filed rewrite issues run
through `/autospec-run`, the offending files no longer trip either
RULE_ID in the PR-time guardian.

## Bug-class sibling sweep (issue #737)

When `.autospec/qa-verdict.json` carries a `release_blocking: true`
finding whose `category` ends in `:bug` (or otherwise matches a
fingerprintable shape), run `qa-bug-class-sweep.sh` to fan out
across the repo for sibling instances of the same pattern class. Goal:
consistency by construction — the implementer fixes every matching
instance in **one PR**, not N PRs.

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/qa-bug-class-sweep.sh" \
    --verdict .autospec/qa-verdict.json --repo-dir .
```

The sweep:

1. Computes a deterministic **pattern fingerprint** over the offending
   line + ±5 context lines (literal-stripped: strings → `""`, numbers →
   `N`). The fingerprint and its sha256 (`pattern_fingerprint`,
   `fingerprint_hash`) are stored on the PARENT finding **at
   finding-time**, not recomputed — this gives pattern drift safety so
   siblings remain resolvable even after the parent line is rewritten.
2. Runs `git grep -nE <fingerprint>` over the repo, excluding
   `node_modules/`, `.git/`, and `vendor/`, plus the parent's own
   `file:line`.
3. For each match, computes a **confidence** score
   `(presence + density_bonus) * specificity` where specificity scales
   with fingerprint length (longer anchors → higher confidence). Matches
   with `confidence >= AUTOSPEC_QA_BUG_CLASS_MIN_CONFIDENCE` (default
   `0.7`) are appended to `findings[]` with
   `category: code_health:bug_class_sibling`, `parent_fingerprint`,
   `confidence`, `file`, `line`. Below-threshold matches land in
   `.autospec/qa-bug-class-flagged.json` (operator review surface, NOT
   filed as issues).
4. Applies the verify-first filter
   (`qa-verify-finding.sh --category missing_function`) to every
   candidate sibling. If the bug is already fixed at HEAD for an
   instance, that sibling is dropped silently.
5. All verdict mutations are atomic (`.tmp` + `mv`) so partial runs
   cannot corrupt the verdict.

Per-class issue filing: `qa-heal-loop.sh` groups findings by
`parent_fingerprint` when present and files **one `auto-implement` issue
per pattern class** carrying every instance's `file:line`. Findings
without a fingerprint stay per-finding as before. This is the key
contract that turns bug-class detection into a single coordinated fix.

## Verify-first discipline

Before any cluster emits a finding to `.autospec/qa-verdict.json`, the cluster
MUST verify that the symptom still reproduces at current HEAD. This eliminates
the dominant false-positive class observed in production: clusters re-flagging
gaps that landed in a recent PR. Run the shared probe per candidate finding:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/qa-verify-finding.sh" \
    --category <missing_function|missing_import|failing_test|spec_mismatch|regression> \
    [--symbol <name>] [--file <path>] [--test-cmd <cmd>] \
    [--spec <path>] [--impl <path>] [--claim <text>] [--commit <sha>]
```

Exit 0 means the finding still reproduces (KEEP and emit). Exit 1 means the
symptom no longer reproduces (DROP and append to `verified_dropped[]` in the
QA report). When ingesting prior-cycle findings from a previous
`.autospec/qa-verdict.json`, run the same probe; drops go into
`stale_dropped[]` rather than `verified_dropped[]`.

Cluster cross-talk dedup: the QA orchestrator maintains
`.autospec/qa-cluster-coverage.json` via the shared coverage helper. Each
cluster MUST attempt to claim the `(file, spec_section, rule_id)` tuple it is
about to scan; a second cluster racing on the same tuple receives exit 1
("already claimed") and logs the skip as `cluster_skip_dedup`:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/qa-cluster-coverage.sh" \
    claim --cluster <id> --file <path> --section <section> --rule <rule_id>
```

The final QA report MUST surface four explicit count fields so the operator
can see what the discipline filtered: `findings_emitted`, `verified_dropped`,
`stale_dropped`, `cluster_skip_dedup`.

After cluster fan-out completes, the QA orchestrator MUST run
`qa-finding-filter.sh` against the merged
`.autospec/qa-verdict.json` before any release-skill (e.g.,
`/autospec-release`) consumes it. The filter partitions findings into
verified, stale (re-probed and either refreshed or dropped to
`verified_dropped[]`), and unverified. Under `AUTOSPEC_QA_STRICT=1`
(the CI default), any finding lacking `verified_at` causes the QA run
to FAIL with status `code_health:verify_first_violation`; under
`AUTOSPEC_QA_STRICT=0` unverified findings are routed to
`unverified_warnings[]` for operator triage. Cluster agents MUST emit
findings via `qa-verify-finding.sh --emit-record <path>` so the
required `verified_at`, `verified_by`, and `verify_category` fields are
populated by the probe rather than hand-constructed.

## Cluster sizing

The QA orchestrator's cluster fan-out is a knob, not a constant. When the
prior QA run dropped ≥30% of candidate findings on verification, prefer **3
verify-first clusters** over **7 blind clusters** for the next run. A drop
rate >50% means clusters are re-litigating the same closed gaps rather than
exploring new surface; shrink the fan-out and let each cluster do deeper
verification work. This is prose guidance to the orchestrator — no enforcement
script — but the rule is non-negotiable for runs whose prior-cycle drop rate
exceeded the threshold.

## Dogfood detectors driver

When QA runs against the autospec repo itself, use `dogfood-detectors.sh` as the authoritative aggregated sweep rather than invoking individual `scripts/qa-*-sweep.sh` directly. The driver enforces an allowlist of intentional findings and fails on regression.

## Critical self-questioning checkpoint

Before finalizing the QA report or accepting regenerated tests, stop and answer
these questions in the working notes:

1. What could still pass in the current mocked tests while failing for a real
   user on the deployed/dev environment?
2. Which user-visible outcome proves the feature worked, independent of DOM
   wiring or request dispatch?
3. Which backend route, service, queue, bridge, cache, or external dependency am
   I assuming is healthy?
4. If that dependency fails, is there a tested fallback route or a clear
   actionable user state?
5. Which button, dropdown, select, text box, validation rule, or error state did
   I not touch because it was inconvenient, slow, or required setup?
6. Does the test prove the intended domain result, or only that a generic panel
   changed?
7. What is the single highest-risk missing test I can add now?
8. Did I search safe deployment/config surfaces deeply enough to distinguish
   missing credentials from stopped infrastructure or a route-specific worker
   failure?
9. Did I emit proof artifacts that connect the spec, implementation, tests,
   live evidence, and remaining blockers?
10. Did I challenge duplicated code and require a spec-linked reason for every
   new code unit?
11. Did I prove at least one critical workflow test fails when the workflow is
   intentionally broken?
12. Are the proof artifacts fresh for the current commit, spec hash, app URL,
   environment, and test command?
13. Does every PASS have evidence provenance a reviewer can inspect?
14. Did browser console and network inspection reveal any unexpected errors?
15. Did I resolve or file spec contradictions before treating behavior as done?
16. For stateful features, did I prove the data lifecycle across create, refresh,
   edit, delete, rollback, and navigation?
17. Is there enough observability to debug a failed canary without exposing
   secrets or PII?
18. Did I classify every retry, sleep, timeout increase, or flaky rerun instead
   of accepting green-after-retry?
19. What else could make this generated app unreliable, duplicated, or falsely
   green against the spec?
20. Did I remove deprecated code paths and stale spec/docs references instead of
   reviving a legacy cache, bucket, route, worker, or fallback to make QA pass?

If any answer identifies a material gap, add the narrowest test for it or file a
follow-up issue with reproduction steps. Do not silently leave it as an
unstated risk.

## Inputs

Collect or infer:

- Spec path or URL.
- App URL or local run command.
- Repository path.
- Test credentials and seed data, if needed.
- Relevant PR/issue numbers.

If the app cannot be run and no URL is available, continue with static/test
audit only, but mark all runtime UI checks `NOT TESTED` with the exact blocker.

## QA prompt

Use this prompt as the audit driver:

```text
You are a senior QA engineer, product tester, and spec-compliance auditor.

Your job is to test this application completely from every meaningful scope and
direction, and to verify that every feature specified in the spec actually works
in the running app.

Inputs:
- Spec: <paste or link the product/spec document>
- App URL or local run command: <URL or command>
- Repository path, if available: <path>
- Test credentials/data, if available: <credentials/data>

Mission:
Create and execute a complete verification pass that proves whether the
implemented app matches the spec. Do not only inspect code. Run the app and
interact with it like a real user. Treat the spec as the source of truth.

Required testing scope:

1. Spec Traceability
- Extract every feature, requirement, acceptance criterion, user flow,
  validation rule, permission rule, and edge case from the spec.
- Build a traceability matrix with requirement, expected behavior, test
  performed, PASS/FAIL/PARTIAL/NOT TESTED result, evidence, and notes.

2. Functional Testing
Test every user-facing feature end to end: buttons, links, text inputs,
textareas, checkboxes, radio buttons, selects, dropdowns, multi-selects, search
fields, filters, sorting, pagination, tabs, modals, forms, uploads/downloads,
navigation, save/edit/delete flows, auth/login/logout, and empty/loading/
success/error/disabled states.

For every workflow button or dropdown-backed action, do not stop at clickability
or request dispatch. Verify the selected values change the real request
payload/query, run a mocked happy path, run a no-mock deployed/dev smoke path
with representative input, assert no live-failure banner appears, and assert a
real domain result is visible.

For every user input element, run the User Input Intent Prompt: identify what
the control is for, why it exists, how it should be used, what request/state/
visible result it should affect, how invalid or rejected input behaves, whether
it is intentionally decorative/read-only, and whether running-app evidence proves
it was implemented and tested correctly.

When live backend smoke is blocked by config, credentials, bridge/queue errors,
timeouts, or stopped services, run the Live Backend Blocker Triage Prompt before
declaring `NOT TESTED`. Search safe config surfaces, do not print secret values,
clean up temporary credential material, separate passed live paths from failing
routes, and classify whether the blocker is credentials, infrastructure health,
route mental model, or a route-specific worker/handler failure.

Create or draft the proof artifacts: `.autospec/proof-matrix.json`,
`.autospec/reliability.yml`, `.autospec/control-intent-ledger.json`,
`.autospec/mutation-proof.json`, and `.autospec/canary-results.json` when a
deployed/dev canary exists. Use them to prevent false PASS results: unclassified
controls, missing backend no-mock coverage, missing mutation proof, duplicated
code, or missing code intent keep the result PARTIAL or FAIL until resolved.
Validate artifacts against their schemas when possible, reject stale artifacts,
require evidence provenance for every PASS, fail unexpected console/network
errors, detect spec contradictions, prove data lifecycle for stateful features,
require observability for backend-backed workflows, classify flakes, and run the
Reliability Exhaustion Loop until all actionable reliability ideas are handled
or explicitly filed. Run the Legacy Removal and Spec Cleanup Gate whenever a
workflow touches a deprecated route, cache, bucket, storage backend, worker,
feature flag, config key, UI path, or data source.

3. Form and Validation Testing
For every form/input test valid input, empty input, invalid input, boundary
values, very long text, special characters, whitespace-only values, duplicates,
required-field behavior, error-message clarity, recovery after correction,
keyboard submission, disabled submit behavior, and server-side validation.

4. UI and UX Behavior
Verify text readability, labels, discoverability, visible state changes,
loading/error/success states, desktop/tablet/mobile layouts, and absence of
overlapping or clipped content.

5. Accessibility
Check keyboard-only navigation, focus order, visible focus indicators, input
labels, ARIA where needed, screen-reader names for icon buttons, color contrast,
Escape behavior, and Enter/Space behavior.

6. Data and State Testing
Verify persistence after refresh, unsaved-change handling, optimistic update
reconciliation, delete/edit propagation, filter/search/sort state, browser
back/forward behavior, and deep links.

7. Negative and Edge Cases
Test API failure, slow responses, empty datasets, large datasets, unauthorized
access, expired sessions, invalid routes, concurrent edits, repeated rapid
clicks, and refresh during in-progress actions.

8. Backend/API Integration
When a backend exists, verify frontend requests match API contracts, API errors
are handled correctly, storage reflects create/update/delete behavior, and
permissions/validation are enforced server-side.

Include regression coverage for deployed backend failures such as `502` bridge,
proxy, queue, or cache misses. If a backend route is known fragile, the UI must
either recover through a documented fallback route or show a clear actionable
state. Mocked endpoint success is not sufficient evidence.

9. Regression and Cross-Feature Testing
Combine flows: create then search/filter/sort, edit then verify list/detail
views, delete then verify navigation/empty states, change settings then verify
dependent screens, and complete full user journeys.

10. Automated Test Recommendations
Identify missing unit, integration, component, E2E, accessibility, and API
contract tests. For each missing test, state exactly what it should assert.

11. Reliability and Maintainability Proof
Validate the reliability contract, proof matrix, control intent ledger,
duplicate-code guardian findings, mutation/breakage proof, post-merge canary,
no-mock minimum coverage, and new-code intent gate. Treat these as required
spec-compliance evidence, not optional cleanup.

12. Freshness, Provenance, and Runtime Error Testing
Validate artifact freshness against the current commit/spec/environment, require
reviewable evidence provenance for every PASS, fail unexpected console/network
errors, classify retries/flakes, and record stale or unverifiable artifacts as
PARTIAL.

13. Spec, Data, Observability, and Exhaustion Testing
Detect spec contradictions, prove create/edit/delete/refresh/rollback/navigation
lifecycles for stateful features, verify observability for backend-backed
workflows, and run the Reliability Exhaustion Loop until no new actionable
quality ideas remain.

14. Legacy Removal and Spec Cleanup
Identify deprecated routes, caches, buckets, fallback stores, workers, config
keys, UI paths, and data sources. Verify the canonical replacement works, the
legacy path is not repopulated or relied on, stale specs/docs/tests are removed
or marked unsupported, and any temporary compatibility has a sunset issue.

Execution rules:
- Do not assume a feature works because code appears to exist.
- Do not mark PASS without concrete evidence.
- Mark untestable items NOT TESTED and explain the blocker.
- Mark spec mismatches FAIL even if the implementation seems reasonable.
- Mark ambiguous spec items AMBIGUOUS and propose expected behavior.
- Capture screenshots, logs, console errors, network failures, or command output
  where useful.
- Continue until every spec item has a status.

Final report:
# QA Verification Report
## Summary
- Overall result: PASS / FAIL / PARTIAL
- App/version/commit tested:
- Environment:
- Main risks:
## Spec Traceability Matrix
| Requirement | Expected Behavior | Test Performed | Result | Evidence | Notes |
|---|---|---|---|---|---|
## Proof Artifacts
List `.autospec/proof-matrix.json`, `.autospec/reliability.yml`,
`.autospec/control-intent-ledger.json`, `.autospec/mutation-proof.json`, and
`.autospec/canary-results.json`, or state why each artifact was drafted inline
instead of written.
## Artifact Freshness and Evidence Provenance
List commit/spec/environment/test-command freshness checks and the exact
evidence cited for each PASS row.
## Runtime Error and Flake Classification
List console errors, network failures, runtime warnings, retries, sleeps,
timeouts, flake classification, and fixes or quarantines.
## Spec Contradictions
List contradictory or untestable requirements, UI/API/spec mismatches, proposed
resolution, and follow-up issue or spec edit.
## Data Lifecycle Proof
List create, refresh, edit, delete, rollback, idempotency, navigation, and
concurrency evidence for stateful workflows.
## Observability Contract
List correlation IDs, health probes, error boundary behavior, backend timing,
logs, user-safe messages, and any missing diagnostic hooks.
## Reliability Exhaustion Loop
List each "what else could make this unreliable, duplicated, or falsely green?"
answer and how it was handled.
## Legacy Removal and Spec Cleanup
List deprecated surfaces found, replacement path, code/config/docs/spec/tests
removed or updated, data cleanup or migration action, compatibility window if
any, and evidence that QA did not revive the legacy path.
## Bugs and Gaps
For each issue include severity, area, spec reference, steps to reproduce,
expected, actual, evidence, and suggested fix.
## UI Control Coverage
List every tested text input, select/dropdown, button, form, navigation element,
modal, table/list, and other control.
## Validation Coverage
List every validation rule tested, including valid, invalid, boundary, and
recovery behavior.
## User Input Intent Review
List every input-like control and answer: what it is for, why it exists, how a
user should use it, expected request/state/result effect, invalid/rejected-input
behavior, decorative/read-only status if applicable, and evidence that it was
implemented and tested correctly.
## No-Mock Deployed Smoke Coverage
List every deployed/dev smoke workflow, live URL/config used, representative
input/data, result, and blocker. Any required `NOT TESTED` no-mock smoke row
forces Overall result: PARTIAL unless the QA contract explicitly exempts it.
## Live Backend Blocker Triage
For each blocked live path, list safe config sources checked, secret names or
credential sources referenced without values, temporary credential cleanup,
health/status/fleet probes, passed real-data endpoints, remaining failing route
or subject, root-cause classification, and next operator or implementation
action.
## Presentational Control Classification
List each example chip, shortcut, preset, or token-like control as
decorative/presentational or workflow control, with the evidence or follow-up
issue/task.
## Duplicate-Code Guardian
List duplicated or near-duplicated components, helpers, validators, API clients,
request wrappers, error banners, fixtures, and test utilities, with reused
alternatives or follow-up fixes.
## Mutation and Breakage Proof
List critical workflow tests, the simulated break or mutation, expected failing
assertion, observed result, and strengthening performed if the test stayed green.
## Benchmark Overfit and Test Leak
List tests whose fixtures hard-code public benchmark identifiers, use a fixed
first-N slice, assert exact benchmark answer-key values, lack a randomized or
property-based sibling, or reuse the same fixture as training/selection signal.
For each: test path, leaked dataset, failure mode (1-6 from the gate), and
required remediation.
## Outsourced Implementation
List third-party API/SDK calls or cached external payloads where the spec
required the product to implement the action locally. For each: spec
acceptance criterion, offending client/URL, matched signal(s) from the gate
(1-6), and remediation (replace remote call with in-repo algorithm; keep
external service only as optional comparator behind a flag).
## Post-Merge Deployed Canary
List required canary workflows, representative input, deployed/dev URL, expected
domain result, result, and regression issue/action for failures.
## New Code Intent Gate
List new code units reviewed, owning spec/AC, reason existing helpers were not
enough, public contract, proof test, and what would break if wrong.
## Accessibility Findings
Include keyboard, focus, labels, contrast, ARIA, and screen-reader concerns.
## Cross-Browser / Responsive Findings
Include desktop, tablet, and mobile behavior if applicable.
## Automated Test Gaps
List exact tests to add before the app is fully protected.
## Final Verdict
State whether the app satisfies the spec and the minimum fixes required before
release.
```

## Regeneration loop

After the audit:

1. Convert every `FAIL`, `PARTIAL`, `NOT TESTED`, and high-risk `AMBIGUOUS`
   traceability row into one of:
   - a product bug fix,
   - a regenerated/strengthened automated test,
   - a follow-up GitHub issue when implementation scope is too large.
   A required no-mock deployed smoke row that remains `NOT TESTED` must produce
   an environment setup issue/task, a QA contract update, or an operator-blocker
   line with exact rerun instructions.
   Any input-like control with an unanswered User Input Intent Prompt must
   produce a strengthened test, implementation fix, spec clarification, or
   follow-up issue.
   A blocked live-backend row must run the Live Backend Blocker Triage Prompt
   first and then produce either real no-mock smoke evidence or a classified
   credentials/infrastructure/route/worker blocker.
   Missing proof artifacts, unclassified controls, duplicated generated code,
   missing mutation/breakage proof, missing canary, missing no-mock minimum
   coverage, and unanswered new-code intent gates must each produce a
   strengthened test, implementation fix, reliability contract update, or
   follow-up issue.
   Stale artifacts, missing evidence provenance, unexpected console/network
   errors, unresolved spec contradictions, missing data lifecycle proof, missing
   observability for backend-backed failures, unclassified flakes, and any
   actionable Reliability Exhaustion Loop answer must also produce a fix,
   stronger test, proof update, contract update, or follow-up issue.
   Any deprecated surface still referenced by code, config, specs, docs, tests,
   fixtures, infrastructure, or live-smoke repair steps must produce removal,
   spec cleanup, migration work, or a sunset follow-up before PASS.
2. Regenerate tests in the narrowest existing harness:
   - unit tests for pure functions and validation rules,
   - integration/API tests for server-side validation and persistence,
   - component tests for isolated UI states,
   - E2E/Playwright tests for user journeys, controls, accessibility, and
     cross-feature behavior.
3. Preserve or strengthen assertions. Never replace a specific assertion with a
   weaker smoke assertion.
4. Run the relevant test commands and the repo's validation command.
5. Repeat until the traceability matrix has no unexplained gaps or the remaining
   gaps are filed as issues with reproduction steps and evidence.

## QA verdict artifact

At the end of every audit run (including aborted runs), write
`.autospec/qa-verdict.json` as the machine-readable result. Downstream
skills — notably `/autospec-release` — read this file as the authoritative
gate signal instead of re-parsing the prose report.

Schema:

```json
{
  "verdict": "PASS|PARTIAL|FAIL",
  "head_sha": "<git rev-parse HEAD at run start>",
  "generated_at": "<ISO-8601 UTC>",
  "live_app_proof": true,
  "artifacts": ["path/to/screenshot.png", "path/to/network-trace.har"],
  "findings": [
    {
      "category": "benchmark_overfit|outsourced_implementation|no_mock_smoke|live_backend_blocker|mutation_proof_missing|spec_contradiction|duplicate_code|presentational_misclassified|legacy_residue|new_code_intent|accessibility|cross_browser|visual_fidelity|automated_test_gap|other",
      "release_blocking": true,
      "status": "PASS|PARTIAL|FAIL|NOT_TESTED",
      "summary": "<one-line description>",
      "evidence": "<path or pointer to artifact, report section, or issue>"
    }
  ]
}
```

Rules:

- `live_app_proof` is `true` ONLY if at least one interaction artifact
  (screenshot, DOM snapshot, network trace, recorded session) was captured
  against a booted app whose commit matches `head_sha`. Otherwise `false`.
- `release_blocking` defaults TRUE for:
  - `benchmark_overfit` with failure mode 6 (training-on-test leak),
  - `outsourced_implementation` matching signal 1 (implementation verb
    satisfied by remote call), signal 2 (benchmark/comparator collision),
    or signal 6 (reimplementation directive),
  - `no_mock_smoke` on a required path with any non-PASS status,
  - `live_backend_blocker` unresolved,
  - `mutation_proof_missing` on a critical workflow,
  - `spec_contradiction` unresolved,
  - `legacy_residue` referenced by current user-facing docs.
  Other categories default FALSE; promote to TRUE only when the finding
  documents a user-visible regression, data loss, security exposure, or
  violated spec requirement.
- Write atomically: `.autospec/qa-verdict.json.tmp` then `mv` to the final
  path. Chmod 644. Always overwrite — staleness is checked by `head_sha`,
  not mtime.
- If the audit aborts before completion, still write the file with
  `verdict: "FAIL"` and one finding describing the abort reason. Never
  leave a stale prior verdict in place to be misread as the current run.

## Output contract

Return:

- QA report path or inline report.
- Tests added/changed.
- Product bugs fixed, if any.
- Follow-up issues filed or drafted.
- Proof artifacts written or drafted.
- Reliability contract, control ledger, mutation proof, canary, duplicate-code,
  and new-code intent status.
- Commands run and results.
- Remaining risks.

## Stop mode

If the request is exactly `stop` or `stop` plus `--<word>` flags after
normalization, dispatch to:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>
```

Print the helper output and stop. Do not run the QA audit.

## Harness-aware handoff

Loop dispatch uses `lib/autospec-harness-detect.sh` (issue #723) to
resolve the active AI harness and pick the canonical `/autospec --autonomous`
invocation form:

- Claude Code → `claude "/autospec" "--autonomous" "$PROMPT"`.
- Codex CLI → `codex exec --skip-git-repo-check "/autospec --autonomous $PROMPT"`.
- OpenCode → `opencode "/autospec" "--autonomous" "$PROMPT"` (best-effort).

Detection order: `AUTOSPEC_HANDOFF_DISPATCHER_KIND` env override → skill-mount
probe (`~/.claude/skills` / `~/.codex/prompts` / `~/.config/opencode/agent`) →
PATH probe. Missing dispatcher exits 3 with
`code_health:loop_handoff_no_dispatcher_for_harness`.
