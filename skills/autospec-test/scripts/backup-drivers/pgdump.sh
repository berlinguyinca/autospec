#!/usr/bin/env bash
# backup-drivers/pgdump.sh — pg_dump/pg_restore backup driver for Mode II.
#
# Interface:
#   pgdump.sh snapshot  → pg_dump to dump file; echoes dump path; exit 0
#   pgdump.sh verify    → checks dump file exists; exit 0
#   pgdump.sh restore   → pg_restore from dump file; exit 0
#
# Environment:
#   AUTOSPEC_PG_DUMP_FILE   — path to dump file (default: /tmp/autospec-pre-test.dump)
#   AUTOSPEC_PG_DB          — database name (or PGDATABASE)
#   AUTOSPEC_PG_RESTORE_CMD — optional override for full restore command
#
# Exit codes: 0=success, 1=failure

set -eu

SUBCOMMAND="${1:-}"
DUMP_FILE="${AUTOSPEC_PG_DUMP_FILE:-/tmp/autospec-pre-test.dump}"
DB="${AUTOSPEC_PG_DB:-${PGDATABASE:-}}"

case "$SUBCOMMAND" in
    snapshot)
        if ! command -v pg_dump >/dev/null 2>&1; then
            printf 'pgdump-driver: pg_dump not found\n' >&2
            exit 1
        fi
        if [ -z "$DB" ]; then
            printf 'pgdump-driver: AUTOSPEC_PG_DB or PGDATABASE must be set\n' >&2
            exit 1
        fi
        if ! pg_dump "$DB" -Fc -f "$DUMP_FILE"; then
            printf 'pgdump-driver: pg_dump failed\n' >&2
            exit 1
        fi
        printf '%s\n' "$DUMP_FILE"
        ;;

    verify)
        if [ ! -f "$DUMP_FILE" ]; then
            printf 'pgdump-driver: dump file not found: %s\n' "$DUMP_FILE" >&2
            exit 1
        fi
        # Quick magic-byte check: pg_dump custom format starts with PGDMP
        if ! head -c 5 "$DUMP_FILE" | grep -q 'PGDMP'; then
            printf 'pgdump-driver: dump file appears corrupt: %s\n' "$DUMP_FILE" >&2
            exit 1
        fi
        printf 'pgdump-driver: dump verified: %s\n' "$DUMP_FILE"
        ;;

    restore)
        if [ -n "${AUTOSPEC_PG_RESTORE_CMD:-}" ]; then
            if ! eval "$AUTOSPEC_PG_RESTORE_CMD"; then
                printf 'pgdump-driver: custom restore command failed\n' >&2
                exit 1
            fi
        else
            if ! command -v pg_restore >/dev/null 2>&1; then
                printf 'pgdump-driver: pg_restore not found\n' >&2
                exit 1
            fi
            if [ -z "$DB" ]; then
                printf 'pgdump-driver: AUTOSPEC_PG_DB or PGDATABASE must be set\n' >&2
                exit 1
            fi
            if ! pg_restore -d "$DB" --clean --if-exists "$DUMP_FILE"; then
                printf 'pgdump-driver: pg_restore failed\n' >&2
                exit 1
            fi
        fi
        printf 'pgdump-driver: restore complete\n'
        ;;

    *)
        printf 'pgdump-driver: unknown subcommand: %s\n' "$SUBCOMMAND" >&2
        printf 'Usage: pgdump.sh snapshot|verify|restore\n' >&2
        exit 1
        ;;
esac
