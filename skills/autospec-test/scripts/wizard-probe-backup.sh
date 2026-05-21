#!/usr/bin/env bash
# wizard-probe-backup.sh — detect available backup driver binaries for Mode II.
#
# Probes for known backup driver binaries in order of preference.
# Prints the first detected driver name to stdout and exits 0.
# If none found, exits 1 with diagnostic on stderr.
#
# Exit codes:
#   0 = driver found (driver name printed to stdout)
#   1 = no supported backup driver found on PATH

set -eu

# Probe order: zfs first (best isolation), then pgdump, mysqldump, custom
DRIVERS=(
    "zfs:zfs"
    "pgdump:pg_dump"
    "mysqldump:mysqldump"
)

for entry in "${DRIVERS[@]}"; do
    driver_name="${entry%%:*}"
    binary="${entry##*:}"
    if command -v "$binary" >/dev/null 2>&1; then
        printf '%s\n' "$driver_name"
        exit 0
    fi
done

printf 'wizard-probe-backup: no supported backup driver found on PATH\n' >&2
printf 'Install one of: zfs, pg_dump (postgresql-client), mysqldump (mysql-client)\n' >&2
printf 'Or use driver: custom with custom_snapshot_cmd / custom_restore_cmd\n' >&2
exit 1
