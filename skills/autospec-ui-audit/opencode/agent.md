---
description: Use when a React Router application needs a deterministic whole-site route inventory before UI, UX, navigation, or evidence auditing begins.
mode: primary
---

# Autospec UI audit: route inventory

Build the route inventory before auditing pages. The inventory is deterministic evidence:
every discovered route appears exactly once as runtime-eligible or explicitly excluded,
and navigation registry gaps remain visible with reasons.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-ui-audit -->

## Required capabilities & harness adapter

| Capability | Claude Code | OpenCode | Codex CLI | Fallback if missing |
| --- | --- | --- | --- | --- |
| Shell execution | Bash tool | shell tool | shell | Required; do not infer inventory in prose |
| Subagent model tier | Tier B: current Sonnet | Tier B: smaller task model | Tier B: current cost-optimized Codex | Run the deterministic command inline |
| Subagent dispatch policy | AGENTS.md decision matrix | AGENTS.md decision matrix | AGENTS.md decision matrix | Inline; the helper owns discovery |
<!-- autospec-block:harness-adapter-core -->

**Model tier:** `TIER_B`. The model only selects inputs and reads artifacts; the Node
helper owns discovery, classification, reconciliation, and failure decisions.

## Invocation

```text
/autospec-ui-audit [--repo PATH] [--output-dir PATH]
```

Defaults: `--repo` is the current repository and `--output-dir` is
`.autospec/ui-audit`. Resolve the installed helper and run it as a direct argument
vector:

```bash
node "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-ui-route-inventory.mjs" \
  --repo "$REPO" --output-dir "$OUTPUT_DIR"
```

Do not replace this command with model-authored route lists, `grep` pipelines, or
browser crawling. If the helper is missing, stop with the install command from this
skill's README rather than silently approximating its output.

## Deterministic contract

The first slice supports React Router route elements with literal `path` attributes.
It discovers nested routes, records lazy route evidence, merges duplicate discoveries
under one canonical path, and excludes catch-all routes with a reason. JavaScript and
TypeScript comments are ignored without modifying quoted strings, and common generated
trees (`dist`, `build`, `.next`, `out`, `coverage`, `target`) are not scanned. It also reads:

- `to` and `href` literals from files whose names contain `nav` or `menu`;
- `<loc>` entries from sitemap XML files;
- literal `goto()` and `visit()` paths from E2E or test files.

The helper writes `route-inventory.json` and `route-inventory.md` to the configured
output directory. Registry-only and route-only paths are mismatches with reasons; they
are never silently dropped. Query strings and fragments are removed before matching,
and duplicate mismatch evidence is combined into one record with all source locations.

The command fails closed before writing artifacts when it finds a cyclic route
collection, a duplicate final record, or a missing classification. Exit `2` means bad
arguments or a missing repository; exit `1` means the inventory could not be proven;
exit `0` means the artifacts satisfy the reconciliation invariants.

## Review the artifacts

Confirm all of the following before handing off to a later UI audit:

- each `routes[]` path is unique;
- every route has `runtime-eligible` or `excluded` status;
- every excluded route has a non-empty reason;
- every mismatch has a source and reason;
- the JSON and Markdown describe the same route set.

Do not call a route reachable merely because it is listed. Runtime navigation evidence
and page scoring belong to later slices.

## Out of scope

- Angular and Next.js adapters;
- browser/runtime evidence capture and reachability proof;
- page quality scoring, accessibility, IA, search, and visual review;
- dashboards, remediation waves, or implementation issue generation.

File follow-up issues for those surfaces without expanding this inventory run.
