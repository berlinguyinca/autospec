#!/usr/bin/env bash
# publish-helpers.sh — Build and publish the @autospec/test npm package.
#
# Usage:
#   ./scripts/publish-helpers.sh --dry-run   # build + npm publish --dry-run (safe, no real publish)
#   ./scripts/publish-helpers.sh --release   # build + real npm publish (requires npm auth)
#
# Guards:
#   - Without --release, never calls bare 'npm publish'
#   - Verifies the version in package.json is bumped vs the latest npm registry tag
#   - Requires a clean git working tree when --release is used
#
# Exit codes:
#   0 = success
#   1 = build failure or guard violation

set -eu

SKILL_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$SKILL_DIR"

DRY_RUN=1
RELEASE=0

for arg in "$@"; do
  case "$arg" in
    --dry-run)  DRY_RUN=1; RELEASE=0 ;;
    --release)  RELEASE=1; DRY_RUN=0 ;;
    *)
      printf 'publish-helpers.sh: unknown flag: %s\n' "$arg" >&2
      printf 'Usage: publish-helpers.sh [--dry-run | --release]\n' >&2
      exit 1
      ;;
  esac
done

# ── Step 1: Build ─────────────────────────────────────────────────────────────
printf '[publish-helpers] building TypeScript...\n'
if ! npx tsc -p tsconfig.json; then
  printf '[publish-helpers] ERROR: tsc build failed\n' >&2
  exit 1
fi
printf '[publish-helpers] build OK → dist/\n'

# ── Step 2: Version guard (--release only) ────────────────────────────────────
if [ "$RELEASE" -eq 1 ]; then
  LOCAL_VERSION=$(node -p "require('./package.json').version" 2>/dev/null || echo "")
  REGISTRY_VERSION=$(npm view @autospec/test version 2>/dev/null || echo "0.0.0")

  if [ "$LOCAL_VERSION" = "$REGISTRY_VERSION" ]; then
    printf '[publish-helpers] ERROR: version %s already published. Bump package.json version before releasing.\n' "$LOCAL_VERSION" >&2
    exit 1
  fi
  printf '[publish-helpers] version check OK: local=%s registry=%s\n' "$LOCAL_VERSION" "$REGISTRY_VERSION"

  # Require clean git working tree
  if ! git -C "$SKILL_DIR" diff --quiet HEAD 2>/dev/null; then
    printf '[publish-helpers] ERROR: working tree is dirty. Commit all changes before --release.\n' >&2
    exit 1
  fi
fi

# ── Step 3: Pack verification ─────────────────────────────────────────────────
printf '[publish-helpers] running npm pack --dry-run to verify tarball...\n'
npm pack --dry-run 2>&1 | grep -E '(npm notice|Tarball|files|unpacked)' || true

# ── Step 4: Publish ───────────────────────────────────────────────────────────
if [ "$RELEASE" -eq 1 ]; then
  printf '[publish-helpers] publishing @autospec/test to npm registry...\n'
  # --release gate: only reached when RELEASE=1 (passed via --release flag)
  npm publish --access public
  printf '[publish-helpers] published OK\n'
else
  printf '[publish-helpers] dry-run: running npm publish --dry-run\n'
  npm publish --dry-run --access public 2>&1
  printf '[publish-helpers] dry-run complete (nothing was published)\n'
fi
