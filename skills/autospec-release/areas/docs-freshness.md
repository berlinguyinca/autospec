# Area: docs-freshness

Audit README, AGENTS.md, public docs, runbooks, and user-facing documentation
to confirm they describe current behavior.

## Scope

- Top-level `README.md`, `AGENTS.md`, `SKILLS.md`.
- `docs/USER_MANUAL.md`, `docs/API_REFERENCE.md`, `docs/.llm-manifest.json`,
  `llms.txt`.
- Per-skill `README.md` files.
- Config docs and example files under `examples/`.

## Checks

- Removed flags, dead routes, deprecated config keys must not appear as
  current behavior.
- Compatibility paths must be labeled as compatibility, not preferred.
- New skills shipped in the last release window must be documented in
  README.md, SKILLS.md, and AGENTS.md.
- Generated docs artifacts (USER_MANUAL.md, llms.txt, .llm-manifest.json)
  must be present and fresh — see `check_docs_amendment_presence`.

## Findings shape

- `area: docs-freshness`
- `status`, `release_blocking`, `summary`, `evidence`.
- `release_blocking: true` for user-visible docs that describe removed
  behavior. False for stylistic-only drift.

## PASS criteria

- No removed flag/route/config documented as current.
- All generated artifacts present.
- New skills + commands referenced in top-level indices.

## Token budget

Tier B reviewer; 50-120k context; 25 tool calls cap.
