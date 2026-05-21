#!/usr/bin/env bash
# load-contract.sh — load, merge with autodetect defaults, and validate
# the autospec-test contract for a target repository.
#
# Usage: load_contract <repo_root>
#   Outputs resolved contract JSON to stdout.
#
# Exit codes (shared stdin/stdout JSON convention for all downstream phases):
#   0 = ok — resolved contract JSON on stdout
#   1 = fatal error — tool missing, unreadable file, internal error
#   2 = refuse-to-run — contract invalid or missing required safety fields
#       (operator-actionable; stderr explains what to fix)
#
# Resolution order per field: explicit yml → autodetect → fail-closed if neither.
#
# Dependencies: yq (YAML→JSON), jq (JSON merge), ajv (schema validation)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

load_contract() {
    local repo_root="${1:-.}"

    if [ ! -d "$repo_root" ]; then
        printf 'load-contract: fatal: repo_root not found: %s\n' "$repo_root" >&2
        exit 1
    fi

    # ── Dependency checks ──────────────────────────────────────────────────────
    if ! command -v yq >/dev/null 2>&1; then
        printf 'load-contract: fatal: yq not found. Install with: brew install yq\n' >&2
        exit 1
    fi
    if ! command -v jq >/dev/null 2>&1; then
        printf 'load-contract: fatal: jq not found. Install with: brew install jq\n' >&2
        exit 1
    fi

    # ── Step 1: Parse .autospec/test.yml if present ────────────────────────────
    local explicit_json="{}"
    local contract_file="$repo_root/.autospec/test.yml"
    if [ -f "$contract_file" ]; then
        # yq converts YAML to JSON; exit non-zero on parse error
        if ! explicit_json=$(yq -o=json '.' "$contract_file" 2>/tmp/load-contract-yq-err.txt); then
            printf 'load-contract: fatal: failed to parse %s:\n' "$contract_file" >&2
            cat /tmp/load-contract-yq-err.txt >&2
            exit 1
        fi
        # Verify it's valid JSON (yq can emit "null" for empty files)
        if [ "$explicit_json" = "null" ] || [ -z "$explicit_json" ]; then
            explicit_json="{}"
        fi
    fi

    # ── Step 2: Run autodetect to fill missing fields ──────────────────────────
    local detect_json="{}"
    if ! detect_json=$("$SCRIPT_DIR/autodetect.sh" "$repo_root" 2>/dev/null); then
        detect_json="{}"
    fi

    # ── Step 3: Deep-merge explicit over autodetect defaults ──────────────────
    # Strategy: for each top-level key, explicit wins; for nested objects,
    # merge one level deep (explicit nested keys override autodetect nested keys).
    local merged_json
    merged_json=$(jq -n \
        --argjson explicit "$explicit_json" \
        --argjson detect "$detect_json" \
        '
        # merge two objects one level deep: a wins over b
        def deep_merge(a; b):
            if (a | type) == "object" and (b | type) == "object"
            then b + a
            else a
            end;

        # Start from detect defaults
        $detect
        # Overlay each explicit key (deep-merge for objects)
        | . + (
            $explicit | to_entries | map(
                .value = deep_merge(.value; ($detect[.key] // null))
            ) | from_entries
          )
        '
    )

    # ── Step 4: Validate merged contract ──────────────────────────────────────
    local tmpfile
    tmpfile=$(mktemp /tmp/autospec-test-merged-XXXXXX.json)
    # shellcheck disable=SC2064
    trap "rm -f '$tmpfile'" EXIT
    printf '%s' "$merged_json" > "$tmpfile"

    # Pass the schema path explicitly
    local schema_file="$SCRIPT_DIR/../../../schemas/autospec-test-contract.schema.json"
    # Capture validate exit code explicitly — do not use `if !` which can swallow
    # the exit code under set -e.
    local validate_rc=0
    "$SCRIPT_DIR/validate-contract.sh" "$tmpfile" "$schema_file" || validate_rc=$?
    if [ "$validate_rc" -ne 0 ]; then
        exit "$validate_rc"
    fi

    # ── Step 5: Emit resolved contract JSON ────────────────────────────────────
    printf '%s\n' "$merged_json"
}

load_contract "${1:-.}"
