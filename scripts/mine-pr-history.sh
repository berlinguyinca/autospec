#!/usr/bin/env bash
# mine-pr-history.sh — Extract memory-worthy lessons from merged PR descriptions.
#
# Scans merged PRs for body:lesson: patterns, creates docs/memory/lesson_<slug>.md
# per match. Idempotent: skips PRs already mined (deduped by PR number in frontmatter).
#
# Usage:
#   mine-pr-history.sh --repo <owner/repo> --output-dir <dir>
#   mine-pr-history.sh --repo <owner/repo> --output-dir <dir> --quiet
#
# Exit codes:
#   0 always (gh absence / failures are non-blocking)
#
# Environment:
#   AUTOSPEC_MINE_PR_HISTORY  — must be set to "1" to enable mining (off by default)
#   AUTOSPEC_MINE_MIN_BODY    — minimum PR body length to consider (default: 200)
#   AUTOSPEC_MINE_LIMIT       — max PRs to scan per run (default: 200)
#
# Requires: bash 3.2+
# Optional: gh CLI (silent skip when absent)

set +e

REPO=""
OUTPUT_DIR=""
QUIET=0

# ── Argument parse ────────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
  case "$1" in
    --repo)
      REPO="$2"
      shift 2
      ;;
    --output-dir)
      OUTPUT_DIR="$2"
      shift 2
      ;;
    --quiet)
      QUIET=1
      shift
      ;;
    --help|-h)
      printf 'Usage: mine-pr-history.sh --repo <owner/repo> --output-dir <dir> [--quiet]\n'
      exit 0
      ;;
    *)
      [ "$QUIET" -eq 0 ] && echo "mine-pr-history: unknown argument: $1" >&2
      shift
      ;;
  esac
done

# ── Guard: opt-in flag required ──────────────────────────────────────────────
if [ "${AUTOSPEC_MINE_PR_HISTORY:-0}" != "1" ]; then
  exit 0
fi

# ── Guard: required args ─────────────────────────────────────────────────────
if [ -z "$REPO" ]; then
  [ "$QUIET" -eq 0 ] && echo "mine-pr-history: --repo is required" >&2
  exit 0
fi

if [ -z "$OUTPUT_DIR" ]; then
  [ "$QUIET" -eq 0 ] && echo "mine-pr-history: --output-dir is required" >&2
  exit 0
fi

# ── Guard: gh CLI absent ──────────────────────────────────────────────────────
if ! command -v gh > /dev/null 2>&1; then
  [ "$QUIET" -eq 0 ] && echo "mine-pr-history: gh CLI not found; skipping" >&2
  exit 0
fi

mkdir -p "$OUTPUT_DIR"

MIN_BODY="${AUTOSPEC_MINE_MIN_BODY:-200}"
LIMIT="${AUTOSPEC_MINE_LIMIT:-200}"

# ── Build set of already-mined PR numbers (dedup by frontmatter source_pr_number:) ──
_already_mined() {
  local pr_num="$1"
  grep -rl "source_pr_number: ${pr_num}$" "$OUTPUT_DIR" 2>/dev/null | grep -q .
}

# ── Slugify a string for use in filenames ────────────────────────────────────
_slugify() {
  printf '%s' "$1" \
    | tr '[:upper:]' '[:lower:]' \
    | sed 's/[^a-z0-9]/-/g' \
    | sed 's/-\{2,\}/-/g' \
    | sed 's/^-//;s/-$//' \
    | cut -c1-50
}

# ── Fetch merged PRs with lesson: in body or label:postmortem ────────────────
[ "$QUIET" -eq 0 ] && echo "mine-pr-history: scanning $REPO for lesson PRs (limit=$LIMIT)"

created=0
skipped=0

# Fetch PRs as JSONL — one JSON object per line
pr_json=$(gh pr list \
  --repo "$REPO" \
  --state merged \
  --limit "$LIMIT" \
  --json number,title,url,body \
  2>/dev/null) || pr_json=""

if [ -z "$pr_json" ]; then
  [ "$QUIET" -eq 0 ] && echo "mine-pr-history: no PRs returned from gh" >&2
  exit 0
fi

# Process each PR object (one per line from gh's JSON array)
# Use python3 or jq to iterate; fall back to awk-based JSONL parse
if command -v jq > /dev/null 2>&1; then
  pr_list=$(printf '%s' "$pr_json" | jq -c '.[]' 2>/dev/null) || pr_list=""
else
  pr_list=""
fi

if [ -z "$pr_list" ]; then
  [ "$QUIET" -eq 0 ] && echo "mine-pr-history: jq not available or no results; skipping" >&2
  exit 0
fi

while IFS= read -r pr_obj; do
  [ -z "$pr_obj" ] && continue

  pr_num=$(printf '%s' "$pr_obj" | jq -r '.number' 2>/dev/null)
  pr_title=$(printf '%s' "$pr_obj" | jq -r '.title' 2>/dev/null)
  pr_url=$(printf '%s' "$pr_obj" | jq -r '.url' 2>/dev/null)
  pr_body=$(printf '%s' "$pr_obj" | jq -r '.body' 2>/dev/null)

  [ -z "$pr_num" ] || [ "$pr_num" = "null" ] && continue

  # Filter: body too short
  body_len=${#pr_body}
  if [ "$body_len" -lt "$MIN_BODY" ]; then
    [ "$QUIET" -eq 0 ] && echo "mine-pr-history: skip #$pr_num (body too short: $body_len < $MIN_BODY)"
    skipped=$(( skipped + 1 ))
    continue
  fi

  # Filter: must contain "lesson:" marker in body
  if ! printf '%s' "$pr_body" | grep -qi "lesson:"; then
    [ "$QUIET" -eq 0 ] && echo "mine-pr-history: skip #$pr_num (no lesson: marker)"
    skipped=$(( skipped + 1 ))
    continue
  fi

  # Idempotency: skip already-mined PRs
  if _already_mined "$pr_num"; then
    [ "$QUIET" -eq 0 ] && echo "mine-pr-history: skip #$pr_num (already mined)"
    skipped=$(( skipped + 1 ))
    continue
  fi

  # Extract lesson lines from body
  lesson_text=$(printf '%s' "$pr_body" | grep -i "lesson:" | head -5 | \
    sed 's/^[Ll]esson:[[:space:]]*//' | head -3)
  [ -z "$lesson_text" ] && lesson_text="$pr_title"

  # Build filename slug from PR number + title
  title_slug=$(_slugify "$pr_title")
  out_file="$OUTPUT_DIR/lesson_pr${pr_num}_${title_slug}.md"

  [ "$QUIET" -eq 0 ] && echo "mine-pr-history: creating $out_file"

  # Write lesson file with frontmatter
  cat > "$out_file" <<LESSON_EOF
---
type: feedback
source_pr: $pr_url
source_pr_number: $pr_num
mined_at: $(date -u +%Y-%m-%dT%H:%M:%SZ)
---

# Lesson from PR #$pr_num: $pr_title

$lesson_text
LESSON_EOF

  created=$(( created + 1 ))

done <<< "$pr_list"

[ "$QUIET" -eq 0 ] && echo "mine-pr-history: done — created=$created skipped=$skipped"
exit 0
