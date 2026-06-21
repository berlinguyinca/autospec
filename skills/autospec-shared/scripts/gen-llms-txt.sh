#!/usr/bin/env bash
# gen-llms-txt.sh — generate llms.txt (≤200 lines) and llms-full.txt at repo root.
#
# Usage:
#   bash gen-llms-txt.sh --repo-root <dir> [--cluster-json <file>]
#
# llms.txt   — short curated index (≤200 lines): repo summary, key doc paths, entry points
# llms-full.txt — concatenated full content of USER_MANUAL + API_REFERENCE + ARCHITECTURE + key spec excerpts
#
# Both files are idempotent: skipped when byte-equal to existing.
#
# Exit codes: 0=success, 1=error

set -euo pipefail

REPO_ROOT=""
CLUSTER_JSON=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)   REPO_ROOT="${2:?'--repo-root requires a value'}"; shift 2 ;;
    --cluster-json) CLUSTER_JSON="${2:?'--cluster-json requires a value'}"; shift 2 ;;
    -h|--help)
      echo "Usage: bash gen-llms-txt.sh --repo-root <dir> [--cluster-json <file>]"
      exit 0 ;;
    *) echo "gen-llms-txt.sh: unknown option '$1'" >&2; exit 1 ;;
  esac
done

if [[ -z "$REPO_ROOT" ]]; then
  echo "gen-llms-txt.sh: --repo-root is required" >&2
  exit 1
fi

REPO_ROOT="$(cd "$REPO_ROOT" && pwd)"
DOCS_DIR="$REPO_ROOT/docs"
LLMS_TXT="$REPO_ROOT/llms.txt"
LLMS_FULL_TXT="$REPO_ROOT/llms-full.txt"

# ── Detect repo slug ──────────────────────────────────────────────────────────
# Portable (BSD + GNU): strip a trailing .git and take the repo basename via
# shell parameter expansion. The old `sed -E` used a lazy `+?` quantifier, which
# is a PCRE feature BSD sed (macOS) rejects ("RE error") — it then fell through to
# basename of the worktree dir, corrupting the slug (e.g. a git-worktree run
# produced "# wt-…" instead of "# autospec").
REPO_SLUG="$(git -C "$REPO_ROOT" remote get-url origin 2>/dev/null)"
REPO_SLUG="${REPO_SLUG%.git}"     # drop trailing .git if present
REPO_SLUG="${REPO_SLUG##*/}"      # repo basename (handles https and scp-style URLs)
[ -n "$REPO_SLUG" ] || REPO_SLUG="$(basename "$REPO_ROOT")"

# ── Read README for summary ───────────────────────────────────────────────────
README_SUMMARY=""
for readme in "$REPO_ROOT/README.md" "$REPO_ROOT/README.txt" "$REPO_ROOT/README"; do
  if [[ -f "$readme" ]]; then
    README_SUMMARY="$(head -20 "$readme" 2>/dev/null | grep -v '^#\s*$' | head -5 || true)"
    break
  fi
done

# ── Collect CLI entry points from cluster JSON (if provided) ──────────────────
CLI_ENTRIES=""
if [[ -n "$CLUSTER_JSON" ]] && [[ -f "$CLUSTER_JSON" ]]; then
  CLI_ENTRIES="$(node -e "
    const c = JSON.parse(require('fs').readFileSync('$CLUSTER_JSON','utf8'));
    const entries = [];
    for (const u of (c.significant||[])) {
      for (const ep of (u.entry_points||[])) {
        if (ep.kind === 'cli_command') entries.push(ep.identifier);
      }
    }
    console.log(entries.join('\n'));
  " 2>/dev/null || true)"
fi

