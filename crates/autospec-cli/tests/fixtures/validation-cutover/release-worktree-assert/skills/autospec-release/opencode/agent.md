## Worktree guard (commit gate)
worktree-guard.sh assert MUST exit 0 primary checkout
git worktree remove
git worktree prune
<!-- worktree-assert:begin -->
identical block
<!-- worktree-assert:end -->
