# Playwright Control-Effect Coverage - Autospec Design

<!-- autospec-doc-scope:
  src: ["skills/autospec-test/SKILL.md", "docs/superpowers/plans/2026-05-21-autospec-test-invariants.md"]
  reason: "Design plan for improving autospec-generated Playwright tests so visible controls must prove real workflow effects"
  mismatch_action: warn
  generated: false
-->

## Problem

Autospec-generated Playwright tests can pass while missing broken user workflows when they treat E2E coverage as click inventory. The observed failure mode is concrete:

- The test generator ran against clean `origin/main` while the actual UI changes were uncommitted local `web/` work.
- The prompt asked for "every button", so native `select` controls, dropdowns, tabs, checkboxes, and disclosure controls were under-specified.
- The tests clicked controls without asserting the important downstream effect: changed UI state, changed outbound REST request payload/URL, or persisted setting.

This creates green Playwright runs that do not prove the UI works.

## Goal

Make autospec-generated Playwright tests enumerate every visible form/navigation control in the target workflow and require each enabled control to prove a real user-visible, network, or persistence effect.

## Non-Goals

- Do not replace human-written domain E2E tests.
- Do not require every target repo to adopt data-testid selectors before the gate can run.
- Do not allow generated tests to hit production unless the existing autospec-test isolation/scoped-production contract permits it.

## Design

### 1. Test The Right Surface

Playwright generation must target both relevant surfaces when available:

- The current local dev build/worktree that contains the candidate UI changes.
- The deployed or clone URL configured by `E2E_BASE_URL`, `PLAYWRIGHT_BASE_URL`, `BASE_URL`, or `.autospec/test.yml`.

If local and deployed surfaces differ materially, the generated test report must say so and fail with a remediation hint rather than silently testing the wrong UI.

### 2. Control Inventory Contract

For each declared workflow/page, the generator must enumerate visible, enabled, and disabled controls by category:

- `button`, `[role=button]`
- links and tabs
- text inputs, search inputs, textareas
- checkboxes, radios, switches
- native `select` controls
- custom dropdown/listbox/combobox controls
- date/time inputs and pickers
- disclosure/foldout controls
- drag/drop or upload controls

The inventory becomes a JSON artifact attached to the run, for example:

```json
{
  "page": "Usage",
  "controls": [
    {
      "category": "select",
      "selector": "select[name=taxonomy]",
      "label": "Taxonomy",
      "enabled": true,
      "default_value": "lipid",
      "tested_value": "compound",
      "effect_assertion": "request.query.taxonomy == compound"
    }
  ]
}
```

Disabled visible controls fail unless they are explicitly declared read-only in the target repo's `.autospec/test.yml`.

### 3. Effect Assertions

Every non-read-only control test must assert at least one effect after interaction:

- Visible UI state changed.
- Outbound REST request URL, query, headers, or JSON body changed as expected.
- A persisted setting or local/session storage value changed.
- A resulting table/list/chart changed in a way tied to the selected value.

Controls that can be clicked or changed but produce no measurable effect are reported as decorative-control violations.

Dropdown-specific rules:

- Native `select`: assert it has at least two options unless documented read-only; select a non-default option; submit or trigger the workflow; assert the selected option affects request or output.
- Custom dropdown/listbox/combobox: open it, assert options are visible, choose a non-default option, then assert request/output/persistence effect.
- A disabled dropdown is a failure unless `.autospec/test.yml` declares the selector read-only with a reason.

### 4. Workflow Prompt Template

Autospec should inject a stronger Playwright generation prompt:

```text
Create Playwright E2E tests for deployed and local user workflows, not button-only coverage.

For every visible form or navigation control on each workflow:
- enumerate buttons, links, tabs, text inputs, checkboxes, radios, switches, native selects, custom dropdowns/listboxes/comboboxes, date/time controls, disclosures, upload controls, and drag/drop controls
- verify each enabled dropdown can open and select a non-default option
- after changing a dropdown or input, submit/trigger the workflow and assert the outbound REST request uses the selected value
- fail if any visible select/dropdown is disabled unless `.autospec/test.yml` explicitly marks it read-only with a reason
- assert no control is merely decorative: clicking or changing it must cause a visible state change, request change, or persisted setting change
- run against the current local dev build and the configured deployed/clone URL
- report any local/deployed UI mismatch as a test-generation failure
```

Target repos may add domain workflow names, such as Usage, Taxonomy, ID Convert, Batch Convert, Lipid Match, Content, and Settings.

### 5. Autospec-Test Integration

Extend autospec-test Stage 2/Stage 2.5 rather than creating a separate skill:

- Add `e2e.control_effects.enabled` to `.autospec/test.yml`, default `true` for new configs.
- Add `e2e.control_effects.workflows[]` with route/name selectors and optional submit triggers.
- Add `e2e.control_effects.read_only_controls[]` for intentional disabled controls.
- Emit `.autospec/test-artifacts/control-inventory.json`.
- Add a control-effect section to the PR report.
- Feed violations into the self-heal loop as `missing_test` when coverage is absent and `product_bug` when a control is present but ignored by state/API.

## Acceptance Criteria

- [ ] `skills/autospec-test/SKILL.md` documents control-effect coverage and the stronger Playwright generation prompt.
- [ ] `.autospec/test.yml` schema supports `e2e.control_effects.enabled`, `workflows[]`, and `read_only_controls[]`.
- [ ] The Stage 2 or Stage 2.5 gate emits `.autospec/test-artifacts/control-inventory.json`.
- [ ] Generated Playwright tests enumerate native `select` and custom dropdown controls, not only buttons.
- [ ] Dropdown tests select a non-default option and assert a changed REST request or changed rendered output.
- [ ] Disabled visible dropdowns fail unless declared in `read_only_controls[]` with a reason.
- [ ] Local dev URL and deployed/clone URL are both exercised when both are configured.
- [ ] A synthetic target proves a dropdown can render and still fail when its selected value is not sent to the API.

## Primary Smoke Test

```bash
autospec validate
```

## Implementation Slices

1. Schema and docs: add `e2e.control_effects` to the autospec-test contract and setup docs.
2. Inventory helper: implement a Playwright helper that enumerates visible controls by category and writes JSON.
3. Effect verifier: record outbound requests and assert control-selected values reach request/output/persistence.
4. Prompt wiring: update autospec-test generation prompts and self-heal retry prompts.
5. Synthetic targets: add passing and failing mini-apps for native select, custom combobox, disabled read-only controls, and decorative controls.
6. Reporting: surface inventory and violations in PR comments with a direct fix prompt.

## Risks

- Custom dropdown implementations vary widely; start with ARIA roles and add selector overrides in config.
- Request assertions can be noisy in apps with debounced autosave; allow per-workflow submit/settle triggers.
- Running local and deployed surfaces doubles E2E runtime; allow a documented single-surface fallback only when the other URL is unavailable.
