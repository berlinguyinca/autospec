#!/usr/bin/env bash
set -eu

[ "${1:-} ${2:-}" = "project sync" ] || {
  printf 'unexpected autospec invocation: %s\n' "$*" >&2
  exit 64
}
shift 2
repo_dir=
issue_url=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo-dir) repo_dir="${2:-}"; shift 2 ;;
    --issue-url) issue_url="${2:-}"; shift 2 ;;
    *) printf 'unexpected project sync argument: %s\n' "$1" >&2; exit 64 ;;
  esac
done
[ -n "$repo_dir" ] && [ -n "$issue_url" ] || {
  echo 'project sync requires --repo-dir and --issue-url' >&2
  exit 64
}
