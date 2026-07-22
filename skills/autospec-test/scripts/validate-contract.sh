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
#   3 = shape-missing: edge_case_seeds enforcement=refuse_to_run_if_missing but no require_shapes provided
#
# Validation steps:
#   1. JSON Schema validation via `ajv validate`
#   2. Higher-level rules not expressible in JSON Schema:
#      a. Mode II conjunction: scoped_production requires i_understand_this_writes_to_production=true
#         AND e2e.backup.driver AND e2e.backup.restore_cmd
#      b. Fail-closed rule: e2e.forbidden_url_patterns=[] without
#         e2e.forbidden_url_patterns_intentionally_empty=true → exit 2
#   3. v2 invariants_v2 cross-field rules:
#      c. enabled-requires-metrics: invariants_v2.enabled=true requires at least one non-empty metric block
#      d. refuse-requires-shapes: edge_case_seeds.enforcement=refuse_to_run_if_missing requires
#         at least one entity with non-empty require_shapes → exit 3
#      e. leading-slash-routes: all apply_on_routes entries must start with /
#      f. scope-allowed-routes: when mode=scoped_production + invariants_v2.enabled=true,
#         apply_on_routes must be a subset of scope_tokens route_filter allowed_path_patterns

set -eu

# Resolve schema path
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
DEFAULT_SCHEMA="$REPO_ROOT/schemas/autospec-test-contract.schema.json"

# _v2_route_covered_by_patterns <route> <patterns_newline_sep>
# Returns 0 if route matches at least one pattern (anchors stripped), 1 otherwise.
_v2_route_covered_by_patterns() {
    local route="$1" patterns="$2" pattern clean_pattern
    while IFS= read -r pattern; do
        [ -z "$pattern" ] && continue
        clean_pattern="${pattern#^}"
        clean_pattern="${clean_pattern%\$}"
        if echo "$route" | grep -qE "$clean_pattern" 2>/dev/null; then
            return 0
        fi
    done <<< "$patterns"
    return 1
}

