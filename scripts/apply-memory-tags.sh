#!/usr/bin/env bash
# apply-memory-tags.sh — Idempotent frontmatter tagger for autospec memory files.
# Reads scripts/memory-tags.yml and prepends YAML frontmatter (tags:) to each
# matching feedback_*.md file under AUTOSPEC_MEMORY_DIR.
#
# Usage:
#   bash scripts/apply-memory-tags.sh [--dry-run] [--memory-dir DIR] [--manifest FILE]
#
# Options:
#   --dry-run       Print planned edits without writing any files
#   --memory-dir    Override AUTOSPEC_MEMORY_DIR (default: auto-detect)
#   --manifest      Path to memory-tags.yml (default: scripts/memory-tags.yml beside this script)
#
# Exit codes:
#   0  All files processed (or dry-run completed)
#   1  Manifest file not found
#   2  Memory directory not found

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
DRY_RUN=false
MANIFEST="${SCRIPT_DIR}/memory-tags.yml"
MEMORY_DIR="${AUTOSPEC_MEMORY_DIR:-}"

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      DRY_RUN=true
      shift
      ;;
    --memory-dir)
      MEMORY_DIR="$2"
      shift 2
      ;;
    --manifest)
      MANIFEST="$2"
      shift 2
      ;;
    -h|--help)
      grep '^#' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

# Resolve memory dir
if [[ -z "$MEMORY_DIR" ]]; then
  # Auto-detect: look for the autospec project memory dir
  CANDIDATE="$HOME/.claude/projects/-Users-$(whoami)-IdeaProjects-autospec/memory"
  if [[ -d "$CANDIDATE" ]]; then
    MEMORY_DIR="$CANDIDATE"
  else
    # Fallback: search common patterns
    CANDIDATE2=$(find "$HOME/.claude/projects" -maxdepth 2 -name "memory" -type d 2>/dev/null | grep -i autospec | head -1)
    if [[ -n "$CANDIDATE2" ]]; then
      MEMORY_DIR="$CANDIDATE2"
    fi
  fi
fi

if [[ -z "$MEMORY_DIR" || ! -d "$MEMORY_DIR" ]]; then
  echo "ERROR: Memory directory not found. Set AUTOSPEC_MEMORY_DIR or use --memory-dir." >&2
  exit 2
fi

if [[ ! -f "$MANIFEST" ]]; then
  echo "ERROR: Manifest not found: $MANIFEST" >&2
  exit 1
fi

echo "Memory dir : $MEMORY_DIR"
echo "Manifest   : $MANIFEST"
if [[ "$DRY_RUN" == "true" ]]; then
  echo "Mode       : DRY RUN (no files will be written)"
fi
echo ""

# Parse the YAML manifest and apply tags
# Format: filename.md: / tags: [t1, t2, ...]
current_file=""
processed=0
skipped=0
warned=0

while IFS= read -r line; do
  # Skip comments and blank lines
  [[ "$line" =~ ^[[:space:]]*# ]] && continue
  [[ -z "${line//[[:space:]]/}" ]] && continue

  # File entry: "feedback_something.md:"
  if [[ "$line" =~ ^(feedback_[^:]+\.md):$ ]]; then
    current_file="${BASH_REMATCH[1]}"
    continue
  fi

  # Tags line: "  tags: [t1, t2, ...]"
  if [[ -n "$current_file" && "$line" =~ ^[[:space:]]+tags:[[:space:]]*(\[.+\]) ]]; then
    tags_value="${BASH_REMATCH[1]}"
    target="$MEMORY_DIR/$current_file"

    if [[ ! -f "$target" ]]; then
      echo "WARN  $current_file — file not found, skipping" >&2
      warned=$((warned + 1))
      current_file=""
      continue
    fi

    # Idempotency check: skip if already has 'tags:' in first 10 lines
    if head -10 "$target" | grep -q "^tags:"; then
      echo "SKIP  $current_file — already tagged"
      skipped=$((skipped + 1))
      current_file=""
      continue
    fi

    # Build frontmatter block to prepend
    frontmatter="tags: ${tags_value}"

    if [[ "$DRY_RUN" == "true" ]]; then
      echo "PLAN  $current_file — would add: $frontmatter"
    else
      # Prepend tags line to existing frontmatter or add at top
      # If file starts with '---', insert after that line
      first_line=$(head -1 "$target")
      if [[ "$first_line" == "---" ]]; then
        # Insert tags: after the opening ---
        tmp=$(mktemp)
        {
          echo "---"
          echo "$frontmatter"
          tail -n +2 "$target"
        } > "$tmp"
        mv "$tmp" "$target"
      else
        # Prepend as standalone block
        tmp=$(mktemp)
        {
          echo "---"
          echo "$frontmatter"
          echo "---"
          cat "$target"
        } > "$tmp"
        mv "$tmp" "$target"
      fi
      echo "OK    $current_file — added: $frontmatter"
    fi
    processed=$((processed + 1))
    current_file=""
  fi
done < "$MANIFEST"

echo ""
echo "Summary: ${processed} processed, ${skipped} skipped (already tagged), ${warned} warned (file not found)"
