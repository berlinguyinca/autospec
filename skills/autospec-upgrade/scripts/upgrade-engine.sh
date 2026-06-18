#!/usr/bin/env bash
# upgrade-engine.sh — per-hop upgrade loop for autospec-upgrade (issue #1180)
#
# Usage:
#   upgrade-engine.sh --hops <hops.json> [--root <dir>] [--max-fix <N>]
#
# Iterates the computed hops from compute-upgrade-steps.sh output:
#   {"hops":[{"framework":"angular","from":20,"to":21},...]}
#
# For each hop (in order):
#   1. IDEMPOTENCY: if post-upgrade-<fw>-<to> tag exists, SKIP.
#   2. Set pre-upgrade-<fw>-<to> tag.
#   3. Run codemod-route.sh <fw> <to>.
#   4. Run build (npm run build), type-check (tsc), tests (npm test),
#      then re-verify via behavior-lock.sh.
#   5. On failure: bounded fix-loop up to --max-fix (default 5) attempts.
#      If still failing: STOP, leave last green tag intact, exit non-zero.
#   6. On success: set post-upgrade-<fw>-<to> tag + commit.
#
# Exit codes:
#   0  — all hops completed (or all already complete)
#   1  — a hop's fix-loop exceeded the bound; last green tag preserved
#   2  — argument / file error

set -uo pipefail

# ── Argument parsing ──────────────────────────────────────────────────────────

HOPS_FILE=""
ROOT="."
MAX_FIX=5

while [ "$#" -gt 0 ]; do
  case "$1" in
    --hops)
      HOPS_FILE="${2:-}"
      shift 2
      ;;
    --hops=*)
      HOPS_FILE="${1#--hops=}"
      shift
      ;;
    --root)
      ROOT="${2:-}"
      shift 2
      ;;
    --root=*)
      ROOT="${1#--root=}"
      shift
      ;;
    --max-fix)
      MAX_FIX="${2:-5}"
      shift 2
      ;;
    --max-fix=*)
      MAX_FIX="${1#--max-fix=}"
      shift
      ;;
    *)
      printf 'upgrade-engine: unknown argument: %s\n' "$1" >&2
      exit 2
      ;;
  esac
done

# ── Validate arguments ────────────────────────────────────────────────────────

if [ -z "$HOPS_FILE" ]; then
  printf 'upgrade-engine: --hops <hops.json> is required\n' >&2
  exit 2
fi

if [ ! -f "$HOPS_FILE" ]; then
  printf 'upgrade-engine: hops file not found: %s\n' "$HOPS_FILE" >&2
  exit 2
fi

if [ ! -d "$ROOT" ]; then
  printf 'upgrade-engine: root directory not found: %s\n' "$ROOT" >&2
  exit 2
fi

ROOT="$(cd "$ROOT" && pwd)"

# ── Resolve sibling scripts ───────────────────────────────────────────────────
# AUTOSPEC_SCRIPTS_DIR env overrides the default (script's own directory).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"

CODEMOD_ROUTE="$SCRIPTS_DIR/codemod-route.sh"
BEHAVIOR_LOCK="$SCRIPTS_DIR/behavior-lock.sh"

# ── Helper: check if a git tag exists ────────────────────────────────────────

tag_exists() {
  local tag="$1"
  local result
  result="$(git tag -l "$tag" 2>/dev/null)"
  if [ -n "$result" ]; then
    return 0
  fi
  return 1
}

# ── Helper: set a git tag ─────────────────────────────────────────────────────

set_tag() {
  local tag="$1"
  git tag -a "$tag" -m "upgrade-engine: $tag" 2>/dev/null || \
    git tag "$tag" 2>/dev/null || true
}

# ── Helper: run the per-hop pipeline (codemod + build + test + behavior-lock) ─
# Returns 0 on success, non-zero on any failure.

