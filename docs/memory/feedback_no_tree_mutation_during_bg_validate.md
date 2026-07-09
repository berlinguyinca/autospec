---
name: feedback_no_tree_mutation_during_bg_validate
description: Never switch/delete branches while a background validate.sh (or any tree-reading test) runs; it corrupts the checkout mid-run and yields false "required file missing" failures
metadata:
  type: feedback
---

Switching branches or deleting branches while a **background** `validate.sh`
(or any test that reads the working tree) is still running mutates the
checkout underneath the running job, producing false failures such as
`FAIL — scripts/autospec-constitution-rules.py: required file missing`. Seen
live: launched the authoritative `validate.sh` on a rebased branch, then
`git checkout launch/...` + `git branch -D` for cleanup while it ran; the
launch branch lacked a file `main` has, so mid-run the file "vanished" and
validate failed on merged code that was actually clean.

**Why:** a background job shares the one working tree; git operations that
rewrite the tree race against it.

**How to apply:** run tree-reading background jobs in a **dedicated detached
worktree** (`git worktree add --detach <tmp> origin/main`) that nothing else
touches, and remove it after. Do NOT `git checkout`/`branch -D`/`stash pop`
in the primary checkout until the background job exits. Compounds with
[[feedback_background_pipeline_exit_masking]]: also never trust the
background task's reported exit when the command ends in `; echo ...` — that
reports the echo's exit, not the gate's; make `validate.sh` the SOLE command
(so the reported code is its own) or read the gate's final `OK/FAIL` status
line, and confirm BOTH the exit code and the status line before claiming pass.
