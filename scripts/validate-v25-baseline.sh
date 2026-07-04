#!/usr/bin/env bash
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

failures=0

fail() {
  failures=$((failures + 1))
  printf 'v25-baseline: FAIL: %s\n' "$*"
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

require_file ".autospec/baselines/v25-baseline.json"
require_file ".autospec/releases/v25.md"
require_file ".autospec/reports/repository-audit.md"
require_file ".autospec/reports/spec-inventory.md"
require_file ".autospec/reports/dependency-validation.md"
require_text ".autospec/baselines/v25-baseline.json" "V25_BASELINE_READY=true"
require_text ".autospec/releases/v25.md" "V25_BASELINE_READY=true"

if [ "$failures" -ne 0 ]; then
  printf 'v25-baseline: %s failure(s)\n' "$failures"
  exit 1
fi

printf 'V25_BASELINE_READY=true\n'

