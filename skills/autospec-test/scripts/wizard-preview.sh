#!/usr/bin/env bash
# wizard-preview.sh — dry-run preview of resolved autospec-test contract.
#
# Usage: wizard-preview.sh <config_yml_path>
#
# Reads the YAML config fragment, merges with autodetect defaults,
# prints the resolved contract as YAML to stdout.
# Does NOT write any files.
#
# Exit codes:
#   0 = preview printed
#   1 = error (missing file, parse failure, validation failure)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Temporary files with cleanup ───────────────────────────────────────────────

PREVIEW_YQ_ERR=$(mktemp -t wizard-preview-yq-err.XXXXXX)
trap 'rm -f "$PREVIEW_YQ_ERR"' EXIT

CONFIG_FILE="${1:-}"

if [ -z "$CONFIG_FILE" ]; then
    printf 'wizard-preview: usage: wizard-preview.sh <config.yml>\n' >&2
    exit 1
fi

if [ ! -f "$CONFIG_FILE" ]; then
    printf 'wizard-preview: config file not found: %s\n' "$CONFIG_FILE" >&2
    exit 1
fi

# Check dependencies
if ! command -v yq >/dev/null 2>&1; then
    printf 'wizard-preview: yq not found. Install: brew install yq\n' >&2
    exit 1
fi

# Parse YAML → JSON
CONFIG_JSON=""
if ! CONFIG_JSON=$(yq -o=json '.' "$CONFIG_FILE" 2>"$PREVIEW_YQ_ERR"); then
    printf 'wizard-preview: failed to parse config YAML:\n' >&2
    cat "$PREVIEW_YQ_ERR" >&2
    exit 1
fi

if [ -z "$CONFIG_JSON" ] || [ "$CONFIG_JSON" = "null" ]; then
    printf 'wizard-preview: config file is empty or invalid\n' >&2
    exit 1
fi

printf '=== Resolved contract preview ===\n'
printf '%s\n' "$CONFIG_JSON" | yq -P '.' -

printf '\n=== Constraints that will apply ===\n'

# Extract and display key fields
MODE=$(printf '%s' "$CONFIG_JSON" | jq -r '.mode // "strict_isolation"')
printf 'mode: %s\n' "$MODE"

if [ "$MODE" = "scoped_production" ]; then
    DRIVER=$(printf '%s' "$CONFIG_JSON" | jq -r '.e2e.backup.driver // "none"')
    printf 'backup driver: %s\n' "$DRIVER"
    TOKEN_COUNT=$(printf '%s' "$CONFIG_JSON" | jq '.e2e.production_scoped_access.scope_tokens | length // 0')
    printf 'scope tokens: %s\n' "$TOKEN_COUNT"
fi

FORBIDDEN_COUNT=$(printf '%s' "$CONFIG_JSON" | jq '.e2e.forbidden_url_patterns | length // 0')
printf 'forbidden URL patterns: %s\n' "$FORBIDDEN_COUNT"

UNIT_CMD=$(printf '%s' "$CONFIG_JSON" | jq -r '.unit.test_cmd // "(autodetect)"')
printf 'unit test command: %s\n' "$UNIT_CMD"
