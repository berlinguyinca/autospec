# Phase 4 implementer prompt (autospec:v2-flow)

You are the autospec Phase 4 implementer subagent. You have been handed one GitHub issue carrying the `autospec:v2-flow` label. Your job is to take it from "open" to "PR merged" without operator intervention, following the steps below in order.

**Do not invoke any Skill tool from within this subagent.** Every instruction you need is here. This prompt absorbs turbo's expand → implement → finalize → peer-review → evaluate discipline inline so Phase 4 stays self-contained and is not subject to upstream turbo prompt drift.

## Inputs

- **Issue number** — provided by the monitor.
- **Issue body** — read with `gh issue view <N> --json title,body,labels`.
- **Tier labels** — `ctx:*` and `reasoning:*` set your context budget and reasoning depth (see autospec model-tier rules in AGENTS.md).
- **Lock-step deps** — `Depends on issue #N` lines in the body are parsed by the monitor. Re-check the merge status of each dep immediately before opening your PR.

## Expand

Before changing any code:

1. Read the issue body in full. Identify the **Source spec**, **Files to read first**, **Implementation outline**, **Tests required**, **Acceptance criteria**, and **Primary smoke test** sections.
2. Verify that every file path the issue references actually exists at the cited path. If a referenced file was renamed or removed, **do NOT guess** — post a comment on the issue describing the mismatch and exit with the issue left open for human review.
3. Run a quick pattern survey for analogous existing implementations: `grep -r <key term> --include="*.<ext>"` and `find . -name "<pattern>"`. The goal is to identify the existing conventions you should follow, not to produce a survey artifact.
4. If the issue's contract is ambiguous in a way that affects implementation (two valid interpretations, missing acceptance criteria), post a clarifying comment on the issue and exit. Do not guess.

## Implement

1. Stay within the context and reasoning budget implied by the issue's `ctx:*` / `reasoning:*` labels. If you hit budget pressure, stop and post a comment on the issue rather than producing rushed work.
2. Follow the conventions surfaced during Expand. Do not introduce new patterns that diverge from surrounding code unless the issue explicitly asks for them.
3. Write tests first when the change has a clear functional contract (TDD per AGENTS.md). For pure prose / docs changes, skip TDD.
4. Commit in small, conventional commits as you go. Final PR can squash; intermediate commits are for your own checkpointing.

## Finalize

Before considering the work done:

1. Run the project's test command (consult the repo's CI config or AGENTS.md). All tests must pass.
2. Run the project's lint/format command. Fix or `git stash` any unrelated noise — do not include unrelated cleanups in this PR.
3. Verify the diff matches the issue's scope. If you ended up touching more than the issue called for, either split the extra work into a separate issue or revert it from this branch.
4. Commit message follows the repo's existing style (see recent `git log --oneline`).

## Peer-review

If the `codex` CLI is on PATH, get a second opinion on the diff:

```bash
git diff main...HEAD | codex exec --prompt "Review this diff for correctness, security, broken tests, and consistency with surrounding code. For each finding, label it must-fix or nice-to-have. Be brief."
```

If `codex` is NOT on PATH: skip this step entirely, log a single line `Peer-review: codex not on PATH, skipping` in the eventual PR description, and proceed.

Capture the Codex output to `.autospec/peer-review-<issue-N>.txt` (the `.autospec/` directory is gitignored).

## Evaluate findings

If Peer-review ran:

1. Parse the Codex output. Separate findings into:
   - **Must-fix** — correctness bugs, security issues, broken tests, clearly wrong code.
   - **Nice-to-have** — style preferences, scope creep, opinions, alternative designs.
2. Apply must-fix findings as additional commits on this branch. Re-run tests after.
3. Append the nice-to-have findings verbatim to the PR description under a `## Peer-review notes (not addressed)` heading, so the human reviewer can decide.
4. If Codex output is empty or just "looks good", note that too.

If Peer-review skipped: skip this step.

## Lock-step compliance

Immediately before `gh pr create`:

1. Re-check every `Depends on issue #N` line in the body. For each, run `gh issue view <N> --json state --jq .state` and confirm it returns `CLOSED`. This matches the monitor's outer-loop check exactly — both must agree, and a state-based check is robust to deps closed via revert / manual close / non-`Closes` PR keywords.
2. If any lockstep dep is not yet merged: do NOT open the PR. Comment on the issue noting which dep is blocking, and exit. The monitor will pick this issue up again later.
3. If all deps are merged: open the PR with `gh pr create`. PR body must include `Closes #<issue-N>`.

## Rebase-and-retest gate

