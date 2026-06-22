# autospec-qa — declarative pre-QA deployment contract (`.autospec/qa-deploy.yml`)

**Date:** 2026-06-22
**Issue:** #694 (`feat: autospec-qa deploy contract`)
**Labels:** `autospec:v2-flow`, `needs-human-review` (failed auto-implement once — this spec is written to be implementable from the text alone)
**Status:** Design

## Problem

`/autospec-qa` today assumes the app under audit is *already running* at the URL the operator hands it. The skill has rich after-the-fact recovery for a dead backend — the **Live backend blocker triage prompt** (`skills/autospec-qa/SKILL.md`) — but no notion of "stand the app up first." For repos like tidyboard where each QA cycle needs a fresh deploy plus a production-data clone into a scoped test family, operators do this by hand *before* invoking `/autospec-qa`, and that work is invisible to the artifact trail.

Two concrete failures result:

1. **Deploy failures masquerade as app bugs.** When the manual pre-deploy fails, QA runs against a dead URL, the triage prompt fires on the resulting `502`/`504`, and the operator must reverse-engineer whether the app is broken or simply never deployed.
2. **The deploy is unrecorded.** When the manual deploy *succeeds*, `.autospec/qa-verdict.json` records nothing about *what* was deployed (SHA), *when*, *where* (target env), or what data was cloned in.

## Goal

A declarative deployment contract at `.autospec/qa-deploy.yml`. **When the file is present**, `/autospec-qa` runs the documented deploy stages **before** the audit (deploy → wait-for-ready → optional data-clone into a scoped test family → record what/when/where into the verdict), fails fast on any stage failure with a clear `code_health:qa_deploy_*` error (never a downstream mystery `502`), and records a `deploy:` block in `.autospec/qa-verdict.json`. **When the file is absent**, behavior is byte-for-byte today's behavior.

