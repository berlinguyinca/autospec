#!/usr/bin/env bash
# upgrade-detect.sh — emit a single-line detection JSON for upgrade-detect (issue #1173)
#
# Usage: upgrade-detect.sh [--root <dir>]
#
# Output contract (single-line JSON to stdout):
#   {"frameworks":[...],"versions":{...},"package_manager":"npm|pnpm|yarn|bun",
#    "runners":[...],"monorepo":bool,"has_tests":bool}
#
# Exit 0 always (including unknown stack where frameworks:["unknown"]).

set -uo pipefail

# ── Argument parsing ──────────────────────────────────────────────────────────

ROOT="."
while [ $# -gt 0 ]; do
  case "$1" in
    --root)
      ROOT="$2"
      shift 2
      ;;
    --root=*)
      ROOT="${1#--root=}"
      shift
      ;;
    *)
      shift
      ;;
  esac
done

# Resolve to absolute path (bash 3.2 safe)
if [ ! -d "$ROOT" ]; then
  printf 'upgrade-detect: directory not found: %s\n' "$ROOT" >&2
  exit 1
fi
ROOT="$(cd "$ROOT" && pwd)"

PKG="$ROOT/package.json"
if [ ! -f "$PKG" ]; then
  printf 'upgrade-detect: no package.json found in %s\n' "$ROOT" >&2
  exit 1
fi

# ── Helper: strip leading ^~ from a version range ────────────────────────────

strip_range() {
  printf '%s\n' "$1" | sed 's/^[~^]*//'
}

# ── Detect package manager from lockfile ─────────────────────────────────────

detect_package_manager() {
  if [ -f "$ROOT/pnpm-lock.yaml" ]; then
    printf 'pnpm'
  elif [ -f "$ROOT/yarn.lock" ]; then
    printf 'yarn'
  elif [ -f "$ROOT/bun.lockb" ] || [ -f "$ROOT/bun.lock" ]; then
    printf 'bun'
  elif [ -f "$ROOT/package-lock.json" ]; then
    printf 'npm'
  else
    printf 'npm'
  fi
}

# ── Detect frameworks from package.json deps+devDeps ─────────────────────────
# Returns newline-separated list of: angular, next, react

