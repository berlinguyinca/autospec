# project-board fixtures

Pinned captures of two live GitHub Projects v2 boards, taken 2026-08-25.

- p1-* : InferWeave/projects/1 — 6 repos, `priority:p0` / `area:security` taxonomy,
         dependencies encoded via `autospec:blocked-prerequisite` + `cross-repo` labels.
- p2-* : InferWeave/projects/2 — 1 repo, `priority/normal` / `area/security` taxonomy,
         dependencies encoded in issue bodies as `## Dependencies` → `Blocked by: #N`.

These are CAPTURES. Never regenerate them with project-board-*.sh — a fixture built by
the code under test cannot catch a bug in that code. Re-capture with `gh` only, and
update the spec's evidence table when the numbers move.