run_hop_pipeline() {
  local fw="$1"
  local to="$2"

  # Step 1: codemod
  if [ -x "$CODEMOD_ROUTE" ]; then
    if ! "$CODEMOD_ROUTE" "$fw" "$to"; then
      printf 'upgrade-engine: codemod failed for %s -> %s\n' "$fw" "$to" >&2
      return 1
    fi
  elif command -v codemod-route.sh >/dev/null 2>&1; then
    if ! codemod-route.sh "$fw" "$to"; then
      printf 'upgrade-engine: codemod failed for %s -> %s\n' "$fw" "$to" >&2
      return 1
    fi
  else
    printf 'upgrade-engine: codemod-route.sh not found\n' >&2
    return 1
  fi

  # Step 2: build
  if ! npm run build 2>&1; then
    printf 'upgrade-engine: build failed for %s -> %s\n' "$fw" "$to" >&2
    return 1
  fi

  # Step 3: type-check (tsc -- optional, skip if not found)
  if command -v tsc >/dev/null 2>&1; then
    if ! tsc 2>&1; then
      printf 'upgrade-engine: type-check failed for %s -> %s\n' "$fw" "$to" >&2
      return 1
    fi
  fi

  # Step 4: tests
  if ! npm test 2>&1; then
    printf 'upgrade-engine: tests failed for %s -> %s\n' "$fw" "$to" >&2
    return 1
  fi

  # Step 5: behavior-lock re-verify
  if [ -x "$BEHAVIOR_LOCK" ]; then
    if ! "$BEHAVIOR_LOCK" --root "$ROOT" 2>&1; then
      printf 'upgrade-engine: behavior-lock re-verify failed for %s -> %s\n' "$fw" "$to" >&2
      return 1
    fi
  elif command -v behavior-lock.sh >/dev/null 2>&1; then
    if ! behavior-lock.sh --root "$ROOT" 2>&1; then
      printf 'upgrade-engine: behavior-lock re-verify failed for %s -> %s\n' "$fw" "$to" >&2
      return 1
    fi
  fi

  return 0
}

# ── Main: iterate hops ────────────────────────────────────────────────────────

hop_count="$(jq '.hops | length' "$HOPS_FILE" 2>/dev/null || printf '0')"

if [ "$hop_count" -eq 0 ]; then
  printf 'upgrade-engine: no hops to process\n'
  exit 0
fi

i=0
while [ "$i" -lt "$hop_count" ]; do
  fw="$(jq -r ".hops[$i].framework" "$HOPS_FILE")"
  from="$(jq -r ".hops[$i].from" "$HOPS_FILE")"
  to="$(jq -r ".hops[$i].to" "$HOPS_FILE")"

  pre_tag="pre-upgrade-${fw}-${to}"
  post_tag="post-upgrade-${fw}-${to}"

  printf 'upgrade-engine: hop %s/%s: %s %s -> %s\n' \
    "$((i + 1))" "$hop_count" "$fw" "$from" "$to"

  # IDEMPOTENCY: skip if post-upgrade tag already exists
  if tag_exists "$post_tag"; then
    printf 'upgrade-engine: hop already complete (tag %s exists), skipping\n' "$post_tag"
    i=$(( i + 1 ))
    continue
  fi

  # Set pre-upgrade tag (checkpoint before the hop)
  printf 'upgrade-engine: setting tag %s\n' "$pre_tag"
  set_tag "$pre_tag"

  # Bounded fix-loop
  attempt=1
  hop_ok=false
  while [ "$attempt" -le "$MAX_FIX" ]; do
    printf 'upgrade-engine: attempt %s/%s for %s -> %s\n' \
      "$attempt" "$MAX_FIX" "$fw" "$to"
    if run_hop_pipeline "$fw" "$to"; then
      hop_ok=true
      break
    fi
    attempt=$(( attempt + 1 ))
  done

  if [ "$hop_ok" = "false" ]; then
    printf 'upgrade-engine: hop %s -> %s FAILED after %s attempt(s)\n' \
      "$fw" "$to" "$MAX_FIX" >&2
    printf 'upgrade-engine: last green tag: %s\n' "$pre_tag" >&2
    printf 'upgrade-engine: diff at failure point:\n' >&2
    git diff 2>/dev/null >&2 || true
    exit 1
  fi

  # Success: set post-upgrade tag and commit
  printf 'upgrade-engine: hop succeeded — setting tag %s\n' "$post_tag"
  set_tag "$post_tag"
  git add -A 2>/dev/null || true
  git commit -m "upgrade: ${fw} ${from} -> ${to} [autospec-upgrade]" 2>/dev/null || true

  i=$(( i + 1 ))
done

printf 'upgrade-engine: all hops complete\n'
exit 0