Non-goals (out of scope): network-egress allowlist around stage execution (deferred to v2); auto-generating the contract from spec/issue keyword detection; cross-repo deploys / persistent staging across runs; production-data anonymization (operator's responsibility; documented in `safety.notes`).

## Schema

`.autospec/qa-deploy.yml` mirrors the declarative style of `.autospec/test.yml` and `.autospec/autospec.yml`. Parsed with `yq -o=json`, validated against `schemas/autospec-qa-deploy.schema.json` with `ajv` — the exact toolchain `skills/autospec-test/scripts/load-contract.sh` already uses (yq → jq → ajv).

### Fields

| Field | Type | Required | Default | Meaning |
| --- | --- | --- | --- | --- |
| `required` | bool | no | `false` | Governs only whether the file being *absent* could be an error (it never is). The file's *presence* makes deploy mandatory. |
| `stages` | list | **yes** | — | Ordered deploy stages, sequential; first failure aborts the rest. |
| `stages[].name` | string | **yes** | — | Stable id; appears in `deploy.stages_run`. |
| `stages[].command` | string | **yes** | — | Shell command via `bash -c`. Subject to the safety floor. |
| `stages[].timeout` | int (s) | no | `300` | Hard wall-clock cap; exceeding it = stage failed. |
| `stages[].env` | map | no | `{}` | Extra env; values passed verbatim, secret *values* never logged/recorded (only key names). |
| `stages[].health_check.url` | string | yes (in block) | — | URL polled after the command. Subject to `target_envs.forbidden`. |
| `stages[].health_check.expect_status` | int | no | `200` | Ready status. |
| `stages[].health_check.retry` | int | no | `30` | Max poll attempts. |
| `stages[].health_check.retry_interval` | int (s) | no | `10` | Delay between attempts. |
| `stages[].safety.max_records` | int | conditional | — | **Required for any data-clone stage** (name/command matches `clone\|copy\|replicate\|sync`). |
| `stages[].safety.forbid_production_writes` | bool | no | `false` | Operator intent flag; recorded, advisory. |
| `teardown[].name` | string | **yes** (per entry) | — | Identifier. |
| `teardown[].command` | string | **yes** (per entry) | — | Shell command; same safety floor. |
| `teardown[].on_failure` | enum | no | `warn` | `warn` → logs only; `fail` → downgrades a `PASS` verdict to `PARTIAL`. |
| `target_envs.allowed` | list | no | `[]` | Advisory; recorded. |
| `target_envs.forbidden` | list | **yes** | — | **Absolute safety floor** — any forbidden token in a stage command, a `health_check.url`, or stage **stdout** aborts with exit 3. ≥1 entry required. |
| `safety.notes` | string | no | — | Free text. |

### Safety floor (non-negotiable; enforced regardless of operator config)

`scripts/qa-deploy-runner.sh` enforces before/during stage execution; each violation aborts immediately, runs no further stages, exits **3** with the named category in the verdict:

1. **Forbidden-target match** — each `target_envs.forbidden` entry matched (literal substring, case-insensitive) against each stage `command`, `health_check.url`, and captured **stdout** → `code_health:qa_deploy_forbidden_target`.
2. **Production-pattern rejection** — each `command` (deploy+teardown) word-boundary case-insensitive matched against `--prod`, `--production`, ` prod `, ` production `, `production-only`, `live-prod` → `code_health:qa_deploy_prod_pattern`.
3. **`max_records` required for data-clone stages** — name/command containing `clone|copy|replicate|sync` must declare `safety.max_records` → else `code_health:qa_deploy_missing_records_cap`.

The egress allowlist (rule 4 in #694) is **deferred to v2**.

### Example `.autospec/qa-deploy.yml`

```yaml
required: true
stages:
  - name: deploy-to-test-family
    command: bash scripts/deploy-tidyboard-test.sh
    timeout: 600
    env:
      DEPLOY_TARGET: test-family
    health_check:
      url: https://test.tidyboard.org/health
      expect_status: 200
      retry: 30
      retry_interval: 10
  - name: clone-production-data
    command: bash scripts/clone-prod-data-to-test.sh --target test-family
    timeout: 1800
    safety:
      forbid_production_writes: true
      max_records: 10000          # required: stage name contains "clone"
teardown:
  - name: snapshot-test-state
    command: bash scripts/snapshot-test-state.sh
    on_failure: warn
  - name: cleanup
    command: bash scripts/cleanup-test-family.sh
    on_failure: warn
target_envs:
  allowed: [test.tidyboard.org, staging.tidyboard.org]
  forbidden: [tidyboard.org, www.tidyboard.org]
safety:
  notes: "clone script anonymizes PII in step 2; test family is read-isolated from prod."
```

### `deploy:` block added to `.autospec/qa-verdict.json`

```json
{
  "verdict": "PASS",
  "deploy": {
    "stages_run": ["deploy-to-test-family", "clone-production-data"],
    "stage_durations_ms": {"deploy-to-test-family": 145000, "clone-production-data": 902000},
    "target_env": "https://test.tidyboard.org",
    "head_sha_deployed": "abc1234",
    "data_clone": {"source": "prod-readonly", "records_copied": 8500, "max_records": 10000},
    "teardown_run": true,
    "teardown_failures": []
  },
  "findings": []
}
```

`head_sha_deployed` = `git rev-parse --short HEAD` at deploy time. `data_clone.records_copied` parsed from a `records_copied=<int>` stdout line if present; else omitted. The block is **additive** — every existing verdict consumer (heal loop, `/autospec-release`, `/autospec-sweep`) is unaffected.

## Architecture / integration

`scripts/qa-deploy-runner.sh --repo-dir . --verdict .autospec/qa-verdict.json` is invoked at the **very start** of the `/autospec-qa` orchestration — before any cluster fan-out — and its exit code gates the audit:

- no `.autospec/qa-deploy.yml` → exit 0, no-op → today's QA path, unchanged.
- parse + ajv-validate; enforce safety floor (rules 1-3) → exit 3 on hit.
- run stages in order, each under `timeout`; run `health_check` probes (retry/interval).
- any stage/probe failure → exit 1, write `verdict.deploy` + a `qa_deploy_failed` finding.
- success → write `verdict.deploy` block atomically.

A failed deploy never reaches the cluster fan-out: the operator sees a single `code_health:qa_deploy_*` finding naming the failing stage + stdout tail, not a `502` triage trail. The Live backend blocker triage prompt stays as-is for the deploy-succeeds-but-app-unhealthy-mid-audit case.

**Atomic verdict update** uses the sibling `.tmp` + `mv` pattern from `scripts/qa-finding-filter.sh`. If no verdict exists yet (deploy runs first), the runner creates a minimal `{"deploy": {...}}` the later audit merges into.

**Teardown** runs after the verdict is final (unless `--skip-teardown`). `on_failure: fail` rewrites `PASS`→`PARTIAL` (never touches `FAIL`), appends to `deploy.teardown_failures`. `--skip-teardown` sets `deploy.teardown_run: false`.

## Graceful behavior when the file is absent

No `.autospec/qa-deploy.yml` → runner exits `0` immediately, writes nothing, QA proceeds exactly as today (verdict byte-identical, no `deploy:` key). This is the most important compatibility contract and gets its own bats fixture.

## Error handling / exit codes

| Exit | Meaning | Verdict category |
| --- | --- | --- |
| 0 | No contract (no-op), or all stages + probes succeeded | — / `deploy` block |
| 1 | Stage failed/timed out, or a `health_check` exhausted retries | `code_health:qa_deploy_failed` |
| 2 | Usage / contract invalid (ajv fail, malformed YAML, missing required) | `code_health:qa_deploy_invalid_contract` |
| 3 | Safety-floor violation | `qa_deploy_forbidden_target` \| `qa_deploy_prod_pattern` \| `qa_deploy_missing_records_cap` |

`set -u`, NOT `set -e` around stage execution — stage failures are caught explicitly so the verdict records them (per the project's "set -e short-circuit" memory). Secret env **values** never logged. Missing `yq`/`jq`/`ajv` → exit 2 with an actionable `brew install` message (matches `load-contract.sh`).

## Idempotency / re-run safety

Runner is stateless between runs: re-parses, re-runs stages, **replaces** (not appends) the `deploy:` block. Deploy idempotency is the operator's stage-script responsibility (documented in `safety.notes`). A green health check returns on first poll. `--skip-teardown` leaves the env intact for repeated manual re-QA.

## Testing

Real-services rule: no mocking the contract loader/runner logic. The **deploy commands** are PATH-stubbed (a fixture `bin/` whose `deploy`/`clone`/`curl` shims write a real local file / serve a real readiness file), so the contract runs end-to-end against a real local "stage" without network. Safety floor, parse, verdict-write run for real.

`tests/qa/test_qa_deploy.bats` (10 fixtures, per #694 AC): (1) absent contract → no-op exit 0, no `deploy:` key; (2) simple deploy → ordered `stages_run` + durations + target_env; (3) forbidden URL in command → exit 3 `forbidden_target`, zero stages run; (4) prod pattern → exit 3 `prod_pattern`; (5) clone stage missing `max_records` → exit 3 `missing_records_cap`; (6) health check fails after retry → exit 1 `qa_deploy_failed`; (7) teardown `warn` → no verdict change; (8) teardown `fail` → `PASS`→`PARTIAL`; (9) `--skip-teardown` → skipped, `teardown_run:false`; (10) synthetic end-to-end → `deploy:` block matches expected JSON. (bats memory: write any stub-served readiness file to a real temp file before any `[ -f ]`.)

`scripts/validate.sh` gains `check_qa_deploy_contract()` (mirroring `check_closeout_contract`): trio lockstep for the new `## Deployment contract` section (`check_lockstep`/`check_lockstep_duo`); `qa-deploy-runner.sh` exists + executable + `bash -n` clean; `schemas/autospec-qa-deploy.schema.json` valid (ajv compile); runs the bats. Trio prose + goldens are **one atomic change** (memory): edit `SKILL.md`, then `derive-trio.sh --in-place` + `gen-skill-goldens.sh` in the same child.

## File pointers

- `skills/autospec-qa/SKILL.md` — new `## Deployment contract` section, URL intake, Live backend triage prompt to coordinate with, orchestrator entry.
- `skills/autospec-qa/{codex/prompt.md, opencode/agent.md}` — lockstep trio targets.
- `skills/autospec-test/scripts/load-contract.sh` — **the** yq→jq→ajv load+validate pattern to copy.
- `skills/autospec-test/scripts/run-gate.sh` — contract-gates-pipeline + exit-code (0/1/2) convention.
- `scripts/qa-finding-filter.sh` — atomic `.tmp`+`mv` verdict rewrite.
- `skills/autospec-test/tests/fixtures/repos/minimal-valid/.autospec/test.yml` — declarative-yml fixture style.
- `scripts/validate.sh` (`check_closeout_contract`, `check_lockstep`, `check_derive_trio_consistency`) — check-function shape.
- `schemas/autospec-*.schema.json` — schema-file precedent for the new `autospec-qa-deploy.schema.json`.

## Decomposition hint (3-5 small auto-implement children)

1. **Schema + JSON-Schema file.** `schemas/autospec-qa-deploy.schema.json` (`target_envs.forbidden` + `stages` required) + `tests/qa/fixtures/` valid + each-invalid samples. No runner yet.
2. **Runner core: parse + safety floor + no-op.** yq/jq/ajv load (copy `load-contract.sh`), exit-0 no-op on absent file, the 3 safety rules (exit 3), invalid-contract (exit 2). Bats fixtures 1, 3, 4, 5.
3. **Runner stage execution + health probe + atomic verdict write.** Sequential stages under `timeout`, retry loop, `deploy:` block via `.tmp`+`mv`, `qa_deploy_failed`. Bats fixtures 2, 6, 10.
4. **Teardown + `--skip-teardown` + verdict downgrade.** Post-verdict teardown, `on_failure: warn|fail`, PASS→PARTIAL. Bats fixtures 7, 8, 9.
5. **Trio prose + validate gate (atomic with goldens).** `## Deployment contract` in `SKILL.md`, `derive-trio.sh --in-place` + `gen-skill-goldens.sh`, `check_qa_deploy_contract()` in `validate.sh`. Must Close in one PR.

Ordering: 1 → {2,3,4} → 5 (children 2-4 each Close against child 1's schema; child 5 depends on the runner existing).

## Open decisions (operator-level)

1. **`required: true` on a *failed* stage.** Spec: presence ⇒ deploy mandatory regardless of `required`. Alternative: `required: false` = best-effort (failed stage warns, QA proceeds). Strict reading chosen for fail-fast clarity.
2. **Re-deploy every re-run vs skip-if-healthy.** Always re-runs today; an optional `stages[].skip_if_healthy` would save long clones but adds state assumptions. Deferred unless wanted.
3. **`data_clone.records_copied` source.** Spec parses a `records_copied=<int>` stdout convention; alternative is a `--report <path>` JSON.
4. **Forbidden-target matching strictness.** Literal case-insensitive substring (simple, no regex-injection risk per the jq/regex memory); deliberately over-blocks rather than under-blocks. Confirm acceptable.
5. **Teardown on a `FAIL` verdict.** Spec runs teardown regardless (cleanup should happen). Invert to preserve failed envs for forensics?
