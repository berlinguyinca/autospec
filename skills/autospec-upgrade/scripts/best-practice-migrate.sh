#!/usr/bin/env bash
# best-practice-migrate.sh — Phase 3 best-practice migration for autospec-upgrade
#
# Usage:
#   best-practice-migrate.sh --detect <detect.json> [--root <dir>] [--out <dir>]
#                            [--versions-green]
#
# --detect <file>    Path to detection JSON from upgrade-detect.sh. Required.
# --root <dir>       Project root (default: .).
# --out <dir>        Output directory for .autospec/ artifacts (default: <root>/.autospec).
# --versions-green   Skip tag-based green check; caller asserts version loop passed.
#
# Exit codes:
#   0  — all best-practice schematics applied and behavior-lock re-verified.
#   1  — version loop NOT green; refused to run any schematic.
#   2  — a gated step failed (schematic or behavior-lock re-verify).
#   3  — argument error.
#
# Design invariants:
#   1. GREEN-GATE: run ONLY after the version loop is green (post-upgrade tags
#      must exist, or --versions-green asserted). Refuses (exit 1) otherwise.
#   2. Each schematic step shells codemod-route.sh — never hand-rolls migrations.
#   3. Each step is gated: behavior-lock.sh re-verify MUST pass after the schematic.
#   4. Manual structural migrations are SURFACED as follow-up lines, never attempted.

set -uo pipefail

# ── Script location ───────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Argument parsing ──────────────────────────────────────────────────────────

DETECT_FILE=""
ROOT="."
OUT_DIR=""
VERSIONS_GREEN=0

while [ $# -gt 0 ]; do
  case "$1" in
    --detect)
      DETECT_FILE="$2"
      shift 2
      ;;
    --detect=*)
      DETECT_FILE="${1#--detect=}"
      shift
      ;;
    --root)
      ROOT="$2"
      shift 2
      ;;
    --root=*)
      ROOT="${1#--root=}"
      shift
      ;;
    --out)
      OUT_DIR="$2"
      shift 2
      ;;
    --out=*)
      OUT_DIR="${1#--out=}"
      shift
      ;;
    --versions-green)
      VERSIONS_GREEN=1
      shift
      ;;
    *)
      shift
      ;;
  esac
done

# ── Validate arguments ────────────────────────────────────────────────────────

if [ -z "$DETECT_FILE" ]; then
  printf 'best-practice-migrate: --detect <file> is required\n' >&2
  exit 3
fi

if [ ! -f "$DETECT_FILE" ]; then
  printf 'best-practice-migrate: detect file not found: %s\n' "$DETECT_FILE" >&2
  exit 3
fi

if [ ! -d "$ROOT" ]; then
  printf 'best-practice-migrate: root directory not found: %s\n' "$ROOT" >&2
  exit 3
fi

ROOT="$(cd "$ROOT" && pwd)"

if [ -z "$OUT_DIR" ]; then
  OUT_DIR="$ROOT/.autospec"
fi

mkdir -p "$OUT_DIR"

# ── Read detection JSON ───────────────────────────────────────────────────────

FRAMEWORKS_JSON="$(jq -c '.frameworks' "$DETECT_FILE" 2>/dev/null || printf '[]')"
VERSIONS_JSON="$(jq -c '.versions' "$DETECT_FILE" 2>/dev/null || printf '{}')"

# Extract first framework (primary) and its version
PRIMARY_FW="$(jq -r '.frameworks[0] // "unknown"' "$DETECT_FILE" 2>/dev/null || printf 'unknown')"
PRIMARY_VER="$(jq -r --arg fw "$PRIMARY_FW" '.versions[$fw] // "unknown"' "$DETECT_FILE" 2>/dev/null || printf 'unknown')"

# Extract major version number only
PRIMARY_MAJOR="$(printf '%s\n' "$PRIMARY_VER" | sed 's/\..*//')"

# ── Resolve helper paths ──────────────────────────────────────────────────────
# Resolution order: PATH-visible mock/install first (lets tests inject mocks via
# $MOCK_BIN), then fall back to the sibling scripts/ directory.
# This means a test that puts a mock codemod-route.sh in $MOCK_BIN and prepends
# it to PATH will always win over the real sibling script.

_resolve_helper() {
  local name="$1"
  # Check PATH first (mock shim wins)
  if command -v "$name" >/dev/null 2>&1; then
    printf '%s' "$name"
    return 0
  fi
  # Fall back to sibling scripts/ dir
  if [ -x "$SCRIPT_DIR/$name" ]; then
    printf '%s' "$SCRIPT_DIR/$name"
    return 0
  fi
  return 1
}

CODEMOD_ROUTE="$(_resolve_helper codemod-route.sh || true)"
BEHAVIOR_LOCK="$(_resolve_helper behavior-lock.sh || true)"

# ── GREEN-GATE ────────────────────────────────────────────────────────────────
# Refuse to run any schematic unless the version loop is proven green.
# Green is proven by:
#   (a) --versions-green flag explicitly set by the caller, OR
#   (b) at least one post-upgrade-<fw>-<ver> tag exists in the repo.

is_green() {
  if [ "$VERSIONS_GREEN" -eq 1 ]; then
    return 0
  fi

  # Query git for post-upgrade tags matching this framework
  local tag_pattern="post-upgrade-${PRIMARY_FW}-*"
  local found
  found="$(git tag -l "$tag_pattern" 2>/dev/null | head -1)"

  if [ -n "$found" ]; then
    return 0
  fi

  return 1
}