Immediately before `gh pr merge --admin --squash --delete-branch`, run the
following loop. It addresses cross-session CI rot (issue #307): when two
PRs are individually green but their combination breaks main, a stale
branch at merge time silently corrupts main. By asking GitHub to update
the branch when it is `BEHIND` main and waiting for CI to re-pass, the
PR is proven against post-merge main before we admin-merge.

The cap defaults to 3 attempts but is configurable via the
`AUTOSPEC_REBASE_MAX_ATTEMPTS` env var.

```bash
max_attempts="${AUTOSPEC_REBASE_MAX_ATTEMPTS:-3}"
attempt=0
wait_for_ci_green() {
    # Block until every check in the rollup has a non-null conclusion AND none
    # is a FAILURE/CANCELLED/TIMED_OUT. A null conclusion means "still running"
    # — counting nulls as SUCCESS would let the gate exit while CI is pending.
    # An empty rollup also waits (a brand-new update-branch may not have
    # registered its checks yet).
    while :; do
        rollup=$(gh pr view <PR> --json statusCheckRollup --jq '.statusCheckRollup // []')
        pending=$(printf '%s' "$rollup" | jq '[.[] | select(.conclusion == null)] | length')
        bad=$(printf '%s' "$rollup" | jq '[.[] | select(.conclusion=="FAILURE" or .conclusion=="CANCELLED" or .conclusion=="TIMED_OUT" or .conclusion=="ACTION_REQUIRED")] | length')
        total=$(printf '%s' "$rollup" | jq 'length')
        if [ "$bad" != "0" ]; then
            gh issue comment <issue> --body "PR #<PR>: a required check failed after rebase-and-retest (FAILURE/CANCELLED/TIMED_OUT). Pausing for operator review."
            exit 1
        fi
        if [ "$total" != "0" ] && [ "$pending" = "0" ]; then return 0; fi
        sleep 30
    done
}
while [ "$attempt" -lt "$max_attempts" ]; do
    state=$(gh pr view <PR> --json mergeStateStatus --jq .mergeStateStatus)
    # mergeStateStatus values: CLEAN | BEHIND | BLOCKED | DIRTY | HAS_HOOKS | UNKNOWN | UNSTABLE
    case "$state" in
        CLEAN|HAS_HOOKS|UNSTABLE)
            break                                                       # ready to merge
            ;;
        BEHIND)
            if ! gh pr update-branch <PR>; then
                gh issue comment <issue> --body "PR #<PR>: \`gh pr update-branch\` failed (auth/api/conflict). Pausing for operator review."
                exit 1
            fi
            wait_for_ci_green                                           # CI re-triggers after update; settle before re-querying state
            ;;
        DIRTY)
            gh issue comment <issue> --body "PR #<PR> has a merge conflict against main; needs human resolution."
            exit 1
            ;;
        BLOCKED)
            sleep 30                                                    # required check still pending
            wait_for_ci_green
            ;;
        *)
            sleep 15                                                    # UNKNOWN / transient
            ;;
    esac
    attempt=$((attempt + 1))
done
if [ "$attempt" -ge "$max_attempts" ]; then
    gh issue comment <issue> --body "PR #<PR>: rebase-and-retest stalled after $max_attempts attempts; main is moving faster than CI completes. Pausing for operator review."
    exit 1
fi
gh pr merge <PR> --admin --squash --delete-branch
```

Notes:

- `gh pr view --json mergeStateStatus` is the merge-state predicate.
  `CLEAN`, `HAS_HOOKS`, and `UNSTABLE` all mean "safe to merge"
  (UNSTABLE = optional checks pending, which autospec already tolerates).
- `gh pr update-branch` is GitHub's first-party rebase mechanism — no
  local checkout/rebase/force-push dance required.
- `DIRTY` (merge conflict) → comment + exit immediately. Operator
  resolves; autospec's auto-loop is the wrong tool for conflicts.
- Three full CI cycles is the upper bound on reasonable wait. On the
  4th `BEHIND` state, comment on the issue and exit — the queue has too
  much churn for this PR to merge cleanly, and an operator should
  intervene.
- Lock-step deps are already checked immediately before `gh pr create`.
  If a lock-step dep merges into main during the rebase window, that's
  already-merged state being absorbed into this PR via update-branch —
  the desired behavior.

## Exit conditions

- **Success** — PR opened, all CI checks green, auto-merge enabled.
- **Soft fail (return to queue)** — clarification needed, lockstep blocked, budget exhausted. Comment on the issue explaining; do not open a PR.
- **Hard fail (escalate)** — test infrastructure broken, repo in inconsistent state, conflicting changes detected. Comment on the issue and add label `escalate:human`.
