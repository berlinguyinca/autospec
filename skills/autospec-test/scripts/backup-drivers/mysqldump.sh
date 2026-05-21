#!/usr/bin/env bash
# backup-drivers/mysqldump.sh — mysqldump/mysql backup driver for Mode II.
#
# Interface:
#   mysqldump.sh snapshot  → mysqldump to file; echoes dump path; exit 0
#   mysqldump.sh verify    → checks dump file exists; exit 0
#   mysqldump.sh restore   → mysql < dump file; exit 0
#
# Environment:
#   AUTOSPEC_MYSQL_DUMP_FILE   — path to dump file (default: /tmp/autospec-pre-test.sql)
#   AUTOSPEC_MYSQL_DB          — database name
#   AUTOSPEC_MYSQL_RESTORE_CMD — optional override for full restore command
#   Standard MySQL env vars (MYSQL_HOST, MYSQL_USER, MYSQL_PWD, etc.) are honored.
#
# Exit codes: 0=success, 1=failure

set -eu

SUBCOMMAND="${1:-}"
DUMP_FILE="${AUTOSPEC_MYSQL_DUMP_FILE:-/tmp/autospec-pre-test.sql}"
DB="${AUTOSPEC_MYSQL_DB:-}"

case "$SUBCOMMAND" in
    snapshot)
        if ! command -v mysqldump >/dev/null 2>&1; then
            printf 'mysqldump-driver: mysqldump not found\n' >&2
            exit 1
        fi
        if [ -z "$DB" ]; then
            printf 'mysqldump-driver: AUTOSPEC_MYSQL_DB must be set\n' >&2
            exit 1
        fi
        if ! mysqldump "$DB" > "$DUMP_FILE"; then
            printf 'mysqldump-driver: mysqldump failed\n' >&2
            exit 1
        fi
        printf '%s\n' "$DUMP_FILE"
        ;;

    verify)
        if [ ! -f "$DUMP_FILE" ]; then
            printf 'mysqldump-driver: dump file not found: %s\n' "$DUMP_FILE" >&2
            exit 1
        fi
        # Minimal check: dump should start with a comment or SQL keyword
        if ! head -c 20 "$DUMP_FILE" | grep -qiE '(--.*mysql|CREATE|INSERT|SET)'; then
            printf 'mysqldump-driver: dump file appears empty or corrupt: %s\n' "$DUMP_FILE" >&2
            exit 1
        fi
        printf 'mysqldump-driver: dump verified: %s\n' "$DUMP_FILE"
        ;;

    restore)
        if [ -n "${AUTOSPEC_MYSQL_RESTORE_CMD:-}" ]; then
            if ! eval "$AUTOSPEC_MYSQL_RESTORE_CMD"; then
                printf 'mysqldump-driver: custom restore command failed\n' >&2
                exit 1
            fi
        else
            if ! command -v mysql >/dev/null 2>&1; then
                printf 'mysqldump-driver: mysql not found\n' >&2
                exit 1
            fi
            if [ -z "$DB" ]; then
                printf 'mysqldump-driver: AUTOSPEC_MYSQL_DB must be set\n' >&2
                exit 1
            fi
            if ! mysql "$DB" < "$DUMP_FILE"; then
                printf 'mysqldump-driver: mysql restore failed\n' >&2
                exit 1
            fi
        fi
        printf 'mysqldump-driver: restore complete\n'
        ;;

    *)
        printf 'mysqldump-driver: unknown subcommand: %s\n' "$SUBCOMMAND" >&2
        printf 'Usage: mysqldump.sh snapshot|verify|restore\n' >&2
        exit 1
        ;;
esac
