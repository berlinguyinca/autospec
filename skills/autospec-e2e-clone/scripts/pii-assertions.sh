#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/pii-assertions.sh
#
# Run post-anonymization PII assertions from the clone contract.
# Each assertion is a SQL query that should return 0 rows / count = 0.
# Fails closed: any non-zero count → exit 2.
#
# Usage:
#   pii-assertions.sh <snapshot-dir> [--contract <path-to-clone.yml>]
#                                     [--repo-root <repo-root>]
#                                     [--dsn <connection-string>]
#
# Exit codes:
#   0  all assertions pass
#   1  fatal (missing deps, unreadable contract, internal error)
#   2  one or more PII assertions failed (fail-closed)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SKILL_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------

SNAPSHOT_DIR=""
CONTRACT_PATH=""
REPO_ROOT=""
DSN=""

usage() {
    cat >&2 <<EOF
Usage: $0 <snapshot-dir> [--contract <clone.yml>] [--repo-root <root>] [--dsn <connection-string>]
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --contract)
            shift; CONTRACT_PATH="${1:?--contract requires a value}"
            ;;
        --repo-root)
            shift; REPO_ROOT="${1:?--repo-root requires a value}"
            ;;
        --dsn)
            shift; DSN="${1:?--dsn requires a value}"
            ;;
        -h|--help)
            usage; exit 0
            ;;
        -*)
            printf 'pii-assertions: unknown option: %s\n' "$1" >&2
            usage; exit 1
            ;;
        *)
            if [ -z "$SNAPSHOT_DIR" ]; then
                SNAPSHOT_DIR="$1"
            else
                printf 'pii-assertions: unexpected argument: %s\n' "$1" >&2
                usage; exit 1
            fi
            ;;
    esac
    shift
done

if [ -z "$SNAPSHOT_DIR" ]; then
    printf 'pii-assertions: fatal: <snapshot-dir> is required\n' >&2
    usage; exit 1
fi

if [ ! -d "$SNAPSHOT_DIR" ]; then
    printf 'pii-assertions: fatal: snapshot-dir not found: %s\n' "$SNAPSHOT_DIR" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Locate repo root
# ---------------------------------------------------------------------------

find_repo_root() {
    local dir="$1"
    local depth=0
    while [ "$depth" -lt 20 ]; do
        if [ -d "$dir/.autospec" ]; then
            printf '%s' "$dir"
            return 0
        fi
        local parent
        parent="$(dirname "$dir")"
        if [ "$parent" = "$dir" ]; then break; fi
        dir="$parent"
        depth=$((depth + 1))
    done
    return 1
}

if [ -z "$REPO_ROOT" ]; then
    REPO_ROOT="$(find_repo_root "$(cd "$SNAPSHOT_DIR" && pwd)")" || true
fi
if [ -z "$REPO_ROOT" ]; then
    printf 'pii-assertions: fatal: could not locate repo root (no .autospec/ directory)\n' >&2
    exit 1
fi

if [ -z "$CONTRACT_PATH" ]; then
    CONTRACT_PATH="$REPO_ROOT/.autospec/clone.yml"
fi

if [ ! -f "$CONTRACT_PATH" ]; then
    printf 'pii-assertions: refuse-to-run: contract not found: %s\n' "$CONTRACT_PATH" >&2
    exit 2
fi

# ---------------------------------------------------------------------------
# Dependency checks
# ---------------------------------------------------------------------------

if ! command -v yq >/dev/null 2>&1; then
    printf 'pii-assertions: fatal: yq not found. Install with: brew install yq\n' >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Parse assertions from contract
# ---------------------------------------------------------------------------

# Extract all pii_assertions arrays from all anonymize.rules[*].pii_assertions[]
# yq output: one assertion per line prefixed with "TABLE|SQL"
# We use a temp file to avoid process substitution with set -e
TMP_ASSERTIONS="$(mktemp -t pii-assertions-XXXXXX.txt)"
trap 'rm -f "$TMP_ASSERTIONS"' EXIT