detect_frameworks() {
  jq -r '
    ((.dependencies // {}) + (.devDependencies // {})) as $deps |
    [
      if $deps | has("@angular/core") then "angular" else empty end,
      if $deps | has("next") then "next" else empty end,
      if ($deps | has("react")) then "react" else empty end
    ] | .[]
  ' "$PKG"
}

# ── Resolve version for a given package from package.json ────────────────────

resolve_version() {
  local pkg_name="$1"
  local raw
  raw="$(jq -r --arg p "$pkg_name" '
    (.dependencies // {}) + (.devDependencies // {}) | .[$p] // ""
  ' "$PKG")"
  if [ -n "$raw" ] && [ "$raw" != "null" ]; then
    strip_range "$raw"
  else
    printf ''
  fi
}

# ── Detect test runners ───────────────────────────────────────────────────────

detect_runners() {
  local runners="" name dep globs g found deps
  # All declared dep/devDep names, one per line, for a fixed-string match.
  deps="$(jq -r '((.dependencies // {}) + (.devDependencies // {})) | keys[]' "$PKG" 2>/dev/null)"
  # Table: runner-name | dependency key | config globs (space-separated, under $ROOT).
  while IFS='|' read -r name dep globs; do
    [ -n "$name" ] || continue
    found=no
    if printf '%s\n' "$deps" | grep -Fqx "$dep"; then
      found=yes
    else
      for g in $globs; do
        if ls "$ROOT"/$g 2>/dev/null | grep -q .; then found=yes; break; fi
      done
    fi
    if [ "$found" = "yes" ]; then runners="${runners} ${name}"; fi
  done <<'EOF'
jest|jest|jest.config.*
vitest|vitest|vitest.config.*
karma|karma|karma.conf.*
playwright|@playwright/test|playwright.config.*
cypress|cypress|cypress.config.* cypress.json
EOF
  printf '%s' "$runners"
}

# ── Detect monorepo ───────────────────────────────────────────────────────────

detect_monorepo() {
  # workspaces key in package.json
  local has_ws
  has_ws="$(jq -r 'if has("workspaces") then "yes" else "no" end' "$PKG")"
  if [ "$has_ws" = "yes" ]; then
    printf 'true'
    return
  fi

  # nx.json or turbo.json / turbo.jsonc
  if [ -f "$ROOT/nx.json" ] || \
     [ -f "$ROOT/turbo.json" ] || \
     [ -f "$ROOT/turbo.jsonc" ]; then
    printf 'true'
    return
  fi

  printf 'false'
}

# ── Detect has_tests: scan for *.spec.* / *.test.* / __tests__/ ──────────────

detect_has_tests() {
  # find excluding node_modules; bash 3.2 safe (no process substitution with [ -f ])
  local tmpf
  tmpf="$(mktemp /tmp/upgrade-detect-tests.XXXXXX)"

  find "$ROOT" \
    -not -path "*/node_modules/*" \
    -not -path "*/.git/*" \
    \( \
      -name "*.spec.ts"  -o -name "*.spec.tsx" -o \
      -name "*.spec.js"  -o -name "*.spec.jsx" -o \
      -name "*.test.ts"  -o -name "*.test.tsx" -o \
      -name "*.test.js"  -o -name "*.test.jsx" -o \
      -type d -name "__tests__" \
    \) \
    -print 2>/dev/null > "$tmpf"

  local result="false"
  if [ -s "$tmpf" ]; then
    result="true"
  fi
  rm -f "$tmpf"
  printf '%s' "$result"
}

# ── Main ──────────────────────────────────────────────────────────────────────

PACKAGE_MANAGER="$(detect_package_manager)"

# Collect raw frameworks (newline-separated)
RAW_FRAMEWORKS="$(detect_frameworks)"

# Build frameworks array + versions object
FRAMEWORKS_JSON="[]"
VERSIONS_JSON="{}"

if [ -z "$RAW_FRAMEWORKS" ]; then
  # Unknown stack
  FRAMEWORKS_JSON='["unknown"]'
else
  # Process each detected framework
  while IFS= read -r fw; do
    [ -z "$fw" ] && continue
    FRAMEWORKS_JSON="$(printf '%s' "$FRAMEWORKS_JSON" | jq -c --arg f "$fw" '. + [$f]')"

    case "$fw" in
      angular)
        ver="$(resolve_version "@angular/core")"
        if [ -n "$ver" ]; then
          VERSIONS_JSON="$(printf '%s' "$VERSIONS_JSON" | jq -c --arg v "$ver" '.angular = $v')"
        fi
        ;;
      next)
        ver="$(resolve_version "next")"
        if [ -n "$ver" ]; then
          VERSIONS_JSON="$(printf '%s' "$VERSIONS_JSON" | jq -c --arg v "$ver" '.next = $v')"
        fi
        # next implies react
        FRAMEWORKS_JSON="$(printf '%s' "$FRAMEWORKS_JSON" | jq -c 'if index("react") == null then . + ["react"] else . end')"
        ver_react="$(resolve_version "react")"
        if [ -n "$ver_react" ]; then
          VERSIONS_JSON="$(printf '%s' "$VERSIONS_JSON" | jq -c --arg v "$ver_react" '.react = $v')"
        fi
        ;;
      react)
        ver="$(resolve_version "react")"
        if [ -n "$ver" ]; then
          VERSIONS_JSON="$(printf '%s' "$VERSIONS_JSON" | jq -c --arg v "$ver" '.react = $v')"
        fi
        ;;
    esac
  done << FWEOF
$RAW_FRAMEWORKS
FWEOF
fi

# Build runners array
RAW_RUNNERS="$(detect_runners)"
RUNNERS_JSON="[]"
for r in $RAW_RUNNERS; do
  [ -z "$r" ] && continue
  RUNNERS_JSON="$(printf '%s' "$RUNNERS_JSON" | jq -c --arg r "$r" '. + [$r]')"
done

MONOREPO="$(detect_monorepo)"
HAS_TESTS="$(detect_has_tests)"

# Emit single-line JSON
jq -cn \
  --argjson frameworks "$FRAMEWORKS_JSON" \
  --argjson versions "$VERSIONS_JSON" \
  --arg package_manager "$PACKAGE_MANAGER" \
  --argjson runners "$RUNNERS_JSON" \
  --argjson monorepo "$MONOREPO" \
  --argjson has_tests "$HAS_TESTS" \
  '{
    frameworks: $frameworks,
    versions: $versions,
    package_manager: $package_manager,
    runners: $runners,
    monorepo: $monorepo,
    has_tests: $has_tests
  }'
