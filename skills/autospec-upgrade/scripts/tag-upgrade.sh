#!/usr/bin/env bash
# tag-upgrade.sh — pre/post upgrade tagging + structured upgrade-report.json
#
# Usage:
#   tag-upgrade.sh pre  --framework <fw> --version <ver> [--out <dir>]
#   tag-upgrade.sh post --framework <fw> --version <ver>
#                       [--proof <mutation-proof.json>] [--out <dir>]
#   tag-upgrade.sh report --framework <fw> --from <ver> --to <ver>
#                         [--codemods <cmd>] [--manual-fixes <note>]
#                         [--residual-risk <text>] [--out <dir>]
#
# Modes:
#   pre     Create tag pre-upgrade-<fw>-<ver> at the start of a hop.
#   post    Create tag post-upgrade-<fw>-<ver> ONLY when the mutation gate
#           passed: proof.passed == true AND proof.post_upgrade.score >=
#           proof.baseline.score.  Withholds and exits non-zero otherwise.
#   report  Append a per-hop entry to <out>/.autospec/upgrade-report.json.
#           The file is an array; each entry has framework/from/to/codemods/
#           manual_fixes/residual_risk.  Idempotent append (merge strategy).
#
# Tag format:
#   pre-upgrade-<fw>-<ver>   fw ∈ angular|next|react
#   post-upgrade-<fw>-<ver>
#
# Exit codes:
#   0  — success
#   1  — post tag withheld (gate failed / score regressed)
#   2  — proof file missing or unparseable
#   3  — argument / environment error

set -uo pipefail

# ── Constants ──────────────────────────────────────────────────────────────────

REPORT_FILENAME="upgrade-report.json"

# ── Helpers ────────────────────────────────────────────────────────────────────

die() {
  printf 'tag-upgrade: %s\n' "$*" >&2
  exit 3
}

# ── Argument parsing ───────────────────────────────────────────────────────────

MODE=""
FRAMEWORK=""
VERSION=""
FROM_VER=""
TO_VER=""
PROOF_FILE=""
OUT_DIR=""
CODEMODS=""
MANUAL_FIXES=""
RESIDUAL_RISK=""

if [ $# -gt 0 ]; then
  MODE="$1"
  shift
fi

while [ $# -gt 0 ]; do
  case "$1" in
    --framework)   FRAMEWORK="$2";    shift 2 ;;
    --framework=*) FRAMEWORK="${1#--framework=}"; shift ;;
    --version)     VERSION="$2";      shift 2 ;;
    --version=*)   VERSION="${1#--version=}";     shift ;;
    --from)        FROM_VER="$2";     shift 2 ;;
    --from=*)      FROM_VER="${1#--from=}";       shift ;;
    --to)          TO_VER="$2";       shift 2 ;;
    --to=*)        TO_VER="${1#--to=}";           shift ;;
    --proof)       PROOF_FILE="$2";   shift 2 ;;
    --proof=*)     PROOF_FILE="${1#--proof=}";    shift ;;
    --out)         OUT_DIR="$2";      shift 2 ;;
    --out=*)       OUT_DIR="${1#--out=}";         shift ;;
    --codemods)    CODEMODS="$2";     shift 2 ;;
    --codemods=*)  CODEMODS="${1#--codemods=}";   shift ;;
    --manual-fixes)  MANUAL_FIXES="$2";  shift 2 ;;
    --manual-fixes=*) MANUAL_FIXES="${1#--manual-fixes=}"; shift ;;
    --residual-risk)  RESIDUAL_RISK="$2"; shift 2 ;;
    --residual-risk=*) RESIDUAL_RISK="${1#--residual-risk=}"; shift ;;
    *) shift ;;
  esac
done

# ── Resolve output directory ───────────────────────────────────────────────────

if [ -z "$OUT_DIR" ]; then
  OUT_DIR="$(pwd)"
fi
AUTOSPEC_DIR="$OUT_DIR/.autospec"
mkdir -p "$AUTOSPEC_DIR"

# ── Mode: pre ─────────────────────────────────────────────────────────────────

do_pre() {
  if [ -z "$FRAMEWORK" ]; then
    printf 'tag-upgrade: --framework is required\n' >&2
    exit 3
  fi
  if [ -z "$VERSION" ]; then
    printf 'tag-upgrade: --version is required\n' >&2
    exit 3
  fi

  local tag_name="pre-upgrade-${FRAMEWORK}-${VERSION}"
  git tag "$tag_name"
  printf 'tag-upgrade: created tag %s\n' "$tag_name"
  exit 0
}

# ── Mode: post ────────────────────────────────────────────────────────────────

