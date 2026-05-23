#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/snapshot/dispatch.sh — snapshot dispatcher
#
# Reads resolved contract JSON from stdin (or file path as $1), fans out per
# source entry to the appropriate per-kind snapshot driver.
#
# Usage:
#   load-contract.sh <repo_root> | dispatch.sh [<snap_base_dir>]
#   dispatch.sh [<snap_base_dir>] < contract.json
#
# Snapshot output:
#   <snap_base_dir>/<source-id>/<timestamp>/
#   Default snap_base_dir: .autospec/clone-snapshots
#
# Environment (passed through to drivers):
#   DSN (or MYSQL_HOST/MYSQL_USER/etc.) — set by caller per source
#
# Exit codes:
#   0 = all sources snapshotted
#   1 = fatal
#   2 = one or more sources failed (operator-actionable)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

SNAP_BASE_DIR="${1:-.autospec/clone-snapshots}"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"

# ── Read contract JSON from stdin ─────────────────────────────────────────────
CONTRACT_JSON="$(cat)"

if [ -z "$CONTRACT_JSON" ] || [ "$CONTRACT_JSON" = "null" ]; then
    printf 'dispatch.sh: fatal: no contract JSON on stdin\n' >&2
    exit 1
fi

# ── Parse sources array ───────────────────────────────────────────────────────
SOURCE_COUNT=$(printf '%s' "$CONTRACT_JSON" | \
    python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d.get('sources',[])))" 2>/dev/null || echo 0)

if [ "$SOURCE_COUNT" -eq 0 ]; then
    printf 'dispatch.sh: fatal: no sources in contract\n' >&2
    exit 1
fi

FAILURES=0

# ── Iterate sources ───────────────────────────────────────────────────────────
for i in $(seq 0 $((SOURCE_COUNT - 1))); do
    SOURCE=$(printf '%s' "$CONTRACT_JSON" | \
        python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d['sources'][$i]))")

    KIND=$(printf '%s' "$SOURCE" | python3 -c "import sys,json; print(json.load(sys.stdin).get('kind',''))")
    SOURCE_ID="${KIND}-${i}"

    OUT_DIR="${SNAP_BASE_DIR}/${SOURCE_ID}/${TIMESTAMP}"
    mkdir -p "$OUT_DIR"

    printf 'dispatch.sh: source[%d] kind=%s → %s\n' "$i" "$KIND" "$OUT_DIR" >&2

    case "$KIND" in
        postgres)
            TABLES_FULL=$(printf '%s' "$SOURCE" | \
                python3 -c "import sys,json; d=json.load(sys.stdin); print(','.join(d.get('tables_full',[])))" 2>/dev/null || echo "")
            TABLES_SAMPLE=$(printf '%s' "$SOURCE" | \
                python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d.get('tables_sample',{})))" 2>/dev/null || echo "{}")

            DSN_ENV=$(printf '%s' "$SOURCE" | \
                python3 -c "import sys,json; print(json.load(sys.stdin).get('dsn_env',''))" 2>/dev/null || echo "")
            if [ -n "$DSN_ENV" ] && [ -n "${!DSN_ENV:-}" ]; then
                export DSN="${!DSN_ENV}"
            fi

            if ! bash "$SCRIPT_DIR/pg.sh" "$TABLES_FULL" "$TABLES_SAMPLE" "$OUT_DIR"; then
                printf 'dispatch.sh: error: postgres source[%d] snapshot failed\n' "$i" >&2
                FAILURES=$((FAILURES + 1))
            fi
            ;;

        mysql)
            TABLES_FULL=$(printf '%s' "$SOURCE" | \
                python3 -c "import sys,json; d=json.load(sys.stdin); print(','.join(d.get('tables_full',[])))" 2>/dev/null || echo "")
            TABLES_SAMPLE=$(printf '%s' "$SOURCE" | \
                python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d.get('tables_sample',{})))" 2>/dev/null || echo "{}")

            DSN_ENV=$(printf '%s' "$SOURCE" | \
                python3 -c "import sys,json; print(json.load(sys.stdin).get('dsn_env',''))" 2>/dev/null || echo "")
            if [ -n "$DSN_ENV" ] && [ -n "${!DSN_ENV:-}" ]; then
                export DSN="${!DSN_ENV}"
            fi

            if ! bash "$SCRIPT_DIR/mysql.sh" "$TABLES_FULL" "$TABLES_SAMPLE" "$OUT_DIR"; then
                printf 'dispatch.sh: error: mysql source[%d] snapshot failed\n' "$i" >&2
                FAILURES=$((FAILURES + 1))
            fi
            ;;

        sqlite)
            DSN_ENV=$(printf '%s' "$SOURCE" | \
                python3 -c "import sys,json; print(json.load(sys.stdin).get('dsn_env',''))" 2>/dev/null || echo "")
            SRC_PATH=""
            if [ -n "$DSN_ENV" ] && [ -n "${!DSN_ENV:-}" ]; then
                SRC_PATH="${!DSN_ENV}"
            fi

            if [ -z "$SRC_PATH" ]; then
                printf 'dispatch.sh: error: sqlite source[%d]: dsn_env not set or empty\n' "$i" >&2
                FAILURES=$((FAILURES + 1))
                continue
            fi

            if ! bash "$SCRIPT_DIR/sqlite.sh" "$SRC_PATH" "$OUT_DIR"; then
                printf 'dispatch.sh: error: sqlite source[%d] snapshot failed\n' "$i" >&2
                FAILURES=$((FAILURES + 1))
            fi
            ;;

        s3|filesystem|custom_cmd)
            printf 'dispatch.sh: info: kind=%s not implemented in C2 (deferred to C3)\n' "$KIND" >&2
            ;;

        *)
            printf 'dispatch.sh: error: unknown source kind: %s\n' "$KIND" >&2
            FAILURES=$((FAILURES + 1))
            ;;
    esac
done

if [ "$FAILURES" -gt 0 ]; then
    printf 'dispatch.sh: %d source(s) failed\n' "$FAILURES" >&2
    exit 2
fi

printf 'dispatch.sh: all sources snapshotted → %s\n' "$SNAP_BASE_DIR" >&2
