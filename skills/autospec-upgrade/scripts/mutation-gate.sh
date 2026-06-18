#!/usr/bin/env bash
# mutation-gate.sh — Stryker mutation-testing adapter for autospec-upgrade
#
# Usage:
#   mutation-gate.sh --baseline [--detect <file>] [--runner <runner>] [--out <dir>]
#   mutation-gate.sh --gate <threshold> [--detect <file>] [--runner <runner>] [--out <dir>]
#
# Modes:
#   --baseline              Run Stryker, parse mutation score, write it as the
#                           baseline into <out>/.autospec/mutation-proof.json.
#                           Exits non-zero if Stryker produces no parseable score.
#   --gate <threshold>      Run Stryker, parse post-upgrade score.  Fails (exit != 0)
#                           when post-upgrade score < recorded baseline OR when
#                           post-upgrade score < explicit <threshold>.  Reports
#                           surviving mutants on failure.  Updates mutation-proof.json
#                           with post_upgrade, threshold, and passed fields.
#
# Options:
#   --detect <file>  Path to upgrade-detect.sh JSON (runners[] used for mapping).
#   --runner <name>  Explicit runner override: jest | vitest | karma.
#   --out <dir>      Output directory that contains (or will contain) .autospec/.
#                    Default: current working directory.
#
# mutation-proof.json shapes:
#   After --baseline:
#     {"baseline":{"score":<number>},"recorded_at":"<ISO8601>"}
#   After --gate:
#     {"baseline":{"score":<number>},"post_upgrade":{"score":<number>},
#      "threshold":<number>,"passed":<bool>,"gated_at":"<ISO8601>"}
#
# Exit codes:
#   0  — success (baseline recorded, or gate passed)
#   1  — gate failed (post-upgrade score < baseline or below threshold)
#   2  — Stryker produced no parseable mutation score
#   3  — argument / environment error

set -uo pipefail

# ── Constants ──────────────────────────────────────────────────────────────────

PROOF_FILENAME="mutation-proof.json"

# ── Argument parsing ───────────────────────────────────────────────────────────

MODE=""          # "baseline" or "gate"
THRESHOLD=""     # numeric threshold for --gate
DETECT_FILE=""
RUNNER_OVERRIDE=""
OUT_DIR=""

while [ $# -gt 0 ]; do
  case "$1" in
    --baseline)
      MODE="baseline"
      shift
      ;;
    --gate)
      MODE="gate"
      THRESHOLD="$2"
      shift 2
      ;;
    --gate=*)
      MODE="gate"
      THRESHOLD="${1#--gate=}"
      shift
      ;;
    --detect)
      DETECT_FILE="$2"
      shift 2
      ;;
    --detect=*)
      DETECT_FILE="${1#--detect=}"
      shift
      ;;
    --runner)
      RUNNER_OVERRIDE="$2"
      shift 2
      ;;
    --runner=*)
      RUNNER_OVERRIDE="${1#--runner=}"
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
    *)
      shift
      ;;
  esac
done

# ── Validate mode ──────────────────────────────────────────────────────────────

if [ -z "$MODE" ]; then
  printf 'mutation-gate: --baseline or --gate <threshold> is required\n' >&2
  exit 3
fi

if [ "$MODE" = "gate" ] && [ -z "$THRESHOLD" ]; then
  printf 'mutation-gate: --gate requires a numeric threshold argument\n' >&2
  exit 3
fi

# ── Resolve output directory ───────────────────────────────────────────────────

if [ -z "$OUT_DIR" ]; then
  OUT_DIR="$(pwd)/.autospec"
fi
mkdir -p "$OUT_DIR"

PROOF_FILE="$OUT_DIR/$PROOF_FILENAME"

# ── Detect runner ──────────────────────────────────────────────────────────────
# Priority: --runner flag > first entry in detect JSON > default "jest"

detect_runner() {
  local runner="jest"

  if [ -n "$RUNNER_OVERRIDE" ]; then
    runner="$RUNNER_OVERRIDE"
  elif [ -n "$DETECT_FILE" ] && [ -f "$DETECT_FILE" ]; then
    local first_runner
    first_runner="$(jq -r '.runners[0] // "jest"' "$DETECT_FILE" 2>/dev/null || printf 'jest')"
    if [ -n "$first_runner" ] && [ "$first_runner" != "null" ]; then
      runner="$first_runner"
    fi
  fi

  printf '%s' "$runner"
}

# ── Map runner to Stryker testRunner value ────────────────────────────────────
# Supported: jest → jest, vitest → vitest, karma → karma
# Fallback: jest

map_stryker_runner() {
  local runner="$1"
  case "$runner" in
    jest)    printf 'jest' ;;
    vitest)  printf 'vitest' ;;
    karma)   printf 'karma' ;;
    *)       printf 'jest' ;;
  esac
}

# ── Write Stryker config ───────────────────────────────────────────────────────

write_stryker_config() {
  local stryker_runner="$1"
  local cfg_file="$OUT_DIR/stryker.config.json"
  printf '{"testRunner":"%s","coverageAnalysis":"perTest"}\n' "$stryker_runner" > "$cfg_file"
  printf '%s' "$cfg_file"
}

# ── Run Stryker and parse mutation score ───────────────────────────────────────
# Returns: sets MUTATION_SCORE variable; exits non-zero if unparseable

