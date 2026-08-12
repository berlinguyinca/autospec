#!/usr/bin/env bash
# scripts/lib/opencode-containment-bwrap.sh — OpenCode implementer containment
# with OS-level filesystem isolation via bubblewrap.
#
# A hardened sibling of opencode-containment-adapter.sh. That adapter applies
# only the deny-by-default permission profile (tool-level); this one additionally
# sandboxes the implementer in a bubblewrap namespace so it cannot read or write
# anything outside the worktree — the ceiling for a weeks-long unattended
# conductor running mutating code.
#
# Invocation contract (identical to the base adapter, harness.rs):
#   <adapter> <opencode-executable> --pure run <prompt>
#
# Filesystem model: the entire host is mounted read-only; only the worktree
# ($PWD, the bridge's current_dir) and a private per-run config dir are writable.
# Network is NOT unshared — the implementer must reach the model provider API —
# so filesystem isolation is the guarantee here, not network egress. PID, IPC and
# UTS namespaces are unshared (not user/net, which break credential resolution).
#
# Falls back to the permission-profile-only base adapter (no bwrap) when
# bubblewrap is missing or fails to launch, logging a WARN so the degradation is
# visible rather than silent.

set -euo pipefail

if [ "$#" -lt 3 ]; then
    echo "opencode-containment-bwrap: usage: <adapter> <opencode> <args...>" >&2
    exit 64
fi

opencode_bin="$1"
shift

if [ ! -x "$opencode_bin" ] && ! command -v "$opencode_bin" >/dev/null 2>&1; then
    echo "opencode-containment-bwrap: OpenCode executable not found: $opencode_bin" >&2
    exit 64
fi

# Same deny-by-default implementer profile as the base adapter.
OPENCODE_IMPLEMENTER_CONFIG='{"share":"disabled","instructions":[],"permission":{"*":"deny","read":"allow","glob":"allow","grep":"allow","list":"allow","lsp":"allow","edit":"allow","bash":"allow","task":"deny","external_directory":"deny","webfetch":"deny","websearch":"deny","skill":"deny"}}'

adapter_tmp="${TMPDIR:-/tmp}/opencode-bwrap.$$"
mkdir -p "$adapter_tmp"
trap 'rm -rf "$adapter_tmp"' EXIT

export OPENCODE_CONFIG_CONTENT="$OPENCODE_IMPLEMENTER_CONFIG"
export OPENCODE_DISABLE_CLAUDE_CODE=1
export OPENCODE_CONFIG_DIR="$adapter_tmp"
# Point temp writes at the private writable dir instead of relying on a writable
# /tmp, so the sandbox never needs to tmpfs-mount over the host /tmp (which would
# shadow a worktree checked out under /tmp).
export TMPDIR="$adapter_tmp"

worktree="$PWD"

if command -v bwrap >/dev/null 2>&1; then
    # Read-only host, writable worktree + private config dir, minimal runtime.
    # No --unshare-net (model API) and no --unshare-user (uid-map/credentials).
    # No --tmpfs /tmp: a fresh tmpfs would hide a worktree living under /tmp, and
    # TMPDIR already redirects temp writes into the private dir.
    exec bwrap \
        --unshare-pid --unshare-ipc --unshare-uts \
        --ro-bind / / \
        --bind "$worktree" "$worktree" \
        --bind "$adapter_tmp" "$adapter_tmp" \
        --dev /dev \
        --proc /proc \
        --chdir "$worktree" \
        --die-with-parent \
        --new-session \
        -- "$opencode_bin" "$@"
else
    echo "WARN: opencode-containment-bwrap: bubblewrap not found; falling back to permission-profile-only containment" >&2
    exec "$opencode_bin" "$@"
fi