# Evaluate the mutation-proof gate at $1. Return 0 if the post tag may be
# created, 1 if it must be withheld (prints reason), 2 if the proof is
# missing/malformed. Uses if/then/else to distinguish JSON false from a missing
# "passed" key (jq `false // x` would wrongly treat false as absent).
gate_evaluate() {
  local proof="$1"
  if [ ! -f "$proof" ]; then
    printf 'tag-upgrade: proof file not found: %s\n' "$proof" >&2
    return 2
  fi
  local passed
  passed="$(jq -r 'if has("passed") then (if .passed then "true" else "false" end) else "null" end' "$proof" 2>/dev/null)"
  if [ "$passed" = "null" ] || [ -z "$passed" ]; then
    printf 'tag-upgrade: proof file missing "passed" field: %s\n' "$proof" >&2
    return 2
  fi
  if [ "$passed" != "true" ]; then
    printf 'tag-upgrade: WITHHELD post tag — mutation gate not passed (passed=%s)\n' "$passed" >&2
    return 1
  fi
  local baseline_score post_score bs_int ps_int
  baseline_score="$(jq -r '.baseline.score // "null"' "$proof" 2>/dev/null)"
  post_score="$(jq -r '.post_upgrade.score // "null"' "$proof" 2>/dev/null)"
  if [ "$baseline_score" != "null" ] && [ "$post_score" != "null" ]; then
    bs_int="$(printf '%s' "$baseline_score" | grep -o '^[0-9]*')"
    ps_int="$(printf '%s' "$post_score" | grep -o '^[0-9]*')"
    if [ -n "$bs_int" ] && [ -n "$ps_int" ] && [ "$ps_int" -lt "$bs_int" ]; then
      printf 'tag-upgrade: WITHHELD post tag — post_upgrade score %s is below baseline %s\n' "$ps_int" "$bs_int" >&2
      return 1
    fi
  fi
  return 0
}

do_post() {
  if [ -z "$FRAMEWORK" ]; then printf 'tag-upgrade: --framework is required\n' >&2; exit 3; fi
  if [ -z "$VERSION" ];   then printf 'tag-upgrade: --version is required\n'   >&2; exit 3; fi
  if [ -z "$PROOF_FILE" ]; then PROOF_FILE="$AUTOSPEC_DIR/mutation-proof.json"; fi
  gate_evaluate "$PROOF_FILE" || exit $?
  local tag_name="post-upgrade-${FRAMEWORK}-${VERSION}"
  git tag "$tag_name"
  printf 'tag-upgrade: created tag %s\n' "$tag_name"
  exit 0
}

# ── Mode: report ──────────────────────────────────────────────────────────────

# Build one per-hop report entry JSON from the CLI-provided fields.
build_hop_entry() {
  local codemods_json="[]" manual_fixes_json="[]"
  if [ -n "$CODEMODS" ];     then codemods_json="$(jq -n --arg c "$CODEMODS" '[$c]')"; fi
  if [ -n "$MANUAL_FIXES" ]; then manual_fixes_json="$(jq -n --arg m "$MANUAL_FIXES" '[$m]')"; fi
  jq -n \
    --arg framework "$FRAMEWORK" \
    --arg from "$FROM_VER" \
    --arg to "$TO_VER" \
    --argjson codemods "$codemods_json" \
    --argjson manual_fixes "$manual_fixes_json" \
    --arg residual_risk "${RESIDUAL_RISK:-}" \
    '{framework:$framework, from:$from, to:$to, codemods:$codemods, manual_fixes:$manual_fixes, residual_risk:$residual_risk}'
}

do_report() {
  if [ -z "$FRAMEWORK" ]; then
    printf 'tag-upgrade: --framework is required for report\n' >&2
    exit 3
  fi
  if [ -z "$FROM_VER" ]; then
    printf 'tag-upgrade: --from is required for report\n' >&2
    exit 3
  fi
  if [ -z "$TO_VER" ]; then
    printf 'tag-upgrade: --to is required for report\n' >&2
    exit 3
  fi

  local report_file="$AUTOSPEC_DIR/$REPORT_FILENAME"

  local new_entry
  new_entry="$(build_hop_entry)"

  # Read existing report or start fresh array
  local existing="[]"
  if [ -f "$report_file" ]; then
    existing="$(cat "$report_file")"
    # Validate it's an array; reset if not
    if ! printf '%s' "$existing" | jq -e 'type == "array"' > /dev/null 2>&1; then
      existing="[]"
    fi
  fi

  # Append new entry and write
  local updated
  updated="$(printf '%s\n%s\n' "$existing" "$new_entry" | jq -s '.[0] + [.[1]]')"
  printf '%s\n' "$updated" > "$report_file"

  printf 'tag-upgrade: report written to %s\n' "$report_file"
  exit 0
}

# ── Dispatch ───────────────────────────────────────────────────────────────────

case "$MODE" in
  pre)    do_pre ;;
  post)   do_post ;;
  report) do_report ;;
  "")     die "mode (pre|post|report) is required as first argument" ;;
  *)      die "unknown mode: ${MODE}; expected pre|post|report" ;;
esac
