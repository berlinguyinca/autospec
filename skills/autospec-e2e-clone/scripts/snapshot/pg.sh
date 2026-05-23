#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/snapshot/pg.sh — PostgreSQL snapshot driver
#
# Usage: pg.sh <tables_full_csv> <tables_sample_json> <out_dir>
#
# Environment:
#   DSN — PostgreSQL connection DSN (e.g. postgres://user:pass@host:5432/db)
#
# Output layout:
#   <out_dir>/schema.sql          — full schema dump
#   <out_dir>/<table>.sql         — per-table data dump (tables_full + tables_sample keys)
#
# Exit codes:
#   0 = ok
#   1 = fatal (missing tool, bad DSN, pg_dump error)
#   2 = refuse-to-run (operator-actionable config error)

set -euo pipefail

TABLES_FULL_CSV="${1:-}"
TABLES_SAMPLE_JSON="${2:-{}}"
OUT_DIR="${3:-}"

# ── Validate arguments ────────────────────────────────────────────────────────
if [ -z "$OUT_DIR" ]; then
    printf 'pg.sh: fatal: OUT_DIR argument required\n' >&2
    exit 1
fi

if [ -z "${DSN:-}" ]; then
    printf 'pg.sh: refuse-to-run: DSN environment variable not set\n' >&2
    printf 'Set DSN to a valid PostgreSQL connection string\n' >&2
    exit 2
fi

# ── Dependency check ──────────────────────────────────────────────────────────
if ! command -v pg_dump >/dev/null 2>&1; then
    printf 'pg.sh: fatal: pg_dump not found. Install postgresql-client\n' >&2
    exit 1
fi

# ── Prepare output directory ──────────────────────────────────────────────────
mkdir -p "$OUT_DIR"

# ── Schema dump ───────────────────────────────────────────────────────────────
printf 'pg.sh: dumping schema...\n' >&2
if ! pg_dump --schema-only "$DSN" > "$OUT_DIR/schema.sql" 2>&1; then
    printf 'pg.sh: fatal: schema dump failed\n' >&2
    exit 1
fi

# ── Full-table data dumps ─────────────────────────────────────────────────────
if [ -n "$TABLES_FULL_CSV" ]; then
    IFS=',' read -ra FULL_TABLES <<< "$TABLES_FULL_CSV"
    for table in "${FULL_TABLES[@]}"; do
        table="${table// /}"  # trim whitespace
        [ -z "$table" ] && continue
        printf 'pg.sh: dumping full table %s...\n' "$table" >&2
        if ! pg_dump --data-only --table="$table" "$DSN" > "$OUT_DIR/${table}.sql" 2>&1; then
            printf 'pg.sh: fatal: data dump failed for table %s\n' "$table" >&2
            exit 1
        fi
    done
fi

# ── Sampled-table data dumps ──────────────────────────────────────────────────
# tables_sample is JSON object: {"events": 10000, "logs": 5000}
if [ "$TABLES_SAMPLE_JSON" != "{}" ] && [ -n "$TABLES_SAMPLE_JSON" ]; then
    SAMPLE_TABLES=$(printf '%s' "$TABLES_SAMPLE_JSON" | \
        python3 -c "import sys,json; d=json.load(sys.stdin); [print(k,v) for k,v in d.items()]" 2>/dev/null || true)

    while IFS=' ' read -r table limit; do
        [ -z "$table" ] && continue
        printf 'pg.sh: dumping sampled table %s (limit %s rows)...\n' "$table" "$limit" >&2
        # Use pg_dump with --where for row-count limiting via CTID sampling approximation
        # For sampling, we write a temp view then dump it
        # Simpler approach: dump with row limit via psql COPY
        if command -v psql >/dev/null 2>&1; then
            psql "$DSN" -c "\COPY (SELECT * FROM \"$table\" LIMIT $limit) TO '$OUT_DIR/${table}.csv' CSV HEADER" 2>&1 || {
                printf 'pg.sh: fatal: sampled dump failed for table %s\n' "$table" >&2
                exit 1
            }
        else
            # Fallback: full dump if psql not available
            printf 'pg.sh: warning: psql not available; falling back to full dump for %s\n' "$table" >&2
            if ! pg_dump --data-only --table="$table" "$DSN" > "$OUT_DIR/${table}.sql" 2>&1; then
                printf 'pg.sh: fatal: data dump failed for table %s\n' "$table" >&2
                exit 1
            fi
        fi
    done <<< "$SAMPLE_TABLES"
fi

printf 'pg.sh: snapshot complete → %s\n' "$OUT_DIR" >&2
