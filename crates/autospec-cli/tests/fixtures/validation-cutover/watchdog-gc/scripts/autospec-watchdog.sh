#!/usr/bin/env bash
set -euo pipefail

gc_orphaned_worktrees() {
  git worktree remove --force "$1"
}

# rm -rf is intentionally only a comment.
gc_orphaned_worktrees
git -C "$1" log --not --remotes