yq -r '
  .anonymize.rules[]? |
  . as $rule |
  (.pii_assertions // [])[] |
  ($rule.table) + "|" + .
' "$CONTRACT_PATH" > "$TMP_ASSERTIONS" 2>/dev/null || true

if [ ! -s "$TMP_ASSERTIONS" ]; then
    printf 'pii-assertions: no pii_assertions defined in contract — nothing to check\n'
    exit 0
fi

# ---------------------------------------------------------------------------
# Determine DB driver from contract sources
# ---------------------------------------------------------------------------

# Detect source kind from contract (first source wins for assertion runner)
SOURCE_KIND="$(yq -r '.sources[0].kind // "postgres"' "$CONTRACT_PATH" 2>/dev/null || echo "postgres")"

# Resolve DSN from env if not explicitly provided
if [ -z "$DSN" ]; then
    DSN_ENV="$(yq -r '.sources[0].dsn_env // ""' "$CONTRACT_PATH" 2>/dev/null || true)"
    if [ -n "$DSN_ENV" ]; then
        DSN="${!DSN_ENV:-}"
    fi
fi

run_sql_assertion() {
    local sql="$1"
    local count

    case "$SOURCE_KIND" in
        postgres)
            if [ -z "$DSN" ]; then
                printf 'pii-assertions: fatal: no DSN available for postgres assertion\n' >&2
                exit 1
            fi
            if ! command -v psql >/dev/null 2>&1; then
                printf 'pii-assertions: fatal: psql not found\n' >&2
                exit 1
            fi
            count="$(psql "$DSN" -tAc "$sql" 2>&1)" || {
                printf 'pii-assertions: fatal: psql query failed: %s\n' "$sql" >&2
                exit 1
            }
            ;;
        mysql)
            if [ -z "$DSN" ]; then
                printf 'pii-assertions: fatal: no DSN available for mysql assertion\n' >&2
                exit 1
            fi
            if ! command -v mysql >/dev/null 2>&1; then
                printf 'pii-assertions: fatal: mysql client not found\n' >&2
                exit 1
            fi
            count="$(mysql "$DSN" -sNe "$sql" 2>&1)" || {
                printf 'pii-assertions: fatal: mysql query failed: %s\n' "$sql" >&2
                exit 1
            }
            ;;
        sqlite)
            # Find .db file in snapshot dir
            local db_file
            db_file="$(find "$SNAPSHOT_DIR" -name '*.db' | head -1)"
            if [ -z "$db_file" ]; then
                printf 'pii-assertions: fatal: no .db file found in snapshot-dir for sqlite assertion\n' >&2
                exit 1
            fi
            if ! command -v sqlite3 >/dev/null 2>&1; then
                printf 'pii-assertions: fatal: sqlite3 not found\n' >&2
                exit 1
            fi
            count="$(sqlite3 "$db_file" "$sql" 2>&1)" || {
                printf 'pii-assertions: fatal: sqlite3 query failed: %s\n' "$sql" >&2
                exit 1
            }
            ;;
        *)
            printf 'pii-assertions: fatal: unsupported source kind for assertions: %s\n' "$SOURCE_KIND" >&2
            exit 1
            ;;
    esac

    printf '%s' "${count:-0}"
}

# ---------------------------------------------------------------------------
# Run assertions
# ---------------------------------------------------------------------------

FAILURES=0
TOTAL=0

while IFS='|' read -r table assertion_sql; do
    [ -z "$assertion_sql" ] && continue
    TOTAL=$((TOTAL + 1))

    printf 'pii-assertions: [%s] checking: %s\n' "$table" "$assertion_sql"

    count="$(run_sql_assertion "$assertion_sql")"
    count_trimmed="$(printf '%s' "$count" | tr -d '[:space:]')"

    if [ "$count_trimmed" != "0" ] && [ -n "$count_trimmed" ]; then
        printf 'pii-assertions: FAIL [%s] assertion returned non-zero count (%s): %s\n' \
            "$table" "$count_trimmed" "$assertion_sql" >&2
        FAILURES=$((FAILURES + 1))
    else
        printf 'pii-assertions: PASS [%s]\n' "$table"
    fi
done < "$TMP_ASSERTIONS"

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------

printf 'pii-assertions: %d/%d assertions passed\n' "$((TOTAL - FAILURES))" "$TOTAL"

if [ "$FAILURES" -gt 0 ]; then
    printf 'pii-assertions: FAIL — %d assertion(s) failed (exit 2)\n' "$FAILURES" >&2
    exit 2
fi

printf 'pii-assertions: all assertions passed\n'
exit 0
