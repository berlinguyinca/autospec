#!/usr/bin/env bash
# Reconcile an already-created product issue into its managed GitHub Project.
set -u

issue_url="${1:-}"
repo_dir="${2:-$PWD}"
[ -n "$issue_url" ] || { echo "project-sync-issue: issue URL required" >&2; exit 2; }
[ "${AUTOSPEC_DRY_RUN:-0}" = "1" ] && exit 0

if ! "${AUTOSPEC_BIN:-autospec}" project sync --repo-dir "$repo_dir" --issue-url "$issue_url" >/dev/null; then
  echo "WARNING: managed Project sync failed for $issue_url; durable projection remains retryable" >&2
fi
exit 0
