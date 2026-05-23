#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/snapshot/custom.sh — Custom snapshot driver
#
# Usage: custom.sh <cmd> <out_dir>
#
# Invokes the operator-provided command with <out_dir> as $1.
# Non-zero exit from <cmd> causes snapshot failure.
#
# Exit codes:
#   0 = ok (cmd exited 0)
#   1 = fatal (missing args)
#   2 = refuse-to-run (cmd not found)
#   <cmd exit code> = propagated on cmd failure

set -euo pipefail

CMD="${1:-}"
OUT_DIR="${2:-}"

# ── Validate arguments ────────────────────────────────────────────────────────
if [ -z "$CMD" ]; then
    printf 'custom.sh: fatal: CMD argument required\n' >&2
    exit 1
fi

if [ -z "$OUT_DIR" ]; then
    printf 'custom.sh: fatal: OUT_DIR argument required\n' >&2
    exit 1
fi

# ── Prepare output directory ──────────────────────────────────────────────────
mkdir -p "$OUT_DIR"

# ── Validate cmd exists ───────────────────────────────────────────────────────
# Extract binary name (first token) for existence check
CMD_BIN="${CMD%% *}"
if [ ! -x "$CMD_BIN" ] && ! command -v "$CMD_BIN" >/dev/null 2>&1; then
    printf 'custom.sh: refuse-to-run: command not found: %s\n' "$CMD_BIN" >&2
    printf 'Ensure the snapshot_cmd binary is installed and on PATH\n' >&2
    exit 2
fi

# ── Execute operator command ──────────────────────────────────────────────────
printf 'custom.sh: executing: %s %s\n' "$CMD" "$OUT_DIR" >&2
eval "$CMD" "$OUT_DIR"
rc=$?

if [ "$rc" -ne 0 ]; then
    printf 'custom.sh: fatal: command exited with code %d\n' "$rc" >&2
    exit "$rc"
fi

printf 'custom.sh: snapshot complete → %s\n' "$OUT_DIR" >&2
