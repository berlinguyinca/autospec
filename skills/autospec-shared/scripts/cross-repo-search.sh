#!/usr/bin/env bash
# cross-repo-search.sh — Search memory files across all indexed repos.
#
# Wraps `mempalace search` across all indexed repos, falling back to grep -r.
# Tags each hit with the source repo slug.
#
# Usage:
#   cross-repo-search.sh "query" [--index-dir DIR] [--top-k N]
#
# Options:
#   --index-dir DIR   Override index dir (default: ~/.autospec/memory-index)
#   --top-k N         Limit results (default: 10)
#   --help            Show usage and exit 0
#
# Exit codes:
#   0 always (no matches is a silent no-op)
#
# Environment:
#   AUTOSPEC_SKIP_CROSS_REPO_INDEX=1  — skip search (opt-out, exits 0 empty)

set +e

# ── Parse args ────────────────────────────────────────────────────────────────
QUERY=""
INDEX_DIR=""
TOP_K=10

while [ $# -gt 0 ]; do
  case "$1" in
    --index-dir)
      INDEX_DIR="$2"
      shift 2
      ;;
    --top-k)
      TOP_K="$2"
      shift 2
      ;;
    --help|-h)
      printf 'Usage: cross-repo-search.sh "query" [--index-dir DIR] [--top-k N]\n'
      exit 0
      ;;
    -*)
      shift
      ;;
    *)
      # First positional arg is the query
      if [ -z "$QUERY" ]; then
        QUERY="$1"
      fi
      shift
      ;;
  esac
done

# ── Opt-out ───────────────────────────────────────────────────────────────────
[ "${AUTOSPEC_SKIP_CROSS_REPO_INDEX:-0}" = "1" ] && exit 0

# ── Guard: empty query ────────────────────────────────────────────────────────
if [ -z "$QUERY" ]; then
  exit 0
fi

# ── Resolve index dir ─────────────────────────────────────────────────────────
if [ -z "$INDEX_DIR" ]; then
  INDEX_DIR="${HOME}/.autospec/memory-index"
fi

# ── Guard: empty or missing index ─────────────────────────────────────────────
if [ ! -d "$INDEX_DIR" ]; then
  exit 0
fi

_found_any=0
_result_count=0

# Build grep pattern from query words
_pattern=$(printf '%s' "$QUERY" | tr ' ' '|')

# ── Search each indexed repo ──────────────────────────────────────────────────
for _link in "$INDEX_DIR"/*; do
  [ -L "$_link" ] || continue
  _target=$(readlink "$_link" 2>/dev/null)
  [ -d "$_target" ] || continue

  _slug=$(basename "$_link")

  # Try mempalace search first (repo-local, with correct cwd)
  _mp_results=""
  if command -v mempalace > /dev/null 2>&1; then
    _mp_results=$(cd "$_target" && mempalace search "$QUERY" 2>/dev/null \
      | grep -o '[^/]*\.md' \
      | grep -v "^MEMORY\.md$" \
      | head -"$TOP_K" || true)
  fi

  if [ -n "$_mp_results" ]; then
    while IFS= read -r _fname; do
      [ -z "$_fname" ] && continue
      _file="$_target/$_fname"
      [ -f "$_file" ] || continue
      [ "$_result_count" -ge "$TOP_K" ] && break
      _content=$(head -5 "$_file" 2>/dev/null | tr '\n' ' ' | cut -c1-200)
      printf '### Memory: %s [source: %s]\n%s\n\n' "$_fname" "$_slug" "$_content"
      _found_any=1
      _result_count=$((_result_count + 1))
    done <<< "$_mp_results"
  else
    # Grep fallback: score files by match count, sort descending
    _grep_hits=$(grep -ril -E "$_pattern" "$_target"/*.md 2>/dev/null \
      | grep -v "/MEMORY\.md$" \
      | while IFS= read -r _f; do
          _count=$(grep -ci -E "$_pattern" "$_f" 2>/dev/null || echo 0)
          printf '%s\t%s\n' "$_count" "$_f"
        done \
      | sort -rn \
      | head -"$TOP_K" \
      | awk '{print $2}')

    if [ -n "$_grep_hits" ]; then
      while IFS= read -r _file; do
        [ -z "$_file" ] && continue
        [ -f "$_file" ] || continue
        [ "$_result_count" -ge "$TOP_K" ] && break
        _fname=$(basename "$_file")
        _content=$(head -5 "$_file" 2>/dev/null | tr '\n' ' ' | cut -c1-200)
        printf '### Memory: %s [source: %s]\n%s\n\n' "$_fname" "$_slug" "$_content"
        _found_any=1
        _result_count=$((_result_count + 1))
      done <<< "$_grep_hits"
    fi
  fi

  [ "$_result_count" -ge "$TOP_K" ] && break
done

exit 0
