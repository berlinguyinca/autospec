#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/snapshot/sqlite.sh — SQLite snapshot driver
#
# Usage: sqlite.sh <src_path> <out_dir>
#
# Copies the source SQLite database file to <out_dir>/snapshot.db and runs
# VACUUM to compact it.
#
# Exit codes:
#   0 = ok
#   1 = fatal
#   2 = refuse-to-run

set -euo pipefail

SRC_PATH="${1:-}"
OUT_DIR="${2:-}"

# ── Validate arguments ────────────────────────────────────────────────────────
if [ -z "$SRC_PATH" ]; then
    printf 'sqlite.sh: fatal: SRC_PATH argument required\n' >&2
    exit 1
fi

if [ -z "$OUT_DIR" ]; then
    printf 'sqlite.sh: fatal: OUT_DIR argument required\n' >&2
    exit 1
fi

if [ ! -f "$SRC_PATH" ]; then
    printf 'sqlite.sh: refuse-to-run: source database not found: %s\n' "$SRC_PATH" >&2
    exit 2
fi

# ── Prepare output directory ──────────────────────────────────────────────────
mkdir -p "$OUT_DIR"

SNAP_DB="$OUT_DIR/snapshot.db"

# ── Copy database ─────────────────────────────────────────────────────────────
printf 'sqlite.sh: copying %s → %s...\n' "$SRC_PATH" "$SNAP_DB" >&2
cp "$SRC_PATH" "$SNAP_DB"

# ── VACUUM to compact ─────────────────────────────────────────────────────────
if command -v sqlite3 >/dev/null 2>&1; then
    printf 'sqlite.sh: running VACUUM...\n' >&2
    sqlite3 "$SNAP_DB" "VACUUM;" 2>&1 || {
        printf 'sqlite.sh: warning: VACUUM failed (non-fatal)\n' >&2
    }
else
    printf 'sqlite.sh: warning: sqlite3 not found; skipping VACUUM\n' >&2
fi

printf 'sqlite.sh: snapshot complete → %s\n' "$SNAP_DB" >&2
