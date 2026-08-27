#!/usr/bin/env bash
# gate-stage-2-5.sh — Stage 2.5 orchestrator for autospec-test v2.
#
# Usage: gate-stage-2-5.sh <target_dir> [--output <gate_json_path>]
#
# Reads the contract at <target_dir>/.autospec/test.yml. If invariants_v2.enabled
# is not true, emits a skipped gate JSON and exits 0 (zero overhead on v1-only targets).
#
# Exit codes:
#   0 = gate passed (all metrics passed or skipped)
#   1 = gate failed (block PR)
#   2 = fatal error (missing target, bad contract, refused to run due to seeds)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

TARGET_DIR="${1:-}"
if [ -z "$TARGET_DIR" ] || [ ! -d "$TARGET_DIR" ]; then
    printf 'gate-stage-2-5: fatal: target directory not found: %s\n' "$TARGET_DIR" >&2
    exit 2
fi
# Canonicalize to an absolute path: file:// base URLs built below require one
# (a relative path produces an invalid file:// URL for Playwright to navigate to).
TARGET_DIR="$(cd "$TARGET_DIR" && pwd)"

OUTPUT_FILE=""
shift
while [ $# -gt 0 ]; do
    case "$1" in
        --output) OUTPUT_FILE="${2:-}"; shift 2 ;;
        *) printf 'gate-stage-2-5: unknown flag: %s\n' "$1" >&2; exit 2 ;;
    esac
done

CONTRACT_YML="$TARGET_DIR/.autospec/test.yml"
if [ ! -f "$CONTRACT_YML" ]; then
    printf 'gate-stage-2-5: fatal: no .autospec/test.yml in %s\n' "$TARGET_DIR" >&2
    exit 2
fi

# ── Read invariants_v2.enabled ────────────────────────────────────────────────

V2_ENABLED=""
if command -v yq >/dev/null 2>&1; then
    V2_ENABLED=$(yq -r '.e2e.invariants_v2.enabled // "false"' "$CONTRACT_YML" 2>/dev/null || echo "false")
fi

if [ "$V2_ENABLED" != "true" ]; then
    SKIPPED_JSON='{"metric":"2.5","skipped":true,"passed":true,"reason":"invariants_v2.enabled != true"}'
    if [ -n "$OUTPUT_FILE" ]; then
        printf '%s\n' "$SKIPPED_JSON" > "$OUTPUT_FILE"
    else
        printf '%s\n' "$SKIPPED_JSON"
    fi
    exit 0
fi

# ── v2 is enabled: verify edge_case_seeds, then run F/G/H/I ─────────────────

TARGET_NAME="$(basename "$TARGET_DIR")"
METRICS_JSON='{}'
ALL_PASSED=true

# 2. Verify edge_case_seeds if declared
SEEDS_DECLARED=""
if command -v yq >/dev/null 2>&1; then
    SEEDS_DECLARED=$(yq -r '.e2e.invariants_v2.edge_case_seeds // ""' "$CONTRACT_YML" 2>/dev/null | grep -v '^$' || true)
fi

if [ -n "$SEEDS_DECLARED" ]; then
    VERIFY_SEEDS="$SCRIPT_DIR/seed-shapes/verify-seeds.mjs"
    if [ -f "$VERIFY_SEEDS" ]; then
        # verify-seeds.mjs's CLI parses --contract/--dsn/--store-kind named
        # flags (it does not accept a bare positional target dir). --dsn and
        # --store-kind default to an in-memory sqlite store: there is no
        # established wiring yet from a contract to a real Mode II clone
        # connection string, so AUTOSPEC_SEED_DSN / AUTOSPEC_SEED_STORE_KIND
        # let a caller that *does* have a live clone override the defaults.
        SEED_EXIT=0
        node "$VERIFY_SEEDS" \
            --contract "$CONTRACT_YML" \
            --dsn "${AUTOSPEC_SEED_DSN:-:memory:}" \
            --store-kind "${AUTOSPEC_SEED_STORE_KIND:-sqlite}" 2>&1 || SEED_EXIT=$?
        if [ "$SEED_EXIT" -eq 2 ]; then
            printf 'gate-stage-2-5: fatal: edge_case_seeds verification refused to run (missing shapes)\n' >&2
            exit 2
        fi
        if [ "$SEED_EXIT" -ne 0 ]; then
            ALL_PASSED=false
        fi
    fi
fi

