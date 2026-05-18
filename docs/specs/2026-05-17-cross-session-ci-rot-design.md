# Cross-Session CI Rot — Design

**Date**: 2026-05-17
**Status**: Design, pending implementation plan
**Source issue**: [#307](https://github.com/berlinguyinca/autospec/issues/307)
**Author**: berlinguyinca (with Claude Opus 4.7)

## Problem

When two or more parallel `/autospec-run` sessions ship PRs against the same target repo, per-PR CI proves each diff is correct against pre-merge main, but it cannot detect "A is fine, B is fine, A+B together is broken." After both squash-merge, main is broken and every subsequent PR's CI fails until a human files a hotfix.

Documented from a 32-feature, 14-wave run on 2026-05-07/08. Three concrete incident classes ([#307](https://github.com/berlinguyinca/autospec/issues/307)):

| # | Incident | Root cause |
|---|---|---|
| 1 | Two PRs each added the same `equityLedgerSvc := …` line in `cmd/server/main.go`. Both CIs green; main got duplicate `:=`; `go vet` blocked every subsequent PR. | Stale branch at merge — main moved underneath PR B between B's CI and B's squash-merge. |
| 2 | PR C migration added `'todo_payout'` to a `CHECK` constraint allow-list. PR D's later-timestamped migration DROP+ADD'd the same constraint with a fresh allow-list omitting `'todo_payout'`. Goose runs in timestamp order → `'todo_payout'` silently lost. | Semantic conflict invisible to static per-PR CI. The two migration files don't textually conflict; the regression appears only when run in sequence. |
| 3 | A settings-refactor cascade restructured a page into tabs; unrelated PRs hit `Unable to find an element with the text: /AI & Automations/i` in `settings/page.test.tsx`. | Same as #1 — stale branch + CI doesn't re-run against post-merge main. The refactor and the broken tests are in the same CI suite but no PR ever ran with both diffs applied to current main. |

Result of the 2026-05-07/08 run: 2 deploy-blocking hotfixes, queue stall, human escalation.

## Goal

Eliminate the documented incident classes by adding two pre-merge gates and one operator-facing setup guide to autospec. After this design lands, parallel `/autospec-run` sessions on the same target repo should ship PRs that, taken together, leave main green.

## Non-goals

- **Cross-session in-flight PR registry / file-region locks.** Considered as a fourth layer (preemptive: stop two sessions from BOTH claiming overlapping diffs). Deferred — the rebase-and-retest gate addresses incidents #1 and #3 reactively, and incident #2 is invisible to any file-overlap check. Re-evaluate if the gate proves insufficient after one run cycle.
- **Generic semantic-conflict detection** beyond migrations. Designing a static analyzer that catches all "A+B together is broken" cases is intractable; this design picks the one semantically-rich domain (DB migrations) where target repos already have or can easily add a replay test.
- **Auto-revert of merged-but-broken PRs.** Out of scope for the first cut. Manual revert workflow remains the fallback for incidents the gates don't catch.

## Architecture

Three independent changes, all isolated to autospec's planning + Phase 4 implementer surface. No target-repo code is required for the autospec changes to work — but target-repo opt-in (branch protection + migration-replay test) is where the operational fix lives.

```
target repo PR lifecycle (after this design)
┌────────────────────────────────────────────────────────────┐
│ Phase 4 implementer creates branch <feat/X> off main       │
│   → implements                                              │
│   → tests pass locally                                      │
│   → if diff touches migrations/: run target's replay test  │  ← change β
│   → gh pr create                                            │
│   → CI runs against PR head                                 │
│   ─── pre-merge gate ───                                    │
│   → if PR is behind main: gh pr update-branch              │  ← change α
│       → wait for re-CI green                                │
│       → up to 3 attempts; on stall: comment + escalate     │
│   → gh pr merge --admin --squash --delete-branch           │
└────────────────────────────────────────────────────────────┘

Operator-side opt-in (documented in docs/target-repo-setup.md)
┌────────────────────────────────────────────────────────────┐
│ Branch protection rule: "Require branches up to date       │  ← change γ
│   before merging" on main                                   │
│ Required CI checks: <project-specific>                      │
│ Migration-replay target: one of `make migrate-test`,        │
│   `npm run migrate:test`, `pytest tests/migrations`,        │
│   or `bin/migrate-test`                                     │
└────────────────────────────────────────────────────────────┘
```

The change set is deliberately additive — legacy issues (no `autospec:v2-flow` label) and target repos that don't opt in keep working exactly as today. The new gates only run when the implementer prompt that contains them is loaded (v2-flow path) or when the target repo provides a migration-replay target.

## Change α — Rebase-and-retest pre-merge gate

**Where**: `skills/autospec-run/prompts/phase4-implementer.md` (v2-flow path) and the legacy inline prompt in `skills/autospec-run/SKILL.md` (so the gate ships to all in-flight issues, not only `autospec:v2-flow`-labeled ones).

**Behavior**: Immediately before `gh pr merge --admin --squash`, execute:

```bash
attempt=0
while [ "$attempt" -lt 3 ]; do
    behind=$(gh pr view <PR> --json mergeStateStatus --jq .mergeStateStatus)
    # mergeStateStatus values: CLEAN | BEHIND | BLOCKED | DIRTY | HAS_HOOKS | UNKNOWN | UNSTABLE
    case "$behind" in
        CLEAN|HAS_HOOKS|UNSTABLE) break ;;  # ready to merge
        BEHIND) gh pr update-branch <PR> 2>&1 ;;
        DIRTY) echo "merge conflict — escalating"; gh issue comment <issue> --body "PR #<PR> has merge conflicts; needs human resolution"; exit 1 ;;
        BLOCKED) echo "merge blocked by required check; waiting for CI"; sleep 30 ;;
        *) sleep 15 ;;
    esac
    # Wait for CI to re-run on the updated branch
    until [ "$(gh pr view <PR> --json statusCheckRollup --jq '[.statusCheckRollup[]?.conclusion] | all(. == "SUCCESS" or . == null)')" = "true" ]; do sleep 30; done
    attempt=$((attempt + 1))
done
if [ "$attempt" -ge 3 ]; then
    gh issue comment <issue> --body "PR #<PR>: rebase-and-retest stalled after 3 attempts; main is moving faster than CI completes. Pausing for operator review."
    exit 1
fi
gh pr merge <PR> --admin --squash --delete-branch
```

**Concrete contract**:

1. Use `gh pr view --json mergeStateStatus` as the predicate. `BEHIND` means PR is behind main; `CLEAN`, `HAS_HOOKS`, or `UNSTABLE` mean ready to merge (UNSTABLE = optional checks pending, which autospec already tolerates per `skills/autospec-run/SKILL.md:202`).
2. `gh pr update-branch` is GitHub's first-party rebase mechanism (Web UI "Update branch" button). No local checkout/rebase/force-push dance required.
3. After update-branch, the PR's CI re-triggers automatically. Wait for it to settle.
4. Cap at 3 rebase loops. Three full CI cycles is the upper bound on reasonable wait. On the 4th `BEHIND` state, comment on the issue and exit — the queue has too much churn for this PR to ever merge, and an operator should intervene.
5. `DIRTY` (merge conflict) → comment + exit immediately. Conflicts need human resolution; autospec-run's auto-loop is the wrong tool.

**Why this works for incidents #1 and #3**: After `gh pr update-branch`, the PR's branch contains main's latest state, and CI runs against that combined state. Incident #1's duplicate `:=` would now appear as a `go vet` failure on the second PR's re-CI, blocking the merge before main is corrupted. Incident #3's DOM-text rename would surface as the test failure in the second PR's re-CI for the same reason.

**Lock-step interaction**: Lock-step deps are already checked immediately before `gh pr create` (`phase4-implementer.md:68`). The rebase loop runs after `gh pr create`; if a lock-step dep merged during the rebase window, that's already-merged state being absorbed into this PR via update-branch — desirable. No additional lock-step recheck needed inside the rebase loop.

## Change β — Migration-replay pre-PR hook

**Where**: `skills/autospec-run/prompts/phase4-implementer.md`, inserted into the **Finalize** section before "Run the project's test command."

**Behavior**: If the diff touches a path matching `*migrations/*` or `*migration*`, look for a migration-replay target and run it. If found, it must pass before opening the PR. If absent, log one line and continue (target repo hasn't opted in).

**Detection order** (first hit wins):

1. `make migrate-test` if `Makefile` exists and grep finds `^migrate-test:`
2. `npm run migrate:test` if `package.json` exists and `jq -r '.scripts."migrate:test"' package.json` is non-null
3. `bin/migrate-test` if the script exists and is executable
4. `pytest tests/migrations -x` if `tests/migrations/` exists
5. Goose-built-in: if `.goose.yaml` or `goose.yaml` exists and the project follows the Goose convention, run `goose -dir <migrations_dir> up && goose -dir <migrations_dir> reset` (resets between runs) inside a throwaway DB container. (Stretch — target repo must provide the DB; defer if not configured.)

If none match: print `migration-replay: target repo has not opted in (no migrate-test target found); continuing without replay check` and proceed. This is informational only — does not block the PR.

**Why this works for incident #2**: PR C's migration adds `'todo_payout'`. PR D's migration drops + re-adds the same constraint without it. The replay test runs PR C's migration then PR D's migration on a fresh DB, then asserts the final schema (or runs a smoke test against the DB) and catches the regression. The target repo's test suite is the source of truth for "what values must remain in this CHECK constraint." Autospec's contribution is just to invoke the test before opening the PR.

**Convention, not code**: The target repo opts in by providing one of the 4 replay-target conventions. autospec is not opinionated about how the test is implemented — just that it exists and exits non-zero on regression.

## Change γ — Target-repo setup guide

**Where**: New file `docs/target-repo-setup.md`. Linked from `README.md` in a new "Target repo setup" section under Install.

**Content** (outline; full text written in implementation):

1. **Required branch protection on main**:
   - Enable "Require status checks to pass before merging."
   - Enable "Require branches to be up to date before merging." (Force GitHub to refuse merges of stale branches even if `--admin` is used. Belt-and-suspenders alongside change α.)
   - List the required status check names.
2. **Migration-replay test convention**:
   - One of: `make migrate-test`, `npm run migrate:test`, `bin/migrate-test`, or `pytest tests/migrations`.
   - Test must exit non-zero on regression.
   - Test must run all migrations on a fresh DB (not against a pre-migrated state).
   - Recommended skeleton for Postgres/Goose/Alembic/Knex.
3. **Why this matters** — link to issue #307 + this design.
4. **Verification** — a one-liner the operator can paste to confirm both gates are wired:

```bash
# Branch protection check
gh api repos/<owner>/<repo>/branches/main/protection --jq '.required_status_checks.strict, .required_status_checks.contexts' \
  && echo "branch protection OK"

# Migration-replay target check
make -n migrate-test 2>/dev/null || npm run --silent migrate:test --dry-run 2>/dev/null || test -x bin/migrate-test \
  && echo "migration-replay target detected"
```

## Error handling

| Failure | Behavior |
|---|---|
| `gh pr update-branch` returns conflict | Comment on issue, exit non-zero. Operator resolves. |
| CI never goes green after 3 rebase cycles | Comment on issue with the rebase count and PR number, exit. Operator decides. |
| Migration-replay test fails | Do NOT open the PR. Post the failure output as a comment on the issue and exit. Implementer must fix the migration before re-attempting. |
| No replay target found | Log one line, continue. Not an error. |
| `mergeStateStatus` returns `UNKNOWN` | Sleep 15s and re-query (GitHub sometimes lags). Counts as one cycle of the 3-attempt cap. |

## Testing

Three test surfaces, one per change:

1. **Change α** — integration: a `tests/phase4/test_rebase_loop.sh` fixture sets up a fake `gh` shim that returns `BEHIND` on the first call and `CLEAN` on the second, and asserts the implementer prompt section (when interpreted as a script) correctly loops and reaches `gh pr merge` exactly once. (Stub-based; the real loop is LLM-executed, so the test verifies the prompt produces correct shell flow when followed literally.)
2. **Change β** — unit: a `tests/phase4/test_migration_replay_detect.sh` fixture creates fake target repos with each of the 4 detection conventions and asserts the detection order is correct.
3. **Change γ** — content: a `tests/docs/test_target_repo_setup_guide.sh` asserts `docs/target-repo-setup.md` exists, references issue #307, and includes the verification one-liner.

All tests run under existing `bash` patterns; no new test infrastructure needed.

## Decomposition preview

The Phase 3 decomposer will split this into ~4 issues:

1. **EPIC umbrella** — tracker for changes α + β + γ.
2. **Change α** — rebase-and-retest pre-merge gate. Touches `skills/autospec-run/prompts/phase4-implementer.md` and `skills/autospec-run/SKILL.md` (legacy path), lock-step synced to `codex/prompt.md` + `opencode/agent.md`. ~80 lines of prompt prose + new test file. `ctx:64k`, `reasoning:medium`.
3. **Change β** — migration-replay pre-PR hook. Touches `skills/autospec-run/prompts/phase4-implementer.md` only (Finalize section). ~30 lines of prose + new test file. `ctx:32k`, `reasoning:medium`.
4. **Change γ** — target-repo setup guide. New file `docs/target-repo-setup.md`, README link, new content test. `ctx:32k`, `reasoning:shallow`.

Dependency edges: γ has no deps; α depends on nothing autospec-side but the migration-replay convention from β should be documented alongside it in γ, so γ depends on β (so γ can cite β's chosen convention strings verbatim). α has no deps.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| `gh pr update-branch` is rate-limited or slow under contention | 3-attempt cap with comment-and-exit prevents infinite loops; operator gets visibility. |
| Target repo doesn't enable branch protection → `--admin` still merges stale | Change α's loop runs regardless of branch protection; it's autospec's own gate. Branch protection is belt-and-suspenders, not a dependency. |
| Migration-replay convention proliferates (every project has a slightly different test name) | 4-target detection covers the common cases; falling back to "no replay target found" is informational, not blocking. Document the convention so new target repos adopt it. |
| The 3-attempt cap is too low in fast-churn target repos | Make the cap configurable via `AUTOSPEC_REBASE_MAX_ATTEMPTS=N` env var (default 3). One-line change. |
| Lock-step deps merge during rebase loop and break unrelated invariants | Lock-step is structural (issue-level), not file-level. A lock-step dep merging into main is the desired state — that's exactly what rebase-and-retest absorbs. |

## Open questions for the implementation plan

- Exact regex for the migration-path detection (`*migrations/*` vs `**/migrations/**` glob).
- Whether `mergeStateStatus: UNSTABLE` should bypass the loop or still trigger one rebase attempt (UNSTABLE means optional checks pending; autospec tolerates these today).
- Whether the legacy implementer prompt (non-v2-flow path) gets the change α treatment in this design or in a separate follow-up. Recommended: bundle it here so the gate ships to all in-flight issues, not just v2-flow ones.

## Next step

After spec lands via PR: invoke Phase 3 (decompose into linked GitHub issues) per `/autospec-define`'s normal pipeline. Expected output: 1 EPIC + 3 children, all labeled `autospec:v2-flow`.
