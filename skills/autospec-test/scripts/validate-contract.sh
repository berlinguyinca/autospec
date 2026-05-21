#!/usr/bin/env bash
# validate-contract.sh — validate a resolved autospec-test contract JSON.
#
# Usage:  validate_contract <json_file> [<schema_file>]
#   OR:   echo '{"mode":"strict_isolation",...}' | validate_contract - [<schema_file>]
#
# Exit codes:
#   0 = valid contract
#   1 = fatal error (missing tool, unreadable file, internal error)
#   2 = refuse-to-run: contract is invalid or missing required fields (operator-actionable)
#
# Validation steps:
#   1. JSON Schema validation via `ajv validate`
#   2. Higher-level rules not expressible in JSON Schema:
#      a. Mode II conjunction: scoped_production requires i_understand_this_writes_to_production=true
#         AND e2e.backup.driver AND e2e.backup.restore_cmd
#      b. Fail-closed rule: e2e.forbidden_url_patterns=[] without
#         e2e.forbidden_url_patterns_intentionally_empty=true → exit 2

set -eu

# Resolve schema path
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
DEFAULT_SCHEMA="$REPO_ROOT/schemas/autospec-test-contract.schema.json"

validate_contract() {
    local input_file="${1:--}"
    local schema_file="${2:-$DEFAULT_SCHEMA}"

    # ── Dependency checks ──────────────────────────────────────────────────────
    if ! command -v ajv >/dev/null 2>&1; then
        printf 'validate-contract: fatal: ajv CLI not found. Install with: npm install -g ajv-cli\n' >&2
        exit 1
    fi
    if ! command -v jq >/dev/null 2>&1; then
        printf 'validate-contract: fatal: jq not found. Install with: brew install jq\n' >&2
        exit 1
    fi
    if [ ! -f "$schema_file" ]; then
        printf 'validate-contract: fatal: schema not found: %s\n' "$schema_file" >&2
        exit 1
    fi

    # ── Read input ─────────────────────────────────────────────────────────────
    local json_data
    if [ "$input_file" = "-" ]; then
        json_data=$(cat)
    else
        if [ ! -f "$input_file" ]; then
            printf 'validate-contract: fatal: input file not found: %s\n' "$input_file" >&2
            exit 1
        fi
        json_data=$(cat "$input_file")
    fi

    # Write to temp file for ajv (which needs a file argument)
    local tmpfile
    tmpfile=$(mktemp /tmp/autospec-test-contract-XXXXXX.json)
    # shellcheck disable=SC2064
    trap "rm -f '$tmpfile'" EXIT
    printf '%s' "$json_data" > "$tmpfile"

    # ── Step 1: JSON Schema validation ────────────────────────────────────────
    local ajv_out
    if ! ajv_out=$(ajv validate -s "$schema_file" -d "$tmpfile" --spec=draft2020 2>&1); then
        printf 'validate-contract: schema validation failed:\n%s\n' "$ajv_out" >&2
        exit 2
    fi

    # ── Step 2a: Mode II conjunction check ────────────────────────────────────
    local mode
    mode=$(printf '%s' "$json_data" | jq -r '.mode // "strict_isolation"')
    if [ "$mode" = "scoped_production" ]; then
        # Check i_understand_this_writes_to_production
        local ack
        ack=$(printf '%s' "$json_data" | jq -r '.i_understand_this_writes_to_production // false')
        if [ "$ack" != "true" ]; then
            printf 'validate-contract: refuse-to-run: mode=scoped_production requires i_understand_this_writes_to_production=true\n' >&2
            exit 2
        fi

        # Check backup.driver
        local backup_driver
        backup_driver=$(printf '%s' "$json_data" | jq -r '.e2e.backup.driver // empty')
        if [ -z "$backup_driver" ]; then
            printf 'validate-contract: refuse-to-run: mode=scoped_production requires e2e.backup.driver (backup configuration missing)\n' >&2
            exit 2
        fi

        # Check backup.restore_cmd
        local restore_cmd
        restore_cmd=$(printf '%s' "$json_data" | jq -r '.e2e.backup.restore_cmd // empty')
        if [ -z "$restore_cmd" ]; then
            printf 'validate-contract: refuse-to-run: mode=scoped_production requires e2e.backup.restore_cmd\n' >&2
            exit 2
        fi
    fi

    # ── Step 2b: Fail-closed forbidden_url_patterns rule ─────────────────────
    local forbidden_count
    forbidden_count=$(printf '%s' "$json_data" | jq '.e2e.forbidden_url_patterns | length // 0')
    if [ "$forbidden_count" = "0" ]; then
        # Check for explicit ack
        local ack_empty
        ack_empty=$(printf '%s' "$json_data" | jq -r '.e2e.forbidden_url_patterns_intentionally_empty // false')
        if [ "$ack_empty" != "true" ]; then
            printf 'validate-contract: refuse-to-run: e2e.forbidden_url_patterns is empty or missing without explicit ack. Set forbidden_url_patterns to at least one pattern, or set forbidden_url_patterns_intentionally_empty=true to acknowledge no URL restrictions apply.\n' >&2
            exit 2
        fi
    fi

    # ── All checks passed ──────────────────────────────────────────────────────
    exit 0
}

validate_contract "${1:--}" "${2:-}"