# ── Write llms.txt ────────────────────────────────────────────────────────────
build_llms_txt() {
  local lines=()
  lines+=("# ${REPO_SLUG}")
  lines+=("")
  if [[ -n "$README_SUMMARY" ]]; then
    lines+=("## Summary")
    lines+=("")
    while IFS= read -r line; do
      lines+=("$line")
    done <<< "$README_SUMMARY"
    lines+=("")
  fi

  lines+=("## Key Documentation")
  lines+=("")
  for doc in "docs/USER_MANUAL.md" "docs/API_REFERENCE.md" "docs/ARCHITECTURE.md"; do
    if [[ -f "$REPO_ROOT/$doc" ]]; then
      lines+=("- ${doc}")
    fi
  done
  lines+=("")

  lines+=("## Design Specifications")
  lines+=("")
  if [[ -d "$DOCS_DIR/specs" ]]; then
    local spec_count=0
    while IFS= read -r spec; do
      if [[ $spec_count -lt 10 ]]; then
        lines+=("- ${spec#$REPO_ROOT/}")
        ((spec_count++)) || true
      fi
    done < <(find "$DOCS_DIR/specs" -name "*.md" -maxdepth 1 2>/dev/null | sort | head -10)
    if [[ $spec_count -eq 0 ]]; then
      lines+=("- docs/specs/ (no spec files found)")
    fi
  fi
  lines+=("")

  if [[ -n "$CLI_ENTRIES" ]]; then
    lines+=("## CLI Entry Points")
    lines+=("")
    while IFS= read -r entry; do
      [[ -z "$entry" ]] && continue
      lines+=("- \`${entry}\`")
    done <<< "$CLI_ENTRIES"
    lines+=("")
  fi

  lines+=("## LLM Artifacts")
  lines+=("")
  lines+=("- llms.txt — this file (short index)")
  lines+=("- llms-full.txt — full docs for context-window ingestion")
  lines+=("- docs/.llm-manifest.json — per-symbol structured manifest")
  lines+=("- docs/ASSISTANT_PROMPT.md — paste-ready system prompt")
  lines+=("")

  # Enforce ≤200 lines
  local total=${#lines[@]}
  if [[ $total -gt 200 ]]; then
    lines=("${lines[@]:0:199}")
    lines+=("... (truncated to 200 lines)")
  fi

  printf '%s\n' "${lines[@]}"
}

NEW_LLMS_TXT="$(build_llms_txt)"
LINE_COUNT="$(echo "$NEW_LLMS_TXT" | wc -l | tr -d ' ')"

if [[ -f "$LLMS_TXT" ]] && [[ "$(cat "$LLMS_TXT")" = "$NEW_LLMS_TXT" ]]; then
  echo "[gen-llms-txt] llms.txt unchanged (${LINE_COUNT} lines)" >&2
else
  echo "$NEW_LLMS_TXT" > "$LLMS_TXT"
  echo "[gen-llms-txt] llms.txt written (${LINE_COUNT} lines)" >&2
fi

# ── Write llms-full.txt ───────────────────────────────────────────────────────
build_llms_full() {
  echo "# Full Documentation for ${REPO_SLUG}"
  echo ""
  echo "> Generated from: USER_MANUAL.md + API_REFERENCE.md + ARCHITECTURE.md + key spec excerpts"
  echo ""

  for doc in "docs/USER_MANUAL.md" "docs/API_REFERENCE.md" "docs/ARCHITECTURE.md"; do
    local doc_path="$REPO_ROOT/$doc"
    if [[ -f "$doc_path" ]]; then
      echo "---"
      echo ""
      echo "## File: ${doc}"
      echo ""
      cat "$doc_path"
      echo ""
    fi
  done

  # Append up to 3 most-recent spec files
  if [[ -d "$DOCS_DIR/specs" ]]; then
    local spec_count=0
    while IFS= read -r spec; do
      if [[ $spec_count -lt 3 ]]; then
        echo "---"
        echo ""
        echo "## Spec: ${spec#$REPO_ROOT/}"
        echo ""
        # First 100 lines of each spec
        head -100 "$spec" 2>/dev/null || true
        echo ""
        ((spec_count++)) || true
      fi
    done < <(find "$DOCS_DIR/specs" -name "*.md" -maxdepth 1 2>/dev/null | sort -r | head -3)
  fi
}

NEW_LLMS_FULL="$(build_llms_full)"

if [[ -f "$LLMS_FULL_TXT" ]] && [[ "$(cat "$LLMS_FULL_TXT")" = "$NEW_LLMS_FULL" ]]; then
  echo "[gen-llms-txt] llms-full.txt unchanged" >&2
else
  echo "$NEW_LLMS_FULL" > "$LLMS_FULL_TXT"
  FULL_LINES="$(echo "$NEW_LLMS_FULL" | wc -l | tr -d ' ')"
  echo "[gen-llms-txt] llms-full.txt written (${FULL_LINES} lines)" >&2
fi

echo "[gen-llms-txt] done" >&2
