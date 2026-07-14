# autospec-explore — spec-first issue filing (route rounds through autospec-define)

## Summary

`/autospec-explore`'s **initial** decomposition already runs through
`/autospec-define` (`scripts/autospec-explore.sh:160`), but its **perpetual
loop files each round's proposals raw** via
`gh issue create --label auto-implement` (`scripts/autospec-explore.sh:260-263`)
— with **no design spec**. Every feature explore ships in rounds 2..N therefore
has no spec for the `spec-vs-code` researcher (explore's own highest-weighted
source, 1.0) to drift-check against. The discovery engine that exists to catch
spec/code drift is itself a drift generator.

This spec makes explore file issues the way the rest of autospec does: **write a
spec → decompose into linked issues → implement**. Each round materializes its
ranked proposals into a **round design spec** under `docs/specs/`, commits it to
the sandbox branch, and decomposes it into linked `auto-implement` issues
through the **existing** `/autospec-define` existing-spec (split) path — so every
filed issue carries `spec_path` + `spec_url`, and every shipped feature has a
spec that travels with the code. No new issue-filing machinery is forked
(ROI rule: reuse `/autospec-define`, `gen-issue-skeleton.sh`).

## Problem statement

- `scripts/autospec-explore.sh:260-263` files proposals as bare issues; the
  `title`/`evidence` from the proposal JSON never become a spec.
- The `/autospec-define` existing-spec path (`skills/autospec-define/SKILL.md`
  §Existing spec mode; also `/autospec-split`) is the canonical
  spec→issues decomposer, but it **requires the spec be tracked on `origin/main`**
  before Phase 3 (verified via `git cat-file -e origin/main:<spec-path>`) so
  child issues can link a stable blob URL. Explore works on a **sandbox branch,
  never `main`** — so that gate cannot be satisfied as-is.
- Result: explore can't currently reuse the define/split decomposer for its
  loop, so it shortcuts to raw issues, and drift follows.

## Team personality

- **Selected team:** Product engineering + technical writer + reliability.
- **Why this team fits:** the change is about making the autonomous loop emit
  the same spec-backed, drift-resistant artifacts the human flow does; the
  writer owns the deterministic round-spec template (the spec IS the artifact),
  reliability owns the sandbox-base gate and the never-block fallback.
- **Carry into child issues:** reuse `/autospec-define`, never fork a second
  issue filer; the round spec is committed with the code on the sandbox branch
  so they cannot drift; the loop must degrade gracefully, never hard-block on a
  decompose failure.

## Review counter-team

- **Selected counter-team:** Drift + autonomy safety.
- **What they should challenge:** does the round spec actually get committed
  *before* the issues link to it (no dangling blob URL)? Does the sandbox-base
  variant ever let an explore issue link to `main` (wrong) or file against
  `main` (forbidden)? If `/autospec-define` is unavailable, does the loop stall
  or silently lose proposals? Does per-round batching bury unrelated proposals
  in one spec?

## Architecture

```
research cycle → ranked proposals (verify/severity/ROI per the discovery spec)
        │
        ▼
  materialize ROUND SPEC  ← NEW
    docs/specs/<date>-explore-<slug>-round-N-design.md
    (deterministic template: summary + one section per proposal
     with evidence, acceptance criteria, estimated_complexity, source)
        │
        ▼
  commit + push round spec to the SANDBOX branch  ← NEW
        │
        ▼
  decompose via /autospec-define existing-spec mode  ← REUSE (was: raw gh issue create)
    --base <sandbox-branch>   ← NEW flag: spec-tracking gate + blob URLs
                                resolve against the sandbox branch, not main
    → linked auto-implement issues, each carrying spec_path + spec_url
      (rendered by gen-issue-skeleton.sh)
        │
        ▼
  drain via /autospec-run (PRs target sandbox)   (unchanged)
```

Everything downstream of decomposition is unchanged: the sandbox contract, loop
driver, `/autospec-run` drain. Only the "how proposals become issues" step
changes — from raw filing to spec-first decomposition.

## Round spec contract

