#!/usr/bin/env bash
# backup-drivers/custom.sh — operator-provided custom backup driver for Mode II.
#
# Interface:
#   custom.sh snapshot  → runs AUTOSPEC_CUSTOM_SNAPSHOT_CMD; echoes snapshot id; exit 0 on success
#   custom.sh verify    → runs AUTOSPEC_CUSTOM_VERIFY_CMD; exit 0 if verified
#   custom.sh restore   → runs AUTOSPEC_CUSTOM_RESTORE_CMD; exit 0 on success
#
# Environment:
#   AUTOSPEC_CUSTOM_SNAPSHOT_CMD  — shell command to take snapshot
#   AUTOSPEC_CUSTOM_VERIFY_CMD    — shell command to verify snapshot exists
#   AUTOSPEC_CUSTOM_RESTORE_CMD   — shell command to restore from snapshot
#
# Exit codes:
#   0 = success
#   1 = failure (command failed or misconfigured)

set -eu

SUBCOMMAND="${1:-}"

case "$SUBCOMMAND" in
    snapshot)
        if [ -z "${AUTOSPEC_CUSTOM_SNAPSHOT_CMD:-}" ]; then
            printf 'custom-driver: AUTOSPEC_CUSTOM_SNAPSHOT_CMD is not set\n' >&2
            exit 1
        fi
        if ! eval "$AUTOSPEC_CUSTOM_SNAPSHOT_CMD"; then
            printf 'custom-driver: snapshot command failed\n' >&2
            exit 1
        fi
        # Emit snapshot id: timestamp-based for cp-style drivers
        SNAP_ID="custom-snap-$(date -u +%s)"
        printf '%s\n' "$SNAP_ID"
        ;;

    verify)
        if [ -z "${AUTOSPEC_CUSTOM_VERIFY_CMD:-}" ]; then
            printf 'custom-driver: AUTOSPEC_CUSTOM_VERIFY_CMD is not set\n' >&2
            exit 1
        fi
        if ! eval "$AUTOSPEC_CUSTOM_VERIFY_CMD"; then
            printf 'custom-driver: verify command failed — backup not found or invalid\n' >&2
            exit 1
        fi
        printf 'custom-driver: snapshot verified\n'
        ;;

    restore)
        if [ -z "${AUTOSPEC_CUSTOM_RESTORE_CMD:-}" ]; then
            printf 'custom-driver: AUTOSPEC_CUSTOM_RESTORE_CMD is not set\n' >&2
            exit 1
        fi
        if ! eval "$AUTOSPEC_CUSTOM_RESTORE_CMD"; then
            printf 'custom-driver: restore command failed\n' >&2
            exit 1
        fi
        printf 'custom-driver: restore complete\n'
        ;;

    *)
        printf 'custom-driver: unknown subcommand: %s\n' "$SUBCOMMAND" >&2
        printf 'Usage: custom.sh snapshot|verify|restore\n' >&2
        exit 1
        ;;
esac
