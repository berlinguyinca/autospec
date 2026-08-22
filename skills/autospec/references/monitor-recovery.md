# autospec monitor-recovery reference

Read this file when the Phase 4 monitor reaches the reviewer verdict, the
reuse-BLOCK refute pass, or Steps 8–11 (SUCCESS/FAILURE/Cleanup/Report). It
holds the cold-tail procedures that the SKILL.md body points to: the reviewer
lens (data-scope invariant, regression gap-check, hard limit, simplicity axis),
the reuse-BLOCK refute pass, verdict handling, Step 8 SUCCESS (full-suite gate,
PR-size final merge, guarded merge, parent reconcile), Step 9 FAILURE, Step 10
Cleanup, Step 11 Report, and the monitor hard rules.

>        > 8a. **data-scope invariant lens (diagnostic/filter endpoints):** When the issue touches endpoints, dashboards, reports, or diagnostics that accept optional job/sample/filter parameters, verify filters never widen to unrelated records. empty optional filters reject unless documented as a deliberate all-records mode. Require concrete evidence for `job-only`, `sample-only`, `job+sample`, `unsupported-filter`, and `empty-filter` paths; unsupported-filter and empty-filter cases must prove rejection or a documented scoped response, not silent broadening.
>        > 9. **Regression gap-check (MANDATORY for `regression`/`priority:high` issues; skip otherwise):** ask "would the reviewer have caught the original gap?" If the fused review as written would NOT have caught the gap this regression closes, add the missing checklist item(s) to `reports/autospec-review/reviewer-lessons.md` (one entry per item, with parent `gap_id` and date) and apply those new checks to this diff before issuing the verdict. This folds the former second-pass regression meta-review into this single reviewer pass — the reviewer-lessons write-path is preserved here; there is no second Tier-A dispatch.
>        >
>        > **Hard limit:** max **25 tool calls total** (Parts 1 + 2 combined). If budget exhausted, append `RULE_ID:OUT_OF_SCOPE: reviewer budget exhausted; PR needs human review` and proceed to verdict.
>        >
>        > **Simplicity axis is ADVISE-only (anti-gold-plating):** the reuse / build-vs-buy / "how could this be better?" axis may argue only toward *less* code — reuse a named existing util (`scripts/lib/`, repo source), adopt a named library, or delete an unneeded abstraction — and only when tied to a named acceptance criterion. It may NEVER emit a `BLOCK` that demands *more* code, a new abstraction, or speculative generality; such suggestions are at most `ADVISE` and never halt the commit. Every reuse verdict must name the matched util or library (evidence-bound), never assert a match from belief.
>        >
>        > **Verdict:** If Part 1 has ZERO blocking findings (INFO lines OK) AND Part 2 has no findings: return ONLY the token: `LGTM`. Otherwise return a numbered findings list — RULE_ID findings first, then LGTM findings. A reuse `BLOCK` is provisional until it survives the refute pass below.
>
>      **Reuse-BLOCK refute pass (before consuming the verdict):** If the findings list contains a build-vs-buy / reuse `BLOCK`, do NOT halt on it yet. Dispatch a **cheap refute pass** — one short `TIER_B` second voter (≤5 tool calls) whose only job is to *kill* the BLOCK: `rg`-search the repo for the named util/library and confirm the claimed reuse target actually exists, is reachable, and fits this call site. **Majority rules:** keep the BLOCK only if the refuter also upholds it; if the refuter refutes it (the named target is absent, unreachable, or ill-fitting), demote that BLOCK to `ADVISE`, drop it from the blocking findings, and continue. If demotion leaves no remaining blocking findings, treat the verdict as `LGTM`. This keeps a hallucinated "library exists" from stalling the merge (`feedback_llm_validator_adaptive_retry`). **Record the outcome of this reuse `BLOCK` decision to the reuse-lens ledger HERE** (issue #1442) — at the decision point, so precision = upheld ÷ total is computed only over real reuse BLOCKs and never from phantom rows on clean passes. Set `_reuse_block_raised=1`, `_reuse_trigger` to the flagged RULE_ID, and `_reuse_upheld=true` when the refuter upheld the BLOCK or `_reuse_upheld=false` when it was refuted/demoted-to-ADVISE, then:
>      **Draw the refuter from a different vendor than the proposer.** Two dispatches to the same model family share failure modes and tend to be wrong together, which is the one case this second vote exists to catch — so resolve the refuter's vendor before dispatching it, passing the harness you detected in step 1 of Phase 0 as `--proposer`:
>        ```bash
>        REFUTE_VENDOR=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/verify-voter-vendor.sh" \
>          --proposer "<HARNESS>") || REFUTE_VENDOR=
>        ```
>        Run the refute pass on `$REFUTE_VENDOR`'s own `TIER_B` — vendor is the independence lever, tier is the quality lever, and this changes only the former. An **empty** `REFUTE_VENDOR` (exit 3: single-harness host, or every alternative already failed over) means keep the same-harness `TIER_B` refuter: a same-vendor second vote is weaker than a cross-vendor one but still better than none, and the script refuses to name the proposer's own vendor rather than report an independence the host cannot provide.
>        **On a quota failure, re-resolve rather than give up.** If the refuter's dispatch fails with a 429 / quota / capacity error, call the script again adding `--unavailable <that vendor>` and dispatch to what it returns; a 429 is the only ground-truth quota signal available, since `usage-observe.sh` reports `observable=false` for all three harnesses. Repeat until it exits 3, then fall back to the same-harness refuter.
>        ```bash
>        if [ "${AUTOSPEC_REUSE_LENS:-}" = "1" ] && [ "${_reuse_block_raised:-0}" = "1" ]; then
>          bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/interrogation-ledger.sh" record \
>            --issue "<ISSUE>" --pr "<PR>" --trigger "${_reuse_trigger:-REINVENT_REPO_UTIL}" \
>            --verdict BLOCK --upheld "${_reuse_upheld:-true}" \
>          || true  # write failure is best-effort; never blocks the PR
>        fi
>        ```
>
>      If `LGTM` && det_exit == 0:
>        gh pr comment <PR> --body "<!-- guardian-block --> Review: clean. <!-- /-->"
>        run **Full test suite gate** and record the exact full-suite command and passing output summary
>        bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ci-wait.sh" <PR>  # fire-and-forget sentinel
>        if [ -f ".autospec/tokens-<ISSUE>-reviewer.json" ]; then
>          bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/record-telemetry.sh" \
>            --dispatch-id "<DISPATCH_ID>-reviewer" --role reviewer --issue "<ISSUE>" \
>            --tokens-json ".autospec/tokens-<ISSUE>-reviewer.json"
>        fi
>        # Reuse-lens verdict is recorded at the refute-pass decision point above
>        # (issue #1442), not here — recording in this LGTM-only branch produced
>        # phantom BLOCK rows on clean passes and never recorded upheld BLOCKs.
>        # monitor exits to parking state HERE — orchestrator relaunches when ~/.autospec/ci-state/<PR>.signal settles
>        # On relaunch: run ci-wait-poll.sh <PR>; break SUCCESS if exit 0 (pass)
>        break SUCCESS only if the full suite passed and required checks pass.
>      If `LGTM` but det_exit != 0:
>        Treat deterministic findings as blocking. Comment, fix, recommit, push. Continue inner loop.
>      If findings list:
>        gh pr comment <PR> --edit-last --body "<!-- guardian-block:begin -->\n## Review findings (iter <K>/3)\n<findings>\n<!-- guardian-block:end -->"
>        Append findings to implementer retry context. Continue inner loop (counts toward 3-iter cap).
>      On 3-iter exhaustion with non-LGTM:
>        gh label create guardian-blocked --color e11d21 --force --repo {repo}
>        gh issue edit <ISSUE> --add-label guardian-blocked
>        Run failure cleanup (comment, swap label, close PR).
>        rm -f /tmp/guardian-<PR>.md
>      <!-- guardian-block:end -->
>    - **Regression coverage** for `regression`/`priority:high` issues is handled inside the single fused reviewer brief (Part 2 item 9 above): the reviewer self-asks "would the reviewer have caught the original gap?", writes any missing checks to `reports/autospec-review/reviewer-lessons.md`, and applies them before its verdict. No second Tier-A dispatch.
>    - If LGTM: break SUCCESS.
>    ```bash
>    # Stop-sentinel: abort if an immediate stop flag is present after this step.
>    if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop-check.sh" "$ISSUE" "$BRANCH" "$LAST_STEP"; then
>      exit 0
>    fi
>    ```
> 8. SUCCESS: Run the **Full test suite gate** one final time after the PR branch is current with `main`; if it fails, fix the failure, recommit, push, rerun the full suite and review, and do NOT merge. Revalidate PR size against GitHub's live OIDs, then guarded-merge only that validated head. Merge auto-closes the issue. After the merge succeeds, reconcile its linked parent. A reconciliation failure cannot undo the completed merge, so post the failure on the child and leave the parent as `complete but stale` for operator-visible retry.
>    ```bash
>    <!-- pr-size-final-merge:begin -->
>    # Query GitHub after update-branch, review, and final local proof. The live PR
>    # endpoints are authoritative; stale local OIDs can never create acceptance.
>    # If the shell boundary discarded the helper, redefine it exactly as in step 4a.
>    # pr-size-final-merge-exec:begin
>    PR_SIZE_REMOTE_OIDS=$(gh pr view <PR> --json baseRefOid,headRefOid \
>      --jq '[.baseRefOid, .headRefOid] | @tsv') || exit 1
>    [ "$(printf '%s\n' "$PR_SIZE_REMOTE_OIDS" | awk -F '\t' \
>      'NF == 2 && $1 != "" && $2 != "" { print "valid" }')" = "valid" ] || exit 1
>    PR_SIZE_BASE_OID=$(printf '%s\n' "$PR_SIZE_REMOTE_OIDS" | cut -f1)
>    PR_SIZE_HEAD_OID=$(printf '%s\n' "$PR_SIZE_REMOTE_OIDS" | cut -f2)
>    git fetch --no-tags origin \
>      "+refs/heads/${AUTOSPEC_BASE_BRANCH:-main}:refs/remotes/origin/${AUTOSPEC_BASE_BRANCH:-main}" \
>      "+refs/heads/<BRANCH>:refs/remotes/origin/<BRANCH>" || exit 1
>    git cat-file -e "${PR_SIZE_BASE_OID}^{commit}" || exit 1
>    git cat-file -e "${PR_SIZE_HEAD_OID}^{commit}" || exit 1
>    [ "$(git rev-parse "origin/${AUTOSPEC_BASE_BRANCH:-main}")" = "$PR_SIZE_BASE_OID" ] || exit 1
>    [ "$(git rev-parse "origin/<BRANCH>")" = "$PR_SIZE_HEAD_OID" ] || exit 1
>    [ "$(git rev-parse HEAD)" = "$PR_SIZE_HEAD_OID" ] || exit 1
>    PR_SIZE_PHASE=final-pre-merge
>    PR_SIZE_EVIDENCE=$(run_pr_size_gate) || {
>      printf '%s\n' "$PR_SIZE_EVIDENCE"
>      exit 1
>    }
>    printf '%s\n' "$PR_SIZE_EVIDENCE"
>    # Reviewer evidence is accepted only as this complete line; prefixes,
>    # suffixes, summaries, and inferred approval are not acceptance.
>    printf '%s\n' "$PR_SIZE_EVIDENCE" | grep -qxF 'INFO:PR_SIZE: acceptance' || exit 1
>    # pr-size-final-merge-exec:end
>    <!-- pr-size-final-merge:end -->
>    # pr-size-guarded-merge-exec:begin
>    run_guarded_pr_size_merge() {
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-guarded-merge.sh" \
>        --pr <PR> --repo {repo} \
>        --merge-args "--admin --squash --delete-branch --match-head-commit $PR_SIZE_HEAD_OID"
>    }
>    # pr-size-guarded-merge-exec:end
>    run_guarded_pr_size_merge || exit 1
>    _parent_slug=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" --canonical "{repo}")
>    export AUTOSPEC_PARENT_STATE_ROOT="${AUTOSPEC_PARENT_STATE_ROOT:-$HOME/.autospec/parent-state/$_parent_slug}"
>    if ! "${AUTOSPEC_BIN:-autospec}" parent reconcile-child --repo {repo} --child "<ISSUE>"; then
>      gh issue comment "<ISSUE>" --repo {repo} --body "Parent reconciliation failed after merge; remote parent state is unknown and will be retried by the recurring parent sweep."
>    fi
>    ```
>    ```bash
>    # Stop-sentinel: abort if an immediate stop flag is present after this step.
>    if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop-check.sh" "$ISSUE" "$BRANCH" "$LAST_STEP"; then
>      exit 0
>    fi
>    ```
>    <!-- token-report:begin -->
>    Post the per-issue token report (best-effort; never fails the run):
>    ```bash
>    # Orchestrator writes .autospec/tokens-<ISSUE>.json from Agent-result usage
>    # (harness-dependent, best-effort; absent fields → null, never blocking).
>    bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/post-token-report.sh" \
>      --issue "<ISSUE>" --repo "<REPO>" \
>      --tokens-json ".autospec/tokens-<ISSUE>.json" || true
>    ```
>    <!-- token-report:end -->
>    # Cleanup single-fetch body temp file on terminal success (D5).
>    rm -f "/tmp/issue-<ISSUE>-body.md" || true
> 9. FAILURE (loop exhausted): comment failure on issue, swap label `in-progress-by-bot` → `auto-implement`, `gh pr close <PR> --delete-branch`.
>    Cleanup single-fetch body temp file on terminal failure: `rm -f "/tmp/issue-<ISSUE>-body.md" || true`
> 10. Cleanup: run `autospec runtime env down --repo /tmp/wt-<BRANCH> --mode "${AUTOSPEC_RUNTIME_MODE:-auto}" --purge-maven`; then run `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-runtime-worktree-cleanup.sh" /tmp/wt-<BRANCH>`; only after both succeed, run `cd / && git -C {repo_root} worktree remove /tmp/wt-<BRANCH> --force`.
> 11. Report: PR number, outcome, one-paragraph summary.
>
> Hard rules: NEVER push to main, force-push, bypass hooks, or touch the umbrella issue. gh CLI only.
> ```
>
> Hard rules for the monitor: ONE issue at a time, sequential. Do NOT touch the umbrella. On transient gh errors retry once. Do NOT ask the user — auto-merge authority is granted in AGENTS.md.
>
> Final output when shutdown: numbered list of every processed issue with PR # and outcome.