- **Path:** `docs/specs/<YYYY-MM-DD>-explore-<sandbox-slug>-round-<N>-design.md`.
- **Generator:** a deterministic renderer (extend `gen-issue-skeleton.sh`'s
  approach or a sibling `gen-explore-round-spec.sh`) consuming the aggregator's
  ranked-proposals JSON. One `## <title>` section per proposal containing its
  `evidence` (file:line), `estimated_complexity`, `source`, `severity`,
  `named_consumer`, and an `Acceptance` block as `- [ ]` checkboxes (so
  `spec-vs-code` can later parse and drift-check explore's own output).
- **Header:** Summary naming the sandbox branch + round, and a back-reference to
  the explore loop summary so the spec is self-describing.
- **Commit:** written, `git add`, committed, and pushed to the sandbox branch
  **before** decomposition runs, so the blob URL the issues link to exists.
- **Granularity:** one round spec per round (batches that round's proposals;
  groups related work and matches the round summary already produced). Each
  proposal becomes one child issue under the round spec. (Per-proposal specs
  are out of scope v1 — see Out of scope.)

## `/autospec-define` sandbox-base integration

The existing-spec gate must accept a base other than `main` so explore can use
it on a sandbox:

- Add `--base <branch>` (default `main`, preserving all current behavior) to the
  `/autospec-define` existing-spec mode and `/autospec-split`. The spec-tracking
  precheck verifies `git cat-file -e <base>:<spec-path>` against `<base>`
  instead of hard-coded `origin/main`, and the child-issue `spec_url` blob link
  uses `<base>` (e.g. `.../blob/<sandbox-branch>/<spec-path>`).
- Explore invokes the existing harness-aware handoff it already uses for the
  initial decomposition (`AUTOSPEC_HARNESS_DISPATCHER "/autospec-define …"`),
  now passing `--base <sandbox-branch>` and the round-spec path.
- Guardrail (preserves the explore contract): with `--base <sandbox-branch>`,
  the decomposer MUST NOT target `main` for any PR/issue; it only files issues
  and links the sandbox blob. The "no accidental main merges" rule from
  `docs/specs/2026-05-29-autospec-explore-design.md` §5 still holds.

## Fallback (never block the autonomous loop)

If the `/autospec-define` handoff is unavailable or fails (missing dispatcher,
non-zero exit), explore logs `code_health:explore_define_unavailable` and falls
back to the current raw `gh issue create` path for that round only — but still
commits the round spec, so a later round (or the operator) can decompose it.
The loop never stalls and never silently drops proposals.

## Integration points

- `scripts/autospec-explore.sh` — replace the per-round raw-filing block
  (lines ~255-265) with: render round spec → commit/push to sandbox →
  `/autospec-define --base <sandbox>` decompose → fallback on failure.
- `skills/autospec-explore/{SKILL.md,codex/prompt.md,opencode/agent.md}` —
  the "Research cycle contract" / "Loop driver integration" prose documents
  spec-first filing; regenerate the three sha256 goldens (trio lockstep).
- `skills/autospec-define/{SKILL.md,codex,opencode}` + `/autospec-split` trio —
  document the `--base` flag; regenerate their goldens.

## Testing (validation-via-shell)

- `tests/explore/test_explore_round_spec.bats` — the renderer turns a
  ranked-proposals fixture into a well-formed `docs/specs/*-round-N-design.md`
  with one `- [ ]`-acceptance section per proposal and spec_path/source fields.
- `tests/explore/test_explore_spec_first_filing.bats` — the loop commits the
  round spec to the sandbox branch *before* decomposition; filed issues carry
  `spec_path`/`spec_url` pointing at the sandbox blob; no issue/PR targets
  `main`.
- `tests/explore/test_explore_define_fallback.bats` — when the define handoff
  exits non-zero, the round spec is still committed and the raw fallback files
  the issues with `code_health:explore_define_unavailable` logged; loop continues.
- `tests/define/test_define_base_flag.bats` — `/autospec-define --base <branch>`
  checks spec-tracking against `<branch>` and renders blob URLs for `<branch>`;
  default `main` behavior is byte-unchanged.

## Acceptance

- [ ] `scripts/autospec-explore.sh` materializes a round spec under
      `docs/specs/<date>-explore-<slug>-round-<N>-design.md` from the ranked
      proposals and commits+pushes it to the sandbox branch before filing.
- [ ] Per-round issues are filed via the `/autospec-define` existing-spec
      (split) path, not raw `gh issue create`, and each carries `spec_path` +
      a sandbox-branch `spec_url`.
- [ ] `/autospec-define` existing-spec mode and `/autospec-split` accept
      `--base <branch>` (default `main`, current behavior byte-unchanged) and
      verify spec-tracking + render blob URLs against that base.
- [ ] No explore-filed issue or PR targets `main` under `--base <sandbox>`;
      the explore sandbox-isolation contract is preserved.
- [ ] On `/autospec-define` failure the loop logs
      `code_health:explore_define_unavailable`, still commits the round spec,
      falls back to raw filing, and continues.
- [ ] Round-spec acceptance criteria are `- [ ]` checkboxes so `spec-vs-code`
      drift-checks explore's own shipped features (loop closed).
- [ ] explore + define/split trios document the change and pass `check_lockstep`
      + regenerated sha256 goldens.
- [ ] `autospec validate` gains
      `check_autospec_explore_spec_first_contract()`; all new bats pass;
      `autospec validate` is green.

## Decomposition into child issues

Aiming for 3 children plus an umbrella.

1. **Issue A — round-spec renderer**: deterministic
   `gen-explore-round-spec.sh` (ranked-proposals JSON → round design spec with
   `- [ ]` acceptance) + bats. No loop wiring yet. Files: ~2.
2. **Issue B — `/autospec-define --base` flag**: existing-spec mode +
   `/autospec-split` accept `--base <branch>`, gate + blob URLs resolve against
   it; default `main` unchanged. Trio lockstep + goldens + bats. Files: ~7.
3. **Issue C — explore loop integration + validate gate**: wire the renderer +
   sandbox commit + `/autospec-define --base <sandbox>` decompose + fallback
   into `scripts/autospec-explore.sh`; explore trio prose + goldens;
   `check_autospec_explore_spec_first_contract()`; e2e bats. Depends on A+B.
   Files: ~6.

Total: 3 children + 1 umbrella.

## Out of scope (defer to v2)

- Per-proposal specs (v1 batches a round into one spec; finer granularity only
  if rounds get large).
- Auto-promoting a sandbox round spec to `main` (the operator's sandbox→main
  merge already carries the specs with the code).
- Backfilling specs for already-filed raw issues from earlier runs.
- Spec deduplication across rounds (the recent-issue-title filter already
  prevents oscillation; spec-level dedup is a later optimization).
