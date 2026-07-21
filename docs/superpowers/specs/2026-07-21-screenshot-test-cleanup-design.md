# Screenshot Test Signal Cleanup Design

## Goal

Make the screenshot/transcript test suite fail meaningfully when optional tools are unavailable, remove vacuous assertions, and run the suite through the shared package test command without changing production behavior.

## Scope

- `skills/autospec-shared/tests/unit/gen-screenshots.test.mjs`
- `skills/autospec-shared/package.json`
- This design document

The duplicate audit issue #2216 is closed by the same change. No production module, dependency manifest, or Playwright installation policy changes.

## Behavior

The tests retain real Playwright and transcript execution when their tools are present. When an optional tool is absent, tests assert the documented unavailable-tool behavior rather than silently skipping or passing vacuously. Transcript capture must report the selected tool and one recorded artifact for a successful command; the no-tool branch must return `{ tool: 'none', recorded: [] }`. The package's normal test command includes this test file.

## Alternatives

1. **Recommended — test-only cleanup plus package registration.** Smallest diff, preserves real integrations, and makes unavailable-tool branches explicit.
2. Add injectable Playwright/recorder paths to production. This expands the public surface and adds seams without improving real integration coverage.
3. Install Playwright and Chromium in shared test setup. This adds substantial dependency and CI weight while leaving the vacuous assertions unresolved.

## Verification

- `node --test skills/autospec-shared/tests/unit/gen-screenshots.test.mjs`
- `npm test --prefix skills/autospec-shared` (including the screenshot suite)
- Repository validator and scoped static checks
