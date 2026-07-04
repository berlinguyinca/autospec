#!/usr/bin/env bash
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

failures=0

fail() {
  failures=$((failures + 1))
  printf 'public-launch: FAIL: %s\n' "$*"
}

run_gate() {
  local label="$1"
  shift
  if "$@" >/tmp/autospec-public-launch-"$label".log 2>&1; then
    cat /tmp/autospec-public-launch-"$label".log
  else
    cat /tmp/autospec-public-launch-"$label".log
    fail "$label gate failed"
  fi
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

run_gate v25 bash scripts/validate-v25-baseline.sh
run_gate v60 bash scripts/validate-v60-release.sh
run_gate v61 bash scripts/validate-launch-readiness.sh

require_file ".autospec/releases/launch-candidate.md"
require_file ".autospec/reports/final-launch-readiness.md"
require_file ".autospec/handoff/codex-final-handoff.md"
require_file "docs/release-checklist.md"
require_file "docs/public-launch-checklist.md"
require_file "docs/good-first-issues.md"
require_file "docs/assets/screenshots-placeholder.md"
require_file "docs/assets/social-preview-placeholder.md"
require_text ".autospec/releases/launch-candidate.md" "AUTOSPEC_PUBLIC_LAUNCH_READY=true"
require_text "README.md" "Comparison"
require_text "README.md" "Current Maturity And Limitations"
require_text "README.md" "bash scripts/demo-recording.sh"

if [ "$failures" -ne 0 ]; then
  printf 'public-launch: %s failure(s)\n' "$failures"
  exit 1
fi

printf 'AUTOSPEC_PUBLIC_LAUNCH_READY=true\n'
