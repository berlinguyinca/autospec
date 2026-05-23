---
name: project-cross-session-ci-rot
description: "Issue #307 cross-session CI rot SHIPPED 2026-05-18 — 3 PRs (#314/#315/#316) merged via autospec-run on autospec itself; Codex peer-review caught 6 real bugs"
metadata: 
  node_type: memory
  type: project
  wing: episodic
  drawer_class: session-log
  originSessionId: 0a77c1fd-c243-4bf9-b3fb-4f83ae5f9830
---

GitHub issue [#307](https://github.com/berlinguyinca/autospec/issues/307) documented three incident classes from a 32-feature parallel `/autospec-run` batch (2026-05-07/08): duplicate `:=` declarations, non-monotonic SQL CHECK constraint allow-lists, and refactor-cascade DOM-text renames. Two deploy-blocking hotfixes resulted. Per-PR CI cannot catch "A is fine, B is fine, A+B together is broken."

**Shipped 2026-05-18, ~25 min wall time:**

| PR | Change | What it does |
|---|---|---|
| [#314](https://github.com/berlinguyinca/autospec/pull/314) | α — rebase-and-retest pre-merge gate | Before `gh pr merge --admin`, polls `gh pr view --json mergeStateStatus`. `BEHIND` triggers `gh pr update-branch` + wait for CI green. Cap 3 attempts (env `AUTOSPEC_REBASE_MAX_ATTEMPTS`). `DIRTY` escalates immediately. Shipped to both v2-flow prompt AND legacy inline prompt. |
| [#315](https://github.com/berlinguyinca/autospec/pull/315) | β — migration-replay pre-PR hook | If `git diff --name-only origin/main...HEAD` matches `*migrations/*` or `*migration*`, first-hit detection across `make migrate-test` / `npm run migrate:test` / `bin/migrate-test` / `pytest tests/migrations -x`. Non-zero exit posts last 200 log lines as issue comment and aborts before `gh pr create`. |
| [#316](https://github.com/berlinguyinca/autospec/pull/316) | γ — target-repo setup guide | New `docs/target-repo-setup.md`. Documents branch protection ("Require branches to be up to date"), 4-target migration-replay convention, why-this-matters, verification one-liner. |

EPIC #310 closed; queue empty; `origin/main` at `d1118a4`.

## Why this run is a milestone

This was the first end-to-end exercise of the just-shipped turbo integration ([[project_turbo_integration_design]]). Codex peer-review caught **6 real bugs across 3 PRs that would have shipped without the absorbed-discipline path:**

1. PR #314: `null`-as-SUCCESS predicate bug in the `wait_for_ci_green` check (a missing rollup was being treated as success).
2. PR #314: silent `gh pr update-branch` failure (output discarded, exit-status ignored).
3. PR #315: missing `git diff --name-only` predicate guard (replay ran on every PR, not just migration PRs).
4. PR #315: missing failure-capture path (non-zero replay rc would have continued to `gh pr create`).
5. PR #316: broken `jq` predicate in the verification one-liner — was checking presence, not `(strict == true) and (contexts length > 0)`.
6. PR #316: unsafe `npm run --silent migrate:test --dry-run` in verification (would have executed the script). Replaced with metadata-only `jq` inspection of `package.json`.

The integration earned its weight on its first run. Peer-review is now established as a real signal, not theater.

## Verbatim user direction (2026-05-17)

> "yeah what next?" (after the turbo integration shipped) — and again "3 + 4" / "2" picking among options I surfaced. The path was: smoke-test → housekeeping → pick-next-issue → close stale #291 → /autospec-define on #307.

Phase 2 design-scope question was explicitly approved by the user:
> "Adopt as-is (Recommended) — 3 issues: rebase-and-retest pre-merge gate + migration-replay hook + target-repo setup guide. Covers all 3 documented incident classes. Defers the cross-session in-flight PR registry as a future layer if these prove insufficient."

## Deferred / open

- **Cross-session in-flight PR registry / file-region locks** — designed but deferred as the 4th layer. Re-evaluate only if the shipped gates prove insufficient after one real multi-session run.
- **UNSTABLE-as-merge-ready** — Codex flagged on #314 as a pre-existing tolerated case; documented in spec but not a code-level guardrail. Operational data should drive any refinement.
- **`wait_for_ci_green` lacks a per-wait timeout** — outer 3-attempt cap exists, but no inner wall-clock cap on a single CI wait. Tolerated for first cut.

## How to apply (when resuming)

- Cross-session CI rot is **shipped, not in-flight**. Don't re-design it; if you need to extend, file a new issue and reference #310/#311/#312/#313.
- All three changes are now baked into `skills/autospec-run/prompts/phase4-implementer.md` and `skills/autospec-run/SKILL.md` (legacy section). Any future Phase 4 prompt edits MUST keep the rebase-and-retest gate and migration-replay hook intact.
- The 4-target migration-replay detection order (Makefile → npm → bin → pytest) is canonical — same order documented in both `phase4-implementer.md` AND `docs/target-repo-setup.md`. Renaming one requires renaming the other.
- Run referenced in [[project_turbo_integration_design]] as the integration's first real-world validation. Update that memory if the integration is later modified.
