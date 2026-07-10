#!/usr/bin/env bash
# scripts/groom-validate.sh — template-groom validator.
#
# Reuses scripts/lint-issue.sh (or an injected override via
# AUTOSPEC_LINT_ISSUE_BIN) to check whether an LLM-filled issue body passes
# the template contract. On failure, surfaces the linter's findings verbatim
# so the caller can feed them back as directives for the next LLM attempt.
#
# This script owns ONLY validation — the LLM fill itself lives in the
# grooming loop's prose contract, because a bash script cannot spawn a
# subagent.
#
# Usage:
#   scripts/groom-validate.sh <body-file>
#
# Output:
#   rc 0: {"ok":true}
#   rc 1: {"ok":false,"findings":[...captured linter output lines...]}

set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LINT_BIN="${AUTOSPEC_LINT_ISSUE_BIN:-$SCRIPT_DIR/lint-issue.sh}"

BODY_FILE="${1:?usage: groom-validate.sh <body-file>}"

set +e
LINT_OUTPUT="$("$LINT_BIN" "$BODY_FILE" 2>&1)"
RC=$?
set -e

if [ "$RC" -eq 0 ]; then
  printf '{"ok":true}\n'
  exit 0
fi

FINDINGS_JSON="$(printf '%s\n' "$LINT_OUTPUT" | grep -v '^[[:space:]]*$' | jq -R . | jq -s .)"
jq -n --argjson findings "$FINDINGS_JSON" '{ok:false,findings:$findings}'
exit 1
