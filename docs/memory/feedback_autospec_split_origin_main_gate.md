---
name: autospec-split-origin-main-gate
description: "/autospec-split halts at Phase 0.5 if the selected spec isn't on origin/main; check git status vs origin before invoking to avoid a mid-pipeline PR detour"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 37f5bea4-90df-4cf3-8298-d8158b15d2ca
---

`/autospec-split` requires the selected `docs/specs/*.md` file to exist on `origin/main` before Phase 3 dispatches — it verifies via `git cat-file -e origin/main:<path>` and stops with the "Selected spec is not on origin/main yet" message if missing.

**Why:** Phase 3 child issues cite the spec by stable GitHub URL (`https://github.com/{repo}/blob/main/<path>`). A local-only spec produces a 404 link that downstream Phase 4 implementers can't follow. The gate is deliberate.

**How to apply:** Before invoking `/autospec-split <path>`, run `git fetch origin && git cat-file -e origin/main:<spec-path> 2>&1` (or just `git log --oneline origin/main..HEAD` to see what's unpushed). If the spec is local-only, decide in advance:
- **Solo repo + low-risk spec:** `git push origin main` directly.
- **Shared repo or admin-merge available:** branch + PR + `gh pr merge --admin --squash --delete-branch`.

This avoids the mid-pipeline detour observed 2026-05-31 (umbrella #740 work) where /autospec-split bounced out, requiring a manual PR #739 + admin-merge + re-invocation. The Phase 2 protocol inside `/autospec` and `/autospec-define` handles spec-landing automatically; only `/autospec-split` assumes pre-landed specs.

Related: [[admin-merge-denial]] — admin-merge requires settings.json permission rule, not just AskUserQuestion approval.
