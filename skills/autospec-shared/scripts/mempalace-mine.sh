#!/usr/bin/env bash
# mempalace-mine.sh — Mine existing memory files and inject wing + drawer_class frontmatter.
#
# Iterates *.md files under --root, reads metadata.type:, injects wing: and drawer_class:
# lines per spec §3b inference table. Idempotent: skips files already having wing:.
# Best-effort mempalace kg-rebuild after the loop (silent fallback if CLI absent).
#
# Usage:
#   mempalace-mine.sh --root <dir>            # required: memory root dir
#   mempalace-mine.sh --root <dir> --quiet    # suppress per-file logging
#
# Exit codes:
#   0 always (failures are warned but non-blocking)
#
# Inference table (spec §3b):
#   metadata.type: feedback  → wing: synthesis,  drawer_class: lesson
#   metadata.type: project   → wing: episodic,   drawer_class: session-log
#   metadata.type: reference → wing: semantic,   drawer_class: reference
#   metadata.type: user      → wing: semantic,   drawer_class: reference
#   (default/unknown)        → wing: synthesis,  drawer_class: lesson
#
# Requires: bash 3.2+, awk, grep, cp, mv

set +e

ROOT=""
QUIET=0

# ── Argument parse ────────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
  case "$1" in
    --root)
      ROOT="$2"
      shift 2
      ;;
    --quiet)
      QUIET=1
      shift
      ;;
    *)
      echo "mempalace-mine: unknown argument: $1" >&2
      shift
      ;;
  esac
done

if [ -z "$ROOT" ]; then
  echo "mempalace-mine: --root is required" >&2
  exit 0
fi

if [ ! -d "$ROOT" ]; then
  echo "mempalace-mine: root dir not found: $ROOT" >&2
  exit 0
fi

# ── Inference table ───────────────────────────────────────────────────────────
# Returns wing and drawer_class for a given metadata.type value
_infer_wing_drawer() {
  local type_val="$1"
  case "$type_val" in
    feedback)
      echo "synthesis" "lesson"
      ;;
    project)
      echo "episodic" "session-log"
      ;;
    reference)
      echo "semantic" "reference"
      ;;
    user)
      echo "semantic" "reference"
      ;;
    *)
      # Default: synthesis + lesson
      echo "synthesis" "lesson"
      ;;
  esac
}

# ── Inject frontmatter lines via awk ─────────────────────────────────────────
# Inserts "  wing: <w>" and "  drawer_class: <d>" after the "  type:" line if present,
# or after the "metadata:" line if there is no type: field.
_inject_frontmatter() {
  local file="$1"
  local wing="$2"
  local drawer="$3"
  local has_type
  has_type=$(grep -c "^  type:" "$file" 2>/dev/null || echo 0)
  local tmp
  tmp=$(mktemp /tmp/mempalace-mine-XXXXXX.md)

  if [ "$has_type" -gt 0 ]; then
    # Insert after the type: line
    awk -v wing="$wing" -v drawer="$drawer" '
      /^  type:/ {
        print
        print "  wing: " wing
        print "  drawer_class: " drawer
        next
      }
      { print }
    ' "$file" > "$tmp" && mv "$tmp" "$file" || rm -f "$tmp"
  else
    # No type: line — insert after metadata: line
    awk -v wing="$wing" -v drawer="$drawer" '
      /^metadata:/ {
        print
        print "  wing: " wing
        print "  drawer_class: " drawer
        next
      }
      { print }
    ' "$file" > "$tmp" && mv "$tmp" "$file" || rm -f "$tmp"
  fi
}

# ── Main loop ────────────────────────────────────────────────────────────────
processed=0
skipped=0

for md_file in "$ROOT"/*.md; do
  # Skip if glob matched nothing
  [ -f "$md_file" ] || continue

  # Skip MEMORY.md index file
  basename_file=$(basename "$md_file")
  [ "$basename_file" = "MEMORY.md" ] && continue

  # Idempotency guard: skip if already has wing: in frontmatter
  if grep -q "^  wing:" "$md_file" 2>/dev/null; then
    [ "$QUIET" -eq 0 ] && echo "mempalace-mine: skip (already mined): $basename_file"
    skipped=$(( skipped + 1 ))
    continue
  fi

  # Extract metadata.type value (look for "  type: <val>" in frontmatter)
  type_val=$(grep "^  type:" "$md_file" 2>/dev/null | head -1 | sed 's/^  type:[[:space:]]*//' | tr -d '\r')

  # Infer wing + drawer_class
  read -r wing drawer <<< "$(_infer_wing_drawer "$type_val")"

  [ "$QUIET" -eq 0 ] && echo "mempalace-mine: processing $basename_file (type=$type_val → wing=$wing, drawer_class=$drawer)"

  _inject_frontmatter "$md_file" "$wing" "$drawer"
  processed=$(( processed + 1 ))
done

[ "$QUIET" -eq 0 ] && echo "mempalace-mine: done — processed=$processed skipped=$skipped"

# ── Best-effort KG rebuild ────────────────────────────────────────────────────
if command -v mempalace > /dev/null 2>&1; then
  mempalace kg-rebuild --root "$ROOT" 2>/dev/null || true
fi

exit 0