if ! is_green; then
  printf 'best-practice-migrate: versions_not_green — version loop must complete successfully before best-practice migration.\n' >&2
  printf 'best-practice-migrate: no post-upgrade-%s-* tags found; run the upgrade engine first or pass --versions-green.\n' "$PRIMARY_FW" >&2
  exit 1
fi

# ── Follow-up accumulator ─────────────────────────────────────────────────────
# Manual structural migrations that no official schematic performs are collected
# here and printed at the end as operator follow-ups.

FOLLOWUP_FILE="$OUT_DIR/best-practice-followups.txt"
: > "$FOLLOWUP_FILE"

add_followup() {
  local msg="$1"
  printf 'follow-up: %s\n' "$msg" >> "$FOLLOWUP_FILE"
  printf 'follow-up: %s\n' "$msg"
}

# ── Gated step runner ─────────────────────────────────────────────────────────
# run_gated_step <description> <codemod-route args...>
#
# Shells codemod-route.sh with the given args, then re-verifies behavior-lock.
# Returns non-zero if either the schematic or behavior-lock re-verify fails.

run_gated_step() {
  local description="$1"
  shift

  printf 'best-practice-migrate: applying "%s"\n' "$description"

  if [ -z "$CODEMOD_ROUTE" ]; then
    printf 'best-practice-migrate: codemod-route.sh not found; cannot apply schematic\n' >&2
    return 2
  fi

  if ! "$CODEMOD_ROUTE" "$@"; then
    printf 'best-practice-migrate: schematic failed for "%s"\n' "$description" >&2
    return 2
  fi

  printf 'best-practice-migrate: re-verifying behavior-lock after "%s"\n' "$description"

  if [ -z "$BEHAVIOR_LOCK" ]; then
    printf 'best-practice-migrate: behavior-lock.sh not found; cannot re-verify\n' >&2
    return 2
  fi

  if ! "$BEHAVIOR_LOCK" --detect "$DETECT_FILE" --root "$ROOT" --out "$OUT_DIR"; then
    printf 'best-practice-migrate: behavior-lock re-verify FAILED after "%s" — step rejected\n' "$description" >&2
    return 2
  fi

  printf 'best-practice-migrate: "%s" accepted (behavior-lock green)\n' "$description"
  return 0
}

# ── Per-framework best-practice phases ───────────────────────────────────────

run_angular_best_practices() {
  # Official schematic: standalone component migration
  if ! run_gated_step "angular standalone migration" angular "$PRIMARY_MAJOR" --standalone; then
    return 2
  fi

  # Official schematic: signals adoption is partially covered by standalone;
  # full signals migration requires manual structural changes — surface as follow-up.
  add_followup "angular: adopt Signals API manually (no official schematic covers full signals adoption yet; review https://angular.dev/guide/signals)"

  # Manual: lazy-loading restructure, inject() adoption, standalone bootstrap
  add_followup "angular: migrate constructor DI to inject() function where appropriate (manual; review component by component)"
  add_followup "angular: ensure bootstrapApplication() is used instead of platformBrowserDynamic() for standalone apps"

  return 0
}

run_next_best_practices() {
  # Official schematic: async request APIs (cookies, headers, params become async)
  if ! run_gated_step "next async request API migration" next "$PRIMARY_MAJOR" --async-request-api; then
    return 2
  fi

  # Manual: Pages Router → App Router is a structural restructure — no schematic covers it
  add_followup "next: Pages Router → App Router migration is a structural restructure; no official schematic covers it — plan manually (https://nextjs.org/docs/app/building-your-application/upgrading/app-router-migration)"

  # Manual: metadata API changes (Head component → generateMetadata)
  add_followup "next: migrate <Head> component usage to generateMetadata() API in App Router layout files"

  return 0
}

run_react_best_practices() {
  # Official schematic: React 19 types codemod
  if ! run_gated_step "react 19 types migration" react "$PRIMARY_MAJOR" --types; then
    return 2
  fi

  # Manual: ref as prop (React 19 removes forwardRef), use() hook adoption
  add_followup "react: replace forwardRef() with ref-as-prop pattern (React 19 — manual; no schematic covers all cases)"
  add_followup "react: evaluate use() hook for async resource reading as a React 19 pattern (manual adoption)"

  return 0
}

# ── Dispatch to framework handler ─────────────────────────────────────────────

EXIT_CODE=0

case "$PRIMARY_FW" in
  angular)
    if ! run_angular_best_practices; then
      EXIT_CODE=2
    fi
    ;;
  next)
    if ! run_next_best_practices; then
      EXIT_CODE=2
    fi
    ;;
  react)
    if ! run_react_best_practices; then
      EXIT_CODE=2
    fi
    ;;
  *)
    printf 'best-practice-migrate: unknown framework "%s" — code_health:upgrade_unknown_framework\n' \
      "$PRIMARY_FW" >&2
    EXIT_CODE=3
    ;;
esac

# ── Print follow-up summary ───────────────────────────────────────────────────

if [ -s "$FOLLOWUP_FILE" ]; then
  printf '\nbest-practice-migrate: operator follow-ups (manual structural migrations — NOT attempted):\n'
  cat "$FOLLOWUP_FILE"
fi

printf 'best-practice-migrate: follow-ups written to %s\n' "$FOLLOWUP_FILE"

exit "$EXIT_CODE"
