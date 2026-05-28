---
name: autospec-qa
description: Use when the user wants to revalidate a running app against a spec, regenerate missing or weak tests, audit UI controls/forms/validation/dropdowns/API behavior/accessibility, or prove implemented features actually work after autospec-run.
---

# autospec-qa workflow

Run a spec-to-running-app QA audit, then regenerate missing or weak tests until
the application behavior is covered by executable evidence. This skill is the
explicit revalidation companion to `autospec-test`: `autospec-test` gates PRs
with deterministic coverage and E2E checks; `autospec-qa` performs a broad
human-style audit from the spec and turns gaps into stronger tests or follow-up
issues.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-qa   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify
if [ "${AUTOSPEC_NO_SELF_UPDATE:-0}" = "1" ]; then exit 0; fi
mkdir -p "$HOME/.autospec"
LOCKDIR="$HOME/.autospec/.update.lock.d"
LAST="$HOME/.autospec/last-update-check"
INSTALLED="$HOME/.autospec/installed-version"
NOW=$(date -u +%s)
if [ -f "$LAST" ]; then
    PREV=$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null \
        || date -u -d "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null || echo 0)
    if [ "$((NOW - PREV))" -lt 86400 ]; then exit 0; fi
fi
if ! mkdir "$LOCKDIR" 2>/dev/null; then
    echo "WARN: self-update skipped (concurrent update in progress)" >&2; exit 0
fi
trap 'rmdir "$LOCKDIR" 2>/dev/null' EXIT
date -u +'%Y-%m-%dT%H:%M:%SZ' > "$LAST.tmp" && mv "$LAST.tmp" "$LAST"
REMOTE=$(curl -fsSL --max-time 5 \
    "https://api.github.com/repos/berlinguyinca/autospec/commits/main" \
    2>/dev/null | jq -r '.sha // empty' 2>/dev/null | cut -c1-7)
if [ -z "$REMOTE" ]; then
    echo "WARN: self-update skipped (network); continuing on installed version" >&2; exit 0
fi
LOCAL=$(cat "$INSTALLED" 2>/dev/null || true)
if [ "$REMOTE" = "$LOCAL" ]; then exit 0; fi
curl -fsSL --max-time 30 \
    "https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh" \
    | bash -s -- --skill all --harness all --update >/dev/null 2>&1
RC=$?
if [ "$RC" -ne 0 ]; then
    echo "WARN: self-update skipped (install rc=$RC); continuing on installed version" >&2; exit 0
fi
printf '%s\n' "$REMOTE" > "$INSTALLED.tmp" && mv "$INSTALLED.tmp" "$INSTALLED"
# Auto-init cross-tool memory (idempotent, <50ms fast-path)
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
```

## Self-update mode

If the feature-request argument matches `update` after trimming and lowercasing,
re-install the full autospec suite from `main`, show the before/after diff if the
harness exposes it, then stop. Do not run the QA audit.

## Required capabilities & harness adapter

| Capability | Claude Code | OpenCode | Codex CLI | Fallback if missing |
| --- | --- | --- | --- | --- |
| Subagent model tier | Tier A: `opus` + ultrathink | Tier A: top-tier `task` + max reasoning | Tier A: current top GPT + `reasoning_effort=high` | Run inline, but keep the same report contract |
| Browser/E2E execution | Playwright/browser tool or shell | Playwright/browser task | shell + browser when available | Mark UI-only checks NOT TESTED with blocker |
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
## Bugs and Gaps
For each issue include severity, area, spec reference, steps to reproduce,
expected, actual, evidence, and suggested fix.
## UI Control Coverage
List every tested text input, select/dropdown, button, form, navigation element,
modal, table/list, and other control.
## Validation Coverage
List every validation rule tested, including valid, invalid, boundary, and
recovery behavior.
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

## Output contract

Return:

- QA report path or inline report.
- Tests added/changed.
- Product bugs fixed, if any.
- Follow-up issues filed or drafted.
- Commands run and results.
- Remaining risks.

## Stop mode

If the request is exactly `stop` or `stop` plus `--<word>` flags after
normalization, dispatch to:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>
```

Print the helper output and stop. Do not run the QA audit.
