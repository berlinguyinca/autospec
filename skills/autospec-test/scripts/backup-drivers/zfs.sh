#!/usr/bin/env bash
# backup-drivers/zfs.sh — ZFS backup driver for Mode II scoped-production.
#
# Interface:
#   zfs.sh snapshot  → creates ZFS snapshot; echoes snapshot id; exit 0 on success
#   zfs.sh verify    → verifies snapshot exists; exit 0 if ok
#   zfs.sh restore   → rolls back to snapshot; exit 0 on success
#
# Environment / contract fields (resolved by preflight):
#   AUTOSPEC_ZFS_DATASET    — e.g. "tank/db/prod"
#   AUTOSPEC_ZFS_SNAP_NAME  — snapshot tag (default: e2e-pre)
#
# Exit codes:
#   0 = success
#   1 = failure

set -eu

SUBCOMMAND="${1:-}"
DATASET="${AUTOSPEC_ZFS_DATASET:-}"
SNAP_TAG="${AUTOSPEC_ZFS_SNAP_NAME:-e2e-pre}"

if [ -z "$DATASET" ]; then
    printf 'zfs-driver: AUTOSPEC_ZFS_DATASET is not set\n' >&2
    exit 1
fi

SNAP_ID="${DATASET}@${SNAP_TAG}"

case "$SUBCOMMAND" in
    snapshot)
        if ! zfs snapshot "$SNAP_ID"; then
            printf 'zfs-driver: snapshot failed: %s\n' "$SNAP_ID" >&2
            exit 1
        fi
        printf '%s\n' "$SNAP_ID"
        ;;

    verify)
        if ! zfs list "$SNAP_ID" >/dev/null 2>&1; then
            printf 'zfs-driver: snapshot not found: %s\n' "$SNAP_ID" >&2
            exit 1
        fi
        printf 'zfs-driver: snapshot verified: %s\n' "$SNAP_ID"
        ;;

    restore)
        if ! zfs rollback "$SNAP_ID"; then
            printf 'zfs-driver: rollback failed: %s\n' "$SNAP_ID" >&2
            exit 1
        fi
        printf 'zfs-driver: restore complete: %s\n' "$SNAP_ID"
        ;;

    *)
        printf 'zfs-driver: unknown subcommand: %s\n' "$SUBCOMMAND" >&2
        printf 'Usage: zfs.sh snapshot|verify|restore\n' >&2
        exit 1
        ;;
esac
