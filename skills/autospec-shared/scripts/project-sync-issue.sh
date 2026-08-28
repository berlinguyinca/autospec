#!/usr/bin/env bash
# Reconcile an already-created product issue into its managed GitHub Project.
set -u

issue_url="${1:-}"
repo_dir="${2:-$PWD}"
[ -n "$issue_url" ] || { echo "project-sync-issue: issue URL required" >&2; exit 2; }
[ "${AUTOSPEC_DRY_RUN:-0}" = "1" ] && exit 0

diagnostic="$(mktemp "${TMPDIR:-/tmp}/autospec-project-sync.XXXXXX")" || exit 2
trap 'rm -f "$diagnostic"' EXIT INT TERM
if "${AUTOSPEC_BIN:-autospec}" project sync --repo-dir "$repo_dir" --issue-url "$issue_url" >/dev/null 2>"$diagnostic"; then
  exit 0
fi
if grep -Fq 'journaled_projection_pending:' "$diagnostic"; then
  echo "WARNING: managed Project sync failed for $issue_url; durable projection remains retryable" >&2
  exit 0
fi
cat "$diagnostic" >&2
echo "ERROR: managed Project sync failed before durable journaling for $issue_url" >&2
exit 1
