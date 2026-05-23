#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/load-contract.sh — load and validate the
# autospec-e2e-clone contract (.autospec/clone.yml) for a target repository.
#
# Usage: load-contract.sh <repo_root>
#   Outputs resolved contract JSON to stdout.
#
# Exit codes:
#   0 = ok — resolved contract JSON on stdout
#   1 = fatal — tool missing, unreadable file, internal error
#   2 = refuse-to-run — contract invalid or missing required fields
#       (operator-actionable; stderr explains what to fix)
#
# Dependencies: yq (YAML→JSON), ajv (JSON Schema validation)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SKILL_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SKILL_DIR/../../.." && pwd)"

SCHEMA_FILE="$REPO_ROOT/schemas/autospec-clone-contract.schema.json"

load_contract() {
    local repo_root="${1:-.}"

    # ── Validate repo_root exists ──────────────────────────────────────────────
    if [ ! -d "$repo_root" ]; then
        printf 'load-contract: fatal: repo_root not found: %s\n' "$repo_root" >&2
        exit 1
    fi

    # ── Check for contract file ────────────────────────────────────────────────
    local contract_file="$repo_root/.autospec/clone.yml"
    if [ ! -f "$contract_file" ]; then
        printf 'load-contract: refuse-to-run: .autospec/clone.yml not found in %s\n' "$repo_root" >&2
        printf 'Create .autospec/clone.yml with required fields: sources (>=1 item), expose.kind\n' >&2
        exit 2
    fi

    # ── Dependency checks ──────────────────────────────────────────────────────
    if ! command -v yq >/dev/null 2>&1; then
        printf 'load-contract: fatal: yq not found. Install with: brew install yq\n' >&2
        exit 1
    fi

    # ── Parse YAML → JSON ─────────────────────────────────────────────────────
    local contract_json
    local yq_err
    yq_err=$(mktemp -t load-contract-yq-err-XXXXXX.txt)
    # shellcheck disable=SC2064
    trap "rm -f '$yq_err'" EXIT

    if ! contract_json=$(yq -o=json '.' "$contract_file" 2>"$yq_err"); then
        printf 'load-contract: fatal: failed to parse %s:\n' "$contract_file" >&2
        cat "$yq_err" >&2
        exit 1
    fi

    # yq emits "null" for empty files
    if [ "$contract_json" = "null" ] || [ -z "$contract_json" ]; then
        printf 'load-contract: refuse-to-run: %s is empty or unparseable\n' "$contract_file" >&2
        exit 2
    fi

    # ── Validate required fields inline (fast path, no ajv needed for basics) ─
    local has_sources has_expose_kind
    has_sources=$(printf '%s' "$contract_json" | \
        python3 -c "import sys,json; d=json.load(sys.stdin); print('yes' if d.get('sources') and len(d['sources'])>=1 else 'no')" 2>/dev/null || echo "no")
    has_expose_kind=$(printf '%s' "$contract_json" | \
        python3 -c "import sys,json; d=json.load(sys.stdin); print('yes' if d.get('expose',{}).get('kind') else 'no')" 2>/dev/null || echo "no")

    if [ "$has_sources" != "yes" ]; then
        printf 'load-contract: refuse-to-run: sources is missing or empty in %s\n' "$contract_file" >&2
        printf 'Add at least one source entry with kind: (postgres|mysql|sqlite|s3|filesystem|custom_cmd)\n' >&2
        exit 2
    fi

    if [ "$has_expose_kind" != "yes" ]; then
        printf 'load-contract: refuse-to-run: expose.kind is missing in %s\n' "$contract_file" >&2
        printf 'Add expose.kind: (docker_compose|k8s_ephemeral_namespace|dedicated_staging_slot|custom_cmd)\n' >&2
        exit 2
    fi

    # ── JSON Schema validation via ajv (if available) ─────────────────────────
    if command -v ajv >/dev/null 2>&1 && [ -f "$SCHEMA_FILE" ]; then
        local tmpjson
        tmpjson=$(mktemp -t load-contract-XXXXXX.json)
        # shellcheck disable=SC2064
        trap "rm -f '$yq_err' '$tmpjson'" EXIT
        printf '%s\n' "$contract_json" > "$tmpjson"

        local ajv_out ajv_rc=0
        ajv_out=$(ajv validate -s "$SCHEMA_FILE" -d "$tmpjson" --spec=draft2020 2>&1) || ajv_rc=$?
        if [ "$ajv_rc" -ne 0 ]; then
            printf 'load-contract: refuse-to-run: schema validation failed:\n' >&2
            printf '%s\n' "$ajv_out" >&2
            exit 2
        fi
    fi

    # ── Emit resolved contract JSON ────────────────────────────────────────────
    printf '%s\n' "$contract_json"
}

load_contract "${1:-.}"
