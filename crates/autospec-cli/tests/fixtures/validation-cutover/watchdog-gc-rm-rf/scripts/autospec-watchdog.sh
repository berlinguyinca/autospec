#!/usr/bin/env bash
set -euo pipefail

gc_orphaned_worktrees() {
  if [ -f "$DISK_HYGIENE_SH" ]; then
    keep="$(bash "$DISK_HYGIENE_SH" keep-reason "$wt" 2>/dev/null || echo status_failed)"
  else
    keep="disk_hygiene_missing"
  fi
  git worktree remove "$1"
}

gc_orphaned_worktrees
rm -rf "$1"