# _v2_check_scope_routes <json_data>
# Rule f: in scoped_production mode, every apply_on_routes entry must match a
# route_filter allowed_path_pattern. Exits 2 on violation.
_v2_check_scope_routes() {
    local json_data="$1"
    local allowed_patterns all_routes route
    allowed_patterns=$(printf '%s' "$json_data" | jq -r '
        [ .e2e.production_scoped_access.scope_tokens[]?
          | select(.kind == "route_filter")
          | .allowed_path_patterns[]? ] | .[]
    ' 2>/dev/null || true)
    [ -z "$allowed_patterns" ] && return 0
    all_routes=$(printf '%s' "$json_data" | jq -r '
        [ .e2e.invariants_v2.invariants[]?.apply_on_routes[]? ] | .[]
    ' 2>/dev/null || true)
    while IFS= read -r route; do
        [ -z "$route" ] && continue
        if ! _v2_route_covered_by_patterns "$route" "$allowed_patterns"; then
            printf 'validate-contract: refuse-to-run: mode=scoped_production: apply_on_routes "%s" not covered by any route_filter allowed_path_patterns\n' "$route" >&2
            exit 2
        fi
    done <<< "$all_routes"
}

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
    # Use mktemp -t for macOS/BSD compatibility (no .json extension in template)
    local tmpfile
    tmpfile=$(mktemp -t autospec-test-contract)
    # shellcheck disable=SC2064
    trap "rm -f '$tmpfile'" EXIT
    printf '%s' "$json_data" > "$tmpfile"

    # ── Step 1: JSON Schema validation ────────────────────────────────────────
    # ajv-cli 5.x --spec=draft2020 flag has a known CLI parsing bug; use Node/Ajv2020
    # directly (the same module ajv-cli ships) for reliable draft-2020-12 validation.
    local ajv_node
    ajv_node=$(node -e "
try {
  var fs = require('fs');
  // Prefer ajv-cli's bundled Ajv2020; fall back to system ajv
  var Ajv2020;
  try { Ajv2020 = require('ajv/dist/2020'); }
  catch(e) {
    var paths = [
      '/opt/homebrew/lib/node_modules/ajv-cli/node_modules/ajv/dist/2020',
      '/usr/local/lib/node_modules/ajv-cli/node_modules/ajv/dist/2020'
    ];
    for (var i = 0; i < paths.length; i++) {
      try { Ajv2020 = require(paths[i]); break; } catch(e2) {}
    }
  }
  if (!Ajv2020) { console.error('ajv/dist/2020 not found'); process.exit(1); }
  var schema = JSON.parse(fs.readFileSync('$schema_file', 'utf8'));
  var data   = JSON.parse(fs.readFileSync('$tmpfile',     'utf8'));
  var ajv    = new Ajv2020({strict: false, allErrors: true});
  var valid  = ajv.validate(schema, data);
  if (!valid) { console.error(ajv.errorsText()); process.exitCode = 2; }
} catch(e) { console.error(e.message); process.exit(1); }
" 2>&1)
    local ajv_rc=$?
    if [ "$ajv_rc" -ne 0 ]; then
        printf 'validate-contract: schema validation failed:\n%s\n' "$ajv_node" >&2
        exit "$ajv_rc"
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

    # ── Step 3: v2 invariants_v2 cross-field rules ────────────────────────────
    local v2_enabled
    v2_enabled=$(printf '%s' "$json_data" | jq -r '.e2e.invariants_v2.enabled // false')

    if [ "$v2_enabled" = "true" ]; then

        # ── Rule c: enabled-requires-metrics ──────────────────────────────────
        # At least one of: invariants (non-empty array), window_contracts (non-empty array),
        # crawler.enabled=true, contract_symmetry (non-empty array)
        local inv_count wc_count crawler_on cs_count
        inv_count=$(printf '%s' "$json_data" | jq '.e2e.invariants_v2.invariants | length // 0')
        wc_count=$(printf '%s' "$json_data" | jq '.e2e.invariants_v2.window_contracts | length // 0')
        crawler_on=$(printf '%s' "$json_data" | jq -r '.e2e.invariants_v2.crawler.enabled // false')
        cs_count=$(printf '%s' "$json_data" | jq '.e2e.invariants_v2.contract_symmetry | length // 0')

        if [ "$inv_count" = "0" ] && [ "$wc_count" = "0" ] && [ "$crawler_on" != "true" ] && [ "$cs_count" = "0" ]; then
            printf 'validate-contract: refuse-to-run: e2e.invariants_v2.enabled=true but no metric block is populated (invariants, window_contracts, crawler, or contract_symmetry must have at least one entry)\n' >&2
            exit 2
        fi

        # ── Rule e: leading-slash-routes ──────────────────────────────────────
        # All apply_on_routes entries must start with /
        local bad_routes
        bad_routes=$(printf '%s' "$json_data" | jq -r '
            [ .e2e.invariants_v2.invariants[]?.apply_on_routes[]? ] |
            map(select(startswith("/") | not)) |
            .[]
        ' 2>/dev/null || true)
        if [ -n "$bad_routes" ]; then
            printf 'validate-contract: refuse-to-run: all apply_on_routes entries must start with /. Found invalid routes:\n%s\n' "$bad_routes" >&2
            exit 2
        fi

        # ── Rule f: scope-allowed-routes (Mode II only) ───────────────────────
        # When mode=scoped_production + invariants_v2.enabled=true,
        # all apply_on_routes must be covered by route_filter allowed_path_patterns
        if [ "$mode" = "scoped_production" ]; then
            _v2_check_scope_routes "$json_data"
        fi

    fi

    # ── Rule d: refuse-requires-shapes ────────────────────────────────────────
    # Runs regardless of enabled flag — enforcement is a declaration-level contract
    local enforcement
    enforcement=$(printf '%s' "$json_data" | jq -r '.e2e.invariants_v2.edge_case_seeds.enforcement // empty')

    if [ "$enforcement" = "refuse_to_run_if_missing" ]; then
        # Count total require_shapes entries across all entity keys (excluding "enforcement")
        local total_shapes
        total_shapes=$(printf '%s' "$json_data" | jq '
            [ .e2e.invariants_v2.edge_case_seeds
              | to_entries[]
              | select(.key != "enforcement")
              | .value.require_shapes?
              | length? // 0
            ] | add // 0
        ' 2>/dev/null || echo "0")

        if [ "$total_shapes" = "0" ]; then
            printf 'validate-contract: shape-missing: edge_case_seeds.enforcement=refuse_to_run_if_missing but no entity with non-empty require_shapes found. Provision seed shapes or change enforcement to warn_if_missing/skip_if_missing.\n' >&2
            exit 3
        fi
    fi

    # ── All checks passed ──────────────────────────────────────────────────────
    exit 0
}

validate_contract "${1:--}" "${2:-}"