# ── Build the {contract, base_url} JSON payload shared by F/G/H/I ───────────
#
# All four runners read a JSON document from stdin: { contract, base_url,
# route_list?, custom_kinds_dir? }. `contract` is the parsed .autospec/test.yml
# as JSON; `base_url` may be a live server URL or a file:///... URL.
#
# This wiring supports only the tractable case: a target that ships a static
# `src/index.html` (or `index.html`) fixture reachable at route "/". It does
# NOT stand up a dev server. A target whose contract needs an actual network
# request (window_contracts, contract_symmetry) or references any route other
# than "/" is detected here and its metric is skipped with a loud, explicit
# reason instead of being invoked with a payload that could never work.
CONTRACT_JSON="{}"
if command -v yq >/dev/null 2>&1; then
    CONTRACT_JSON=$(yq -o=json '.' "$CONTRACT_YML" 2>/dev/null || echo '{}')
fi
if [ -z "$CONTRACT_JSON" ] || [ "$CONTRACT_JSON" = "null" ]; then
    CONTRACT_JSON='{}'
fi

WEB_ROOT=""
if [ -f "$TARGET_DIR/src/index.html" ]; then
    WEB_ROOT="$TARGET_DIR/src"
elif [ -f "$TARGET_DIR/index.html" ]; then
    WEB_ROOT="$TARGET_DIR"
fi

BASE_URL=""
if [ -n "$WEB_ROOT" ]; then
    # file:// has no automatic index.html resolution (unlike an HTTP server),
    # so a directory URL just renders a listing. Point base_url directly at
    # index.html with a trailing "#": each runner builds its navigation URL
    # as `base_url.replace(/\/$/, '') + route`, and for route "/" that
    # appends "/" as a harmless URL fragment (ignored by the browser for
    # resource resolution), landing back on the same document instead of a
    # directory listing. Only route "/" is supported this way.
    BASE_URL="file://$WEB_ROOT/index.html#"
fi

NEEDS_WINDOW_SERVER=$(printf '%s' "$CONTRACT_JSON" | jq -r '((.e2e.invariants_v2.window_contracts // []) | length) > 0' 2>/dev/null || echo "false")
NEEDS_SYMMETRY_SERVER=$(printf '%s' "$CONTRACT_JSON" | jq -r '((.e2e.invariants_v2.contract_symmetry // []) | length) > 0' 2>/dev/null || echo "false")
HAS_NONROOT_ROUTES=$(printf '%s' "$CONTRACT_JSON" | jq -r '[(.e2e.invariants_v2.invariants // [])[].apply_on_routes[]?] | map(select(. != "/")) | length > 0' 2>/dev/null || echo "false")

build_payload() {
    jq -n --argjson contract "$CONTRACT_JSON" --arg base_url "$BASE_URL" \
        '{contract: $contract, base_url: $base_url}'
}

# Helper: run a Node metric runner (stdin JSON payload) and collect JSON output.
# $1 = metric name, $2 = runner path relative to $SCRIPT_DIR, $3 = non-empty
# skip reason (skip without invoking, loud reason), $4 = stdin payload JSON.
run_metric() {
    local name="$1"
    local runner="$SCRIPT_DIR/$2"
    local skip_reason="$3"
    local payload="$4"

    if [ -n "$skip_reason" ]; then
        jq -n --arg name "$name" --arg reason "$skip_reason" \
            '{"metric": $name, "passed": true, "skipped": true, "reason": $reason}'
        return 0
    fi

    if [ ! -f "$runner" ]; then
        # Runner not yet installed — emit a stub pass so v1 targets are unaffected
        printf '{"metric":"%s","passed":true,"skipped":true,"reason":"runner not installed"}\n' "$name"
        return 0
    fi

    local out
    if out=$(printf '%s' "$payload" | node "$runner" 2>&1); then
        printf '%s\n' "$out"
    else
        local exit_code=$?
        if [ "$exit_code" -eq 2 ]; then
            printf '{"metric":"%s","passed":false,"refused":true,"reason":"runner refused to run"}\n' "$name"
        elif printf '%s' "$out" | jq -e . >/dev/null 2>&1; then
            # Non-zero exit (1) with well-formed JSON on stdout is a real
            # failing verdict (each runner does `process.exit(passed?0:1)`),
            # not a crash — pass it through as-is instead of masking it
            # behind a generic "runner exited N" placeholder.
            printf '%s\n' "$out"
        else
            printf '{"metric":"%s","passed":false,"reason":"runner exited %s","raw":"%s"}\n' \
                "$name" "$exit_code" "$(printf '%s' "$out" | head -1 | tr '"' "'")"
        fi
    fi
}

# 3. Metric F — Structural invariants
F_SKIP_REASON=""
if [ -z "$WEB_ROOT" ]; then
    F_SKIP_REASON="no static index.html found under <target>/src or <target>/; F needs a base_url to navigate to and this wiring does not stand up a live server"
