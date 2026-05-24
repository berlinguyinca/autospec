#!/usr/bin/env bash
# cross-repo-index.sh — Register current repo into ~/.autospec/memory-index/<slug>
#
# Idempotent: re-running produces the same result.
# Also performs GC: stale entries whose docs/memory/ no longer exists are purged.
#
# Usage:
#   cross-repo-index.sh [--repo-root DIR]
#
# Options:
#   --repo-root DIR   Override repo root (default: git rev-parse --show-toplevel)
#   --index-dir DIR   Override index dir (default: ~/.autospec/memory-index)
#   --help            Show usage and exit 0
#
# Exit codes:
#   0 always (best-effort; errors are silent)
#
# Environment:
#   AUTOSPEC_SKIP_CROSS_REPO_INDEX=1  — skip registration silently (opt-out)

set +e

# ── Parse args ────────────────────────────────────────────────────────────────
REPO_ROOT=""
INDEX_DIR=""

while [ $# -gt 0 ]; do
  case "$1" in
    --repo-root)
      REPO_ROOT="$2"
      shift 2
      ;;
    --index-dir)
      INDEX_DIR="$2"
      shift 2
      ;;
    --help|-h)
      printf 'Usage: cross-repo-index.sh [--repo-root DIR] [--index-dir DIR]\n'
      exit 0
      ;;
    *)
      shift
      ;;
  esac
done

# ── Opt-out ───────────────────────────────────────────────────────────────────
[ "${AUTOSPEC_SKIP_CROSS_REPO_INDEX:-0}" = "1" ] && exit 0

# ── Resolve repo root ─────────────────────────────────────────────────────────
if [ -z "$REPO_ROOT" ]; then
  REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)
fi
if [ -z "$REPO_ROOT" ]; then
  exit 0
fi

MEMORY_DIR="$REPO_ROOT/docs/memory"

# ── Resolve index dir ─────────────────────────────────────────────────────────
if [ -z "$INDEX_DIR" ]; then
  INDEX_DIR="${HOME}/.autospec/memory-index"
fi
mkdir -p "$INDEX_DIR"

# ── GC: remove stale entries ──────────────────────────────────────────────────
# A symlink is stale if its target docs/memory/ dir no longer exists.
if [ -d "$INDEX_DIR" ]; then
  for _link in "$INDEX_DIR"/*; do
    [ -L "$_link" ] || continue
    _target=$(readlink "$_link" 2>/dev/null)
    if [ -z "$_target" ] || [ ! -d "$_target" ]; then
      rm -f "$_link"
    fi
  done
fi

# ── Register current repo ─────────────────────────────────────────────────────
# Only register if docs/memory/ exists
[ -d "$MEMORY_DIR" ] || exit 0

# Compute slug: repo root path with slashes → dashes, strip leading dash
_slug=$(printf '%s' "$REPO_ROOT" | sed 's|^/||' | tr '/' '-')
_link_path="$INDEX_DIR/$_slug"

# Idempotent: skip if symlink already points to the right target
if [ -L "$_link_path" ] && [ "$(readlink "$_link_path")" = "$MEMORY_DIR" ]; then
  exit 0
fi

# Remove stale link for this slug (different target) if present
[ -L "$_link_path" ] && rm -f "$_link_path"
[ -e "$_link_path" ] && rm -rf "$_link_path"

ln -s "$MEMORY_DIR" "$_link_path"

exit 0
