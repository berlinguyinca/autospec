<!-- dispatch-implementer: branch_verdict={"state":"fresh","pr":null} -->

**Branch verdict:** `{"state":"fresh","pr":null}`
The orchestrator should act on `state` before proceeding:
- `open-pr`: an open PR already exists — validate + merge the existing PR, no re-implementation.
- `branch-only`: branch exists remotely but no open PR — adopt in this worktree and continue.
- `fresh`: no prior work exists — proceed with new implementation.

**Workdir:** `@@WT@@` (worktree). All `cd`, `git`, `gh`, edit, and
test commands MUST run from this worktree. Do NOT touch the main checkout.
Do NOT `git checkout` other branches. This is parallel-safety isolation
per autospec-run Phase 4 worktree contract (issue #690).

Issue: #@@ISSUE@@
Branch: @@BRANCH@@

---

IMPLEMENTER_PROMPT_BODY
