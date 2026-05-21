# Custom Invariant Kind Protocol

This document defines the export contract for custom invariant kind modules used by the
`autospec-test` Metric F runner (`run-structural.mjs`) and the `@autospec/test` helper library.

## Overview

Built-in kinds live in `skills/autospec-test/scripts/invariants/kinds/`. Target repos may
register additional custom kinds by placing modules under `.autospec/invariant-kinds/<name>.mjs`
in the target repository root. The runner auto-discovers and loads these alongside built-ins.

## Export Contract

Every kind module **must** export three named exports:

### `id` (string)

The canonical kind identifier used in contract YAML under `invariants[].kind`.
Must be unique across all loaded kinds. Convention: `every_<subject>_<predicate>`.

```js
export const id = 'every_widget_has_refresh_button';
```

### `signature` (object)

JSON Schema describing the parameters the kind accepts from the contract declaration.
Used for contract validation and IDE auto-complete.

```js
export const signature = {
  params: {
    widget: { type: 'string', description: 'Selector for the widget containers.' },
    refresh_label: { type: 'string', description: 'Text label of the refresh button.' },
  },
  required: ['widget'],
};
```

### `run(page, params, ctx)` (async function)

The implementation. Receives:
- `page` — Playwright `Page` object at the target route (already navigated).
- `params` — the invariant declaration object from the contract (your kind's fields).
- `ctx` — `{ baseUrl: string, route: string }` — the current base URL and route path.

Returns a `KindResult`:

```ts
type KindResult = {
  passed: boolean;                    // true iff zero violations
  violations: Array<{
    index: number;                    // 0-based index of the offending element (-1 for global)
    selector: string;                 // selector that failed
    reason: string;                   // human-readable explanation
  }>;
  count_observed: number;             // how many elements were evaluated
};
```

## Worked Example

Suppose your app has a pattern: every dashboard widget must have a "Refresh" button.
Create `.autospec/invariant-kinds/every-widget-has-refresh-button.mjs`:

```js
export const id = 'every_widget_has_refresh_button';

export const signature = {
  params: {
    widget: { type: 'string', description: 'Selector for widget containers.' },
    refresh_label: {
      type: 'string',
      description: 'Accessible name / text of the refresh button.',
      default: 'Refresh',
    },
  },
  required: ['widget'],
};

export async function run(page, params, _ctx) {
  const { widget: widgetSel, refresh_label: label = 'Refresh' } = params;
  const violations = [];

  const widgets = page.locator(widgetSel);
  const count = await widgets.count();

  for (let i = 0; i < count; i++) {
    const w = widgets.nth(i);
    const btn = w.getByRole('button', { name: new RegExp(label, 'i') });
    const visible = await btn.isVisible().catch(() => false);
    if (!visible) {
      violations.push({
        index: i,
        selector: widgetSel,
        reason: `widget at index ${i} is missing a "${label}" button`,
      });
    }
  }

  return { passed: violations.length === 0, violations, count_observed: count };
}
```

Then declare it in `.autospec/test.yml`:

```yaml
e2e:
  invariants_v2:
    enabled: true
    invariants:
      - id: widgets-have-refresh
        kind: every_widget_has_refresh_button
        widget: '[data-testid^="dashboard-widget-"]'
        refresh_label: Refresh
        apply_on_routes:
          - /dashboard
```

## Error Reporting

- Violations are aggregated per-element; the runner emits all violations, not just the first.
- `index: -1` signals a global failure (e.g., `require_count_at_least` not met, no elements found).
- `reason` should be actionable: mention selectors, expected vs actual counts, and route context.

## Registration

The Metric F runner (`run-structural.mjs`) loads custom kinds at startup:
1. Built-in catalog loaded first (this directory).
2. Glob `<target_repo_root>/.autospec/invariant-kinds/*.mjs` — each file's default export is
   checked for `id`, `signature`, and `run`. Missing exports cause a startup warning, not a crash.
3. Duplicate `id` values: last-loaded wins (custom kinds can override built-ins if needed).
