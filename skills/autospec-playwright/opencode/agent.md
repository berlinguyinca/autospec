---
name: autospec-playwright
description: Use when you want to run disciplined no-mock Playwright UI-test authoring (Stage 2A) for a project. Detects the .autospec/test.yml authoring blocks, invokes autospec-test Stage 2A, and prints the coverage report.
mode: primary
---

# autospec-playwright — Disciplined Playwright Authoring Dispatcher

Thin dispatcher for the `autospec-test` Stage 2A disciplined-authoring pipeline.
Reads `.autospec/test.yml` authoring and reset blocks, invokes Stage 2A, and
prints the e2e coverage report from `e2e/.autospec/coverage.json`. Carries NO
machinery of its own — all logic lives in `autospec-test`.

## Self-update mode

Decide this purely from the request text the harness handed you. Do NOT
shell out (no `grep`, `sed`, `[[ =~ ]]`, command substitution, etc.) to
test the user's free-form request — passing it through a shell is what
historically tripped harness permission engines. Read the request, normalize
it in your reasoning (collapse whitespace, trim, lowercase), and if the result is
exactly `update`, this skill enters self-update mode and does NOT run the
normal pipeline.

1. Detect harness by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-playwright/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-playwright.md`
   - Codex CLI:   `~/.codex/prompts/autospec-playwright.md`
2. Re-install from `main` by piping the canonical installer:
   ```
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. Show the diff between the prior installed file(s) and the freshly fetched copy.
4. Stop. Do not enter any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-playwright found; run install.sh first.` and exit.

## Model tier

**Model tier:** `TIER_B` for all dispatch work (thin dispatcher; no design decisions).

## Required capabilities & harness adapter

This workflow assumes a small set of capabilities. Map each one to your harness's actual tool.

| Capability                  | Claude Code                             | OpenCode                              | Codex CLI                             | Fallback if missing                    |
|-----------------------------|-----------------------------------------|---------------------------------------|---------------------------------------|----------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)         | `task` agent in read-only mode        | shell `grep`                          | Do the search in-thread                |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent                   | spawn nested CLI session              | Do the work in-thread                  |
| Ask the user a question     | `AskUserQuestion`                       | inline prompt                         | inline prompt                         | Ask in response and wait               |
| Subagent model tier         | **Model tier:** `TIER_B` for all dispatch work | top task model + medium reasoning | top GPT + reasoning_effort=medium | Honor per-phase tier mapping          |
| Subagent dispatch policy    | per AGENTS.md decision matrix           | per AGENTS.md decision matrix         | per AGENTS.md decision matrix         | inline with main-session token cost    |

## Harness detection

Detect your harness by checking available tools before any phase:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`
   - `TIER_B` = `sonnet`

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available (quota, capacity, or authorization failure),
silently retry with `TIER_A`. Never ask the user.

## Invocation

```
/autospec-playwright [--dry-run]
```

## Screenshot-audit guardrails

Projects may turn the JSON produced by the screenshot audit into documentation
and stable, pixel-independent Playwright checks:

```bash
node skills/autospec-doc/scripts/gen-ui-audit-doc.mjs --input .autospec/screenshot-audit.json --output docs/ui-audit.md
node skills/autospec-qa/scripts/gen-layout-guardrails.mjs --input .autospec/screenshot-audit.json --output e2e/generated/layout-guardrails.spec.mjs
```

The generated suite checks visible content, auth-shell absence, and **document**
horizontal overflow at desktop/tablet/mobile widths. Intentional component
scroll containers (tables and charts) are not treated as document overflow.
Primitive-count checks remain disabled by default; add `--strict-counts` only
when the fixture's live data is deterministic, then include the generated file
in the project's existing Playwright command. No pixel snapshots are required.

- `--dry-run` — print what would be done; do not invoke Stage 2A.

## Dispatcher flow

1. **Locate config.** Read `.autospec/test.yml` via `loadAuthoringConfig()` from
   `skills/autospec-test/scripts/authoring-config.mjs`. If the file is missing or
   `e2e.authoring.enabled` is false, print:
   ```
   autospec-playwright: e2e.authoring.enabled is false (or .autospec/test.yml missing).
   Set e2e.authoring.enabled: true to enable Stage 2A authoring.
   ```
   and exit 0.

2. **Invoke Stage 2A.** Call `autospec-test` Stage 2A — disciplined authoring —
   passing the resolved `spec_dir`, `helpers_dir`, `route_clusters`, `coverage_target_pct`,
   and `fanout_max` from the authoring config. Stage 2A is owned by
   `skills/autospec-test/scripts/stage2a-orchestrator.mjs`; this dispatcher does
   NOT duplicate any of its logic.

3. **Print coverage report.** After Stage 2A completes, read
   `e2e/.autospec/coverage.json` (shape: `{total, covered, pct}`) and print:
   ```
   autospec-playwright coverage: covered=<covered>/<total> (<pct>%)
   ```
   If the file is absent, print `autospec-playwright: coverage.json not found — Stage 2A may not have run.`

4. **Exit code.** Exit 0 on success, 1 on Stage 2A failure.

## Loaded-data dashboard evidence

For autospec-created dashboard E2E or visual tests, do not accept a fixture,
screenshot, or baseline until the test proves the dashboard is past its loading
shell and has rendered service-backed data. The authored test must include at least one loaded-data assertion against
a stable KPI, table row, service field, or route-specific datum that comes from the installed fixture response rather
than from placeholder UI chrome.

Reject any visual baseline or snapshot whose visible page still matches
`/loading|skeleton|empty/i`, including loading spinners, skeleton cards, empty
states, or placeholder-only dashboards. If such text or UI is still present,
wait for the loaded-data assertion to pass or fix the fixture route before
capturing the baseline.

When dashboard tests install network fixtures, register broad catch-all mocks
first, then install route-specific dashboard fixtures after catch-all mocks so
the loaded service response wins over fallback handlers. The acceptance proof
must cite the loaded-data assertion and the fixture route that supplies it.

## Out of scope

- Stage 2A machinery (owned by `autospec-test` issue #996).
- Reset-endpoint generation (owned by `autospec-test` issue #997).
- Loop classification (owned by `autospec-test` issue #998).
- Any Playwright spec file authoring directly — this dispatcher wires; it does not author.
