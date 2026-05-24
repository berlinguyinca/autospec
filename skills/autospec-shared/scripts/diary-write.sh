#!/usr/bin/env bash
# diary-write.sh — Write an episodic diary entry to docs/memory/diary/YYYY-MM-DD.md.
#
# Usage:
#   diary-write.sh --phase <n> --event <slug> --body <markdown-text> [--memory-dir <path>]
#
# Options:
#   --phase <n>          Phase number (1=brainstorm, 4=monitor, etc.)
#   --event <slug>       Event slug (e.g. brainstorm-locked, monitor-exit)
#   --body <text>        Markdown body of the entry
#   --memory-dir <path>  Override path for memory root (default: docs/memory in repo root)
#
# Exit codes:
#   0 always (best-effort; never block the caller)
#
# Environment:
#   AUTOSPEC_SKIP_DIARY=1              — skip entirely (no-op)
#   AUTOSPEC_SKIP_DIARY_MEMPALACE=1    — skip mempalace diary_write call
#   AUTOSPEC_SKIP_MEMPALACE_INSTALL=1  — skip mempalace auto-install
#
# Frontmatter written per entry:
#   wing: episodic
#   drawer_class: diary
#   date: YYYY-MM-DD
#   phase: <n>
#   event: <slug>
#
# Append-safe: multiple events on the same day go into one file, separated by ---.

set +e

# ── Opt-out ───────────────────────────────────────────────────────────────────
[ "${AUTOSPEC_SKIP_DIARY:-0}" = "1" ] && exit 0

# ── Parse args ────────────────────────────────────────────────────────────────
PHASE=""
EVENT=""
BODY=""
MEMORY_DIR_ARG=""

while [ $# -gt 0 ]; do
  case "$1" in
    --phase)      PHASE="$2";      shift 2 ;;
    --event)      EVENT="$2";      shift 2 ;;
    --body)       BODY="$2";       shift 2 ;;
    --memory-dir) MEMORY_DIR_ARG="$2"; shift 2 ;;
    *) shift ;;
  esac
done

# Best-effort: if required args missing, exit silently
if [ -z "$PHASE" ] || [ -z "$EVENT" ]; then
  exit 0
fi

# ── Resolve memory dir ────────────────────────────────────────────────────────
if [ -n "$MEMORY_DIR_ARG" ]; then
  MEMORY_ROOT="$MEMORY_DIR_ARG"
else
  # Try git repo root + docs/memory
  REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)
  if [ -n "$REPO_ROOT" ]; then
    MEMORY_ROOT="$REPO_ROOT/docs/memory"
  else
    # Fallback: ~/.autospec/memory
    MEMORY_ROOT="$HOME/.autospec/memory"
  fi
fi

DIARY_DIR="$MEMORY_ROOT/diary"
mkdir -p "$DIARY_DIR" 2>/dev/null || true

TODAY=$(date -u +%Y-%m-%d)
TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
DIARY_FILE="$DIARY_DIR/$TODAY.md"

# ── Write entry ───────────────────────────────────────────────────────────────
# Each entry is a YAML frontmatter block followed by the body.
# Append-safe: if the file already exists, add a separator first.

if [ -f "$DIARY_FILE" ]; then
  # Separator between entries
  printf '\n---\n' >> "$DIARY_FILE"
fi

cat >> "$DIARY_FILE" <<ENTRY
---
wing: episodic
drawer_class: diary
date: ${TODAY}
ts: ${TS}
phase: ${PHASE}
event: ${EVENT}
---

${BODY}
ENTRY

# ── Optional: mempalace diary_write ──────────────────────────────────────────
if [ "${AUTOSPEC_SKIP_DIARY_MEMPALACE:-0}" != "1" ] && command -v mempalace > /dev/null 2>&1; then
  mempalace diary_write \
    --date "$TODAY" \
    --phase "$PHASE" \
    --event "$EVENT" \
    --body "$BODY" \
    2>/dev/null || true
fi

exit 0
