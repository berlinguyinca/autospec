#!/usr/bin/env bash
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

failures=0

fail() {
  failures=$((failures + 1))
  printf 'v60-release: FAIL: %s\n' "$*"
}

require_file() {
  [ -f "$1" ] || fail "missing $1"
}

require_text() {
  local path="$1"
  local pattern="$2"
  if [ ! -f "$path" ] || ! grep -q "$pattern" "$path"; then
    fail "$path missing '$pattern'"
  fi
}

require_file ".autospec/releases/v60.md"
require_file "docs/reports/v60-final-report.md"
require_file "scripts/validate-launch-readiness.sh"
require_text ".autospec/releases/v60.md" "AUTOSPEC_V60_RELEASE_READY=true"
require_text "docs/reports/v60-final-report.md" "V61 Readiness Criteria"

if [ "$failures" -ne 0 ]; then
  printf 'v60-release: %s failure(s)\n' "$failures"
  exit 1
fi

printf 'AUTOSPEC_V60_RELEASE_READY=true\n'