run_stryker_and_parse() {
  local cfg_file="$1"
  local stryker_out
  # Run stryker; capture combined stdout+stderr
  stryker_out="$(npx stryker run --configFile "$cfg_file" 2>&1)" || {
    printf 'mutation-gate: npx stryker run exited non-zero\n' >&2
    printf '%s\n' "$stryker_out" >&2
    return 1
  }

  # Parse "Mutation score: <N>%" from output (case-insensitive, integer or decimal)
  local raw_score
  raw_score="$(printf '%s\n' "$stryker_out" | grep -i 'mutation score' | grep -o '[0-9]*\.[0-9]*\|[0-9]\+' | head -1)"

  if [ -z "$raw_score" ]; then
    printf 'mutation-gate: Stryker produced no parseable mutation score\n' >&2
    printf '%s\n' "$stryker_out" >&2
    return 2
  fi

  # Strip decimal for integer comparison (floor)
  MUTATION_SCORE="$(printf '%s' "$raw_score" | grep -o '^[0-9]*')"
  printf '%s\n' "$stryker_out" >&2

  # Capture surviving mutant count for reporting
  SURVIVING_MUTANTS="$(printf '%s\n' "$stryker_out" | grep -i 'survived' | grep -o '[0-9]\+' | head -1)"
  SURVIVING_MUTANTS="${SURVIVING_MUTANTS:-unknown}"
}

# ── ISO8601 timestamp ──────────────────────────────────────────────────────────

iso_now() {
  date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || printf 'unknown'
}

# ── --baseline mode ────────────────────────────────────────────────────────────

do_baseline() {
  local runner
  runner="$(detect_runner)"
  local stryker_runner
  stryker_runner="$(map_stryker_runner "$runner")"
  local cfg_file
  cfg_file="$(write_stryker_config "$stryker_runner")"

  MUTATION_SCORE=""
  SURVIVING_MUTANTS=""
  run_stryker_and_parse "$cfg_file" || exit $?

  local recorded_at
  recorded_at="$(iso_now)"

  # Write baseline proof
  printf '{"baseline":{"score":%s},"recorded_at":"%s"}\n' \
    "$MUTATION_SCORE" "$recorded_at" > "$PROOF_FILE"

  printf 'mutation-gate: baseline recorded — score=%s%% → %s\n' \
    "$MUTATION_SCORE" "$PROOF_FILE"
  exit 0
}

# ── --gate mode ────────────────────────────────────────────────────────────────

do_gate() {
  # Validate threshold is numeric
  case "$THRESHOLD" in
    ''|*[!0-9]*)
      printf 'mutation-gate: threshold must be a non-negative integer, got: %s\n' "$THRESHOLD" >&2
      exit 3
      ;;
  esac

  # Require existing proof file with a baseline score
  if [ ! -f "$PROOF_FILE" ]; then
    printf 'mutation-gate: mutation-proof.json not found at %s; run --baseline first\n' "$PROOF_FILE" >&2
    exit 3
  fi

  local baseline_score
  baseline_score="$(jq -r '.baseline.score // empty' "$PROOF_FILE" 2>/dev/null)"
  if [ -z "$baseline_score" ] || [ "$baseline_score" = "null" ]; then
    printf 'mutation-gate: mutation-proof.json has no baseline.score; run --baseline first\n' >&2
    exit 3
  fi

  local runner
  runner="$(detect_runner)"
  local stryker_runner
  stryker_runner="$(map_stryker_runner "$runner")"
  local cfg_file
  cfg_file="$(write_stryker_config "$stryker_runner")"

  MUTATION_SCORE=""
  SURVIVING_MUTANTS=""
  run_stryker_and_parse "$cfg_file" || exit $?

  local post_score="$MUTATION_SCORE"
  local gated_at
  gated_at="$(iso_now)"

  # Determine pass/fail
  local passed=true
  local fail_reason=""

  if [ "$post_score" -lt "$baseline_score" ]; then
    passed=false
    fail_reason="post-upgrade score ${post_score}% is below baseline ${baseline_score}%"
  fi

  if [ "$post_score" -lt "$THRESHOLD" ]; then
    passed=false
    if [ -n "$fail_reason" ]; then
      fail_reason="${fail_reason} and below threshold ${THRESHOLD}%"
    else
      fail_reason="post-upgrade score ${post_score}% is below threshold ${THRESHOLD}%"
    fi
  fi

  # Write updated proof (preserve baseline, add post_upgrade fields)
  local passed_json
  if [ "$passed" = "true" ]; then
    passed_json="true"
  else
    passed_json="false"
  fi

  # Read existing proof and merge — use jq to combine fields
  local merged
  merged="$(jq -n \
    --argjson existing "$(cat "$PROOF_FILE")" \
    --argjson post_score "$post_score" \
    --argjson threshold "$THRESHOLD" \
    --argjson passed "$passed_json" \
    --arg gated_at "$gated_at" \
    '$existing + {post_upgrade:{score:$post_score},threshold:$threshold,passed:$passed,gated_at:$gated_at}')"
  printf '%s\n' "$merged" > "$PROOF_FILE"

  if [ "$passed" = "false" ]; then
    printf 'mutation-gate: GATE FAILED — %s (surviving mutants: %s)\n' \
      "$fail_reason" "$SURVIVING_MUTANTS" >&2
    exit 1
  fi

  printf 'mutation-gate: gate passed — post-upgrade score=%s%% baseline=%s%% threshold=%s%%\n' \
    "$post_score" "$baseline_score" "$THRESHOLD"
  exit 0
}

# ── Dispatch ───────────────────────────────────────────────────────────────────

case "$MODE" in
  baseline) do_baseline ;;
  gate)     do_gate ;;
esac
