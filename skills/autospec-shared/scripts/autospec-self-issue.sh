#!/usr/bin/env bash
set -u
finding= repo="berlinguyinca/autospec" cache="${AUTOSPEC_SELF_ISSUE_CACHE:-.autospec/self-issue-cache}" dry=0 window="${AUTOSPEC_SELF_ISSUE_WINDOW_SEC:-86400}"
while [ $# -gt 0 ]; do
  case "$1" in
    --finding) [ $# -ge 2 ] || exit 2; finding=$2; shift 2 ;;
    --repo) [ $# -ge 2 ] || exit 2; repo=$2; shift 2 ;;
    --cache) [ $# -ge 2 ] || exit 2; cache=$2; shift 2 ;;
    --window) [ $# -ge 2 ] || exit 2; window=$2; shift 2 ;;
    --dry-run) dry=1; shift ;;
    --help|-h) echo 'Usage: autospec-self-issue.sh --finding JSON [--repo OWNER/REPO] [--dry-run] [--cache PATH]'; exit 0 ;;
    *) echo "autospec-self-issue: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$finding" ] || { echo 'autospec-self-issue: --finding required' >&2; exit 2; }
printf '%s' "$finding" | jq -e . >/dev/null 2>&1 || { echo 'autospec-self-issue: invalid JSON' >&2; exit 2; }
category=$(printf '%s' "$finding" | jq -r '.category // "other"')
summary=$(printf '%s' "$finding" | jq -r '.summary // "unspecified failure"')
evidence=$(printf '%s' "$finding" | jq -r '.evidence // "no evidence recorded"')
normalized=$(printf '%s' "$summary" | tr '[:upper:]' '[:lower:]' | tr -s '[:space:]' ' ' | sed 's/^ //;s/ $//')
key=$(printf '%s|%s' "$category" "$normalized" | sha256sum | awk '{print $1}')
now=$(date +%s)
mkdir -p "$(dirname "$cache")" 2>/dev/null || true
if [ -f "$cache" ]; then
  while IFS='|' read -r timestamp oldkey; do
    [ -n "$timestamp" ] && [ "$oldkey" = "$key" ] && [ $((now - timestamp)) -lt "$window" ] && { echo "autospec-self-issue: dedup-skip $key" >&2; exit 1; }
  done < "$cache"
fi
title="autospec: ${category} — ${summary}"
body=$(printf '%s\n' '## Category' '' "$category" '' '## Summary' '' "$summary" '' '## Evidence' '' "$evidence" '' 'Filed automatically by autospec-self-issue.sh.')
if [ "$dry" = 1 ]; then printf 'TITLE: %s\nREPO: %s\n\n%s\n' "$title" "$repo" "$body"; printf '%s|%s\n' "$now" "$key" >> "$cache"; exit 0; fi
if ! command -v gh >/dev/null 2>&1; then echo 'autospec-self-issue: gh unavailable; refusing to prompt' >&2; exit 2; fi
url=$(gh issue create --repo "$repo" --title "$title" --body "$body" --label auto-implement --label origin:self 2>/dev/null) || { echo 'autospec-self-issue: gh issue create failed' >&2; exit 2; }
bash "$(cd "$(dirname "$0")" && pwd)/project-sync-issue.sh" "$url" "$PWD"
printf '%s|%s\n' "$now" "$key" >> "$cache"
printf '%s\n' "$url"
