# 4581 — `main` now requires the gate its automation runs

`main` had no branch protection, so nothing required a green gate before merge.
Nine unformatted files landed across several merges and `cargo fmt --all --check`
went red on `main`; the conversion pass runs `fmt --check` as its first stage per
patch, so every branch cut from `main` failed a stage for a defect it did not
introduce. The pipeline that turns GPU-hours into PRs stopped because a gate that
nothing enforces is documentation.

The repository setting is now in place (admin, via API — no code path):

- `main` requires `build-test` plus the six TeamCity checks that are reliable
  (file-size-ratchet, python-suites, security-workstream, stack-guard,
  ux-ui-workstream, accessibility-workstream).
- `architecture-fitness` is excluded while it is red at 0s on every PR
  (73 pre-existing violations, 52 frozen baseline data). Re-include it once
  green on `main`.
- `enforce_admins` stays off so the admin merge path remains the explicit,
  recorded-exception channel: the PR body must name the red checks, why each is
  pre-existing, and the clean-`main` reproduction. The rule is written up in
  `AGENTS.d/4581-a-gate-nothing-enforces-is-documentation.md`.

Verified: with `build-test` red at baseline, an open PR reports
`mergeStateStatus: BLOCKED` — the gate gates.
