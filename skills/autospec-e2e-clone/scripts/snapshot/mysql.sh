#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/snapshot/mysql.sh — MySQL snapshot driver
#
# Usage: mysql.sh <tables_full_csv> <tables_sample_json> <out_dir>
#
# Environment:
#   DSN — MySQL connection DSN (e.g. mysql://user:pass@host:3306/db)
#         or individual vars: MYSQL_HOST, MYSQL_PORT, MYSQL_USER, MYSQL_PASSWORD, MYSQL_DATABASE
#
# Output layout:
#   <out_dir>/schema.sql          — full schema dump (no data)
#   <out_dir>/<table>.sql         — per-table data dump
#
# Exit codes:
#   0 = ok
#   1 = fatal
#   2 = refuse-to-run

set -euo pipefail

TABLES_FULL_CSV="${1:-}"
TABLES_SAMPLE_JSON="${2:-{}}"
OUT_DIR="${3:-}"

# ── Validate arguments ────────────────────────────────────────────────────────
if [ -z "$OUT_DIR" ]; then
    printf 'mysql.sh: fatal: OUT_DIR argument required\n' >&2
    exit 1
fi

# ── Parse DSN or fall back to individual env vars ─────────────────────────────
_MYSQL_HOST="${MYSQL_HOST:-127.0.0.1}"
_MYSQL_PORT="${MYSQL_PORT:-3306}"
_MYSQL_USER="${MYSQL_USER:-root}"
_MYSQL_PASSWORD="${MYSQL_PASSWORD:-}"
_MYSQL_DATABASE="${MYSQL_DATABASE:-}"

if [ -n "${DSN:-}" ]; then
    # Parse mysql://user:pass@host:port/db
    _MYSQL_USER=$(printf '%s' "$DSN" | python3 -c "import sys,urllib.parse; u=urllib.parse.urlparse(sys.stdin.read().strip()); print(u.username or '')")
    _MYSQL_PASSWORD=$(printf '%s' "$DSN" | python3 -c "import sys,urllib.parse; u=urllib.parse.urlparse(sys.stdin.read().strip()); print(u.password or '')")
    _MYSQL_HOST=$(printf '%s' "$DSN" | python3 -c "import sys,urllib.parse; u=urllib.parse.urlparse(sys.stdin.read().strip()); print(u.hostname or '127.0.0.1')")
    _MYSQL_PORT=$(printf '%s' "$DSN" | python3 -c "import sys,urllib.parse; u=urllib.parse.urlparse(sys.stdin.read().strip()); print(u.port or 3306)")
    _MYSQL_DATABASE=$(printf '%s' "$DSN" | python3 -c "import sys,urllib.parse; u=urllib.parse.urlparse(sys.stdin.read().strip()); print(u.path.lstrip('/'))")
fi

if [ -z "$_MYSQL_DATABASE" ]; then
    printf 'mysql.sh: refuse-to-run: database name not set (set DSN or MYSQL_DATABASE)\n' >&2
    exit 2
fi

# ── Dependency check ──────────────────────────────────────────────────────────
if ! command -v mysqldump >/dev/null 2>&1; then
    printf 'mysql.sh: fatal: mysqldump not found. Install mysql-client\n' >&2
    exit 1
fi

# ── Build connection args ─────────────────────────────────────────────────────
CONN_ARGS=(-h "$_MYSQL_HOST" -P "$_MYSQL_PORT" -u "$_MYSQL_USER")
if [ -n "$_MYSQL_PASSWORD" ]; then
    CONN_ARGS+=("-p${_MYSQL_PASSWORD}")
fi

# ── Prepare output directory ──────────────────────────────────────────────────
mkdir -p "$OUT_DIR"

# ── Schema dump (no data) ─────────────────────────────────────────────────────
printf 'mysql.sh: dumping schema...\n' >&2
if ! mysqldump "${CONN_ARGS[@]}" --no-data "$_MYSQL_DATABASE" > "$OUT_DIR/schema.sql" 2>&1; then
    printf 'mysql.sh: fatal: schema dump failed\n' >&2
    exit 1
fi

# ── Full-table data dumps ─────────────────────────────────────────────────────
if [ -n "$TABLES_FULL_CSV" ]; then
    IFS=',' read -ra FULL_TABLES <<< "$TABLES_FULL_CSV"
    for table in "${FULL_TABLES[@]}"; do
        table="${table// /}"
        [ -z "$table" ] && continue
        printf 'mysql.sh: dumping full table %s...\n' "$table" >&2
        if ! mysqldump "${CONN_ARGS[@]}" --no-create-info "$_MYSQL_DATABASE" "$table" > "$OUT_DIR/${table}.sql" 2>&1; then
            printf 'mysql.sh: fatal: data dump failed for table %s\n' "$table" >&2
            exit 1
        fi
    done
fi

# ── Sampled-table data dumps ──────────────────────────────────────────────────
if [ "$TABLES_SAMPLE_JSON" != "{}" ] && [ -n "$TABLES_SAMPLE_JSON" ]; then
    SAMPLE_TABLES=$(printf '%s' "$TABLES_SAMPLE_JSON" | \
        python3 -c "import sys,json; d=json.load(sys.stdin); [print(k,v) for k,v in d.items()]" 2>/dev/null || true)

    if command -v mysql >/dev/null 2>&1; then
        while IFS=' ' read -r table limit; do
            [ -z "$table" ] && continue
            printf 'mysql.sh: dumping sampled table %s (limit %s rows)...\n' "$table" "$limit" >&2
            mysql "${CONN_ARGS[@]}" "$_MYSQL_DATABASE" \
                -e "SELECT * FROM \`$table\` LIMIT $limit" \
                --batch --quick \
                > "$OUT_DIR/${table}.tsv" 2>&1 || {
                printf 'mysql.sh: fatal: sampled dump failed for table %s\n' "$table" >&2
                exit 1
            }
        done <<< "$SAMPLE_TABLES"
    else
        # Fallback: full dump
        while IFS=' ' read -r table _; do
            [ -z "$table" ] && continue
            printf 'mysql.sh: warning: mysql cli not available; full dump for %s\n' "$table" >&2
            if ! mysqldump "${CONN_ARGS[@]}" --no-create-info "$_MYSQL_DATABASE" "$table" > "$OUT_DIR/${table}.sql" 2>&1; then
                printf 'mysql.sh: fatal: data dump failed for table %s\n' "$table" >&2
                exit 1
            fi
        done <<< "$SAMPLE_TABLES"
    fi
fi

printf 'mysql.sh: snapshot complete → %s\n' "$OUT_DIR" >&2
