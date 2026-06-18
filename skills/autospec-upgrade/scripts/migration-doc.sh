#!/usr/bin/env bash
# migration-doc.sh — render the human migration log from upgrade-report.json
#
# Usage:
#   migration-doc.sh [--report <file>] [--root <dir>] [--out <dir>]
#
# Options:
#   --report <file>   Path to upgrade-report.json.
#                     Default: <root>/.autospec/upgrade-report.json
#   --root <dir>      Project root used to locate the default report path.
#                     Default: current working directory.
#   --out <dir>       Directory where docs/migrations/<slug>-upgrade.md is written.
#                     Default: <root>
#
# Behaviour:
#   1. Read upgrade-report.json (array of per-hop entries).
#   2. Compose autospec-doc --full (via PATH-resolved binary) to finalize the log.
#   3. Emit one Markdown section per hop:
#        ## <framework> <from> → <to>
#        with: before/after versions, codemods applied, manual fixes, residual risk.
#   4. Write the log to <out>/docs/migrations/<date>-upgrade.md.
#   5. Print the doc path to stdout.
#
# Exit codes:
#   0 — success
#   1 — report file missing or not valid JSON
#   2 — argument / environment error

set -uo pipefail

# ── Helpers ───────────────────────────────────────────────────────────────────

die() {
  printf 'migration-doc: %s\n' "$*" >&2
  exit 2
}

die_report() {
  printf 'migration-doc: %s\n' "$*" >&2
  exit 1
}

# ── Argument parsing ──────────────────────────────────────────────────────────

REPORT_FILE=""
ROOT_DIR=""
OUT_DIR=""

while [ $# -gt 0 ]; do
  case "$1" in
    --report)
      [ $# -ge 2 ] || die "--report requires a value"
      REPORT_FILE="$2"
      shift 2
      ;;
    --root)
      [ $# -ge 2 ] || die "--root requires a value"
      ROOT_DIR="$2"
      shift 2
      ;;
    --out)
      [ $# -ge 2 ] || die "--out requires a value"
      OUT_DIR="$2"
      shift 2
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

# ── Defaults ──────────────────────────────────────────────────────────────────

if [ -z "$ROOT_DIR" ]; then
  ROOT_DIR="$(pwd)"
fi

if [ -z "$REPORT_FILE" ]; then
  REPORT_FILE="$ROOT_DIR/.autospec/upgrade-report.json"
fi

if [ -z "$OUT_DIR" ]; then
  OUT_DIR="$ROOT_DIR"
fi

# ── Validate report file ──────────────────────────────────────────────────────

if [ ! -f "$REPORT_FILE" ]; then
  die_report "report file not found: $REPORT_FILE"
fi

if ! jq -e 'type == "array"' "$REPORT_FILE" > /dev/null 2>&1; then
  die_report "report file is not a valid JSON array: $REPORT_FILE"
fi

# ── Derive output path ────────────────────────────────────────────────────────

DATE_SLUG="$(date -u '+%Y-%m-%d' 2>/dev/null || printf 'unknown-date')"
MIGRATIONS_DIR="$OUT_DIR/docs/migrations"
DOC_FILE="$MIGRATIONS_DIR/${DATE_SLUG}-upgrade.md"

mkdir -p "$MIGRATIONS_DIR"

# ── Render Markdown ───────────────────────────────────────────────────────────

{
  printf '# Upgrade Migration Log\n\n'
  printf 'Generated: %s\n\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || printf 'unknown')"

  hop_count="$(jq 'length' "$REPORT_FILE")"
  i=0
  while [ "$i" -lt "$hop_count" ]; do
    framework="$(jq -r ".[$i].framework" "$REPORT_FILE")"
    from_ver="$(jq -r ".[$i].from" "$REPORT_FILE")"
    to_ver="$(jq -r ".[$i].to" "$REPORT_FILE")"
    residual_risk="$(jq -r ".[$i].residual_risk" "$REPORT_FILE")"

    printf '## %s %s → %s\n\n' "$framework" "$from_ver" "$to_ver"

    printf '### Versions\n\n'
    printf '%s\n' "- **Before:** $framework $from_ver"
    printf '%s\n\n' "- **After:** $framework $to_ver"

    printf '### Codemods Applied\n\n'
    codemod_count="$(jq ".[$i].codemods | length" "$REPORT_FILE")"
    if [ "$codemod_count" -eq 0 ]; then
      printf '_None_\n\n'
    else
      j=0
      while [ "$j" -lt "$codemod_count" ]; do
        codemod="$(jq -r ".[$i].codemods[$j]" "$REPORT_FILE")"
        printf '%s\n' "- $codemod"
        j=$((j + 1))
      done
      printf '\n'
    fi

    printf '### Manual Fixes\n\n'
    fix_count="$(jq ".[$i].manual_fixes | length" "$REPORT_FILE")"
    if [ "$fix_count" -eq 0 ]; then
      printf '_None_\n\n'
    else
      k=0
      while [ "$k" -lt "$fix_count" ]; do
        fix="$(jq -r ".[$i].manual_fixes[$k]" "$REPORT_FILE")"
        printf '%s\n' "- $fix"
        k=$((k + 1))
      done
      printf '\n'
    fi

    printf '### Residual Risk\n\n'
    if [ -z "$residual_risk" ] || [ "$residual_risk" = "null" ]; then
      printf '_None identified_\n\n'
    else
      printf '%s\n\n' "$residual_risk"
    fi

    i=$((i + 1))
  done
} > "$DOC_FILE"

# ── Compose autospec-doc --full ───────────────────────────────────────────────

if command -v autospec-doc > /dev/null 2>&1; then
  autospec-doc --full "$DOC_FILE" || true
fi

# ── Report output path ────────────────────────────────────────────────────────

printf '%s\n' "$DOC_FILE"