elif [ "$HAS_NONROOT_ROUTES" = "true" ]; then
    F_SKIP_REASON="an invariant's apply_on_routes references a route other than \"/\"; only the root route is supported by the static file:// mapping"
fi
F_JSON=$(run_metric "F" "invariants/run-structural.mjs" "$F_SKIP_REASON" "$(build_payload)" || echo '{"metric":"F","passed":true,"skipped":true}')
F_PASSED=$(printf '%s' "$F_JSON" | jq -r 'if .passed == false then false else true end' 2>/dev/null || echo "true")

# 4. Metric G — Window-contract symmetry
G_SKIP_REASON=""
if [ "$NEEDS_WINDOW_SERVER" = "true" ]; then
    G_SKIP_REASON="window_contracts require a live HTTP server to observe a real network request; this wiring only supports static file:// fixtures and does not stand up a dev server"
elif [ -z "$WEB_ROOT" ]; then
    G_SKIP_REASON="no static index.html found under <target>/src or <target>/; G needs a base_url to navigate to and this wiring does not stand up a live server"
fi
G_JSON=$(run_metric "G" "window-contract/run-window.mjs" "$G_SKIP_REASON" "$(build_payload)" || echo '{"metric":"G","passed":true,"skipped":true}')
G_PASSED=$(printf '%s' "$G_JSON" | jq -r 'if .passed == false then false else true end' 2>/dev/null || echo "true")

# 5. Metric H — Extended crawler
H_SKIP_REASON=""
if [ -z "$WEB_ROOT" ]; then
    H_SKIP_REASON="no static index.html found under <target>/src or <target>/; H needs a base_url to navigate to and this wiring does not stand up a live server"
fi
H_JSON=$(run_metric "H" "crawler-v2/extended-crawler.mjs" "$H_SKIP_REASON" "$(build_payload)" || echo '{"metric":"H","passed":true,"skipped":true}')
H_PASSED=$(printf '%s' "$H_JSON" | jq -r 'if .passed == false then false else true end' 2>/dev/null || echo "true")

# 6. Metric I — Data-source contract symmetry
I_SKIP_REASON=""
if [ "$NEEDS_SYMMETRY_SERVER" = "true" ]; then
    I_SKIP_REASON="contract_symmetry requires a live HTTP server to fetch and compare API responses; this wiring only supports static file:// fixtures and does not stand up a dev server"
elif [ -z "$WEB_ROOT" ]; then
    I_SKIP_REASON="no static index.html found under <target>/src or <target>/; I needs a base_url to navigate to and this wiring does not stand up a live server"
fi
I_JSON=$(run_metric "I" "contract-symmetry/run-symmetry.mjs" "$I_SKIP_REASON" "$(build_payload)" || echo '{"metric":"I","passed":true,"skipped":true}')
I_PASSED=$(printf '%s' "$I_JSON" | jq -r 'if .passed == false then false else true end' 2>/dev/null || echo "true")

# If any metric failed, overall fails
if [ "$F_PASSED" != "true" ] || [ "$G_PASSED" != "true" ] || \
   [ "$H_PASSED" != "true" ] || [ "$I_PASSED" != "true" ]; then
    ALL_PASSED=false
fi

# 7. Compose Stage 2.5 gate JSON
OVERALL=$([ "$ALL_PASSED" = "true" ] && echo "true" || echo "false")

GATE_JSON=$(jq -n \
    --arg target "$TARGET_NAME" \
    --argjson passed "$OVERALL" \
    --arg metric "2.5" \
    --argjson f_result "$(printf '%s' "$F_JSON" | jq '.' 2>/dev/null || echo 'null')" \
    --argjson g_result "$(printf '%s' "$G_JSON" | jq '.' 2>/dev/null || echo 'null')" \
    --argjson h_result "$(printf '%s' "$H_JSON" | jq '.' 2>/dev/null || echo 'null')" \
    --argjson i_result "$(printf '%s' "$I_JSON" | jq '.' 2>/dev/null || echo 'null')" \
    '{
        "metric": $metric,
        "target": $target,
        "passed": $passed,
        "metrics": {
            "F": $f_result,
            "G": $g_result,
            "H": $h_result,
            "I": $i_result
        }
    }' 2>/dev/null || echo "{\"metric\":\"2.5\",\"target\":\"$TARGET_NAME\",\"passed\":$OVERALL}")

if [ -n "$OUTPUT_FILE" ]; then
    printf '%s\n' "$GATE_JSON" > "$OUTPUT_FILE"
else
    printf '%s\n' "$GATE_JSON"
fi

if [ "$ALL_PASSED" = "true" ]; then
    exit 0
else
    exit 1
fi
