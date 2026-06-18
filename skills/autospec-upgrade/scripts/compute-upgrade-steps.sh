#!/usr/bin/env bash
# skills/autospec-upgrade/scripts/compute-upgrade-steps.sh
#
# Compute the ordered incremental-major upgrade hops for the detected stack.
#
# CLI:
#   compute-upgrade-steps.sh --detect <detection.json> [--targets <targets.json>]
#                            [--target-angular N] [--target-next N] [--target-react N]
#
# Reads the detection JSON ({frameworks:[],versions:{<fw>:"maj.min.patch"}}) and a
# fixture-injected set of TARGET majors (no network), then emits, on stdout, a single
# JSON object of strictly-sequential one-major hops:
#   {"hops":[{"framework":"angular","from":20,"to":21}, ... ]}
# Angular (and Next/React) never skip a major: 20 -> 23 yields 21, 22, 23.
# A framework already at its target contributes no hops; all-at-latest -> {"hops":[]}.

set -uo pipefail

DETECT=""
TARGETS_FILE=""
# Indexed array of "fw=major" CLI overrides (bash 3.2 safe — no associative arrays).
OVERRIDES=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --detect)  DETECT="${2:-}"; shift 2 ;;
    --targets) TARGETS_FILE="${2:-}"; shift 2 ;;
    --target-angular) OVERRIDES+=("angular=${2:-}"); shift 2 ;;
    --target-next)    OVERRIDES+=("next=${2:-}");    shift 2 ;;
    --target-react)   OVERRIDES+=("react=${2:-}");   shift 2 ;;
    *) printf 'compute-upgrade-steps: unknown argument: %s\n' "$1" >&2; exit 2 ;;
  esac
done

if [ -z "$DETECT" ] || [ ! -f "$DETECT" ]; then
  printf 'compute-upgrade-steps: --detect <detection.json> is required and must exist\n' >&2
  exit 2
fi

detect_json="$(cat "$DETECT")"

# Base targets object from --targets file (default empty), then merge CLI overrides.
targets_json='{}'
if [ -n "$TARGETS_FILE" ]; then
  if [ ! -f "$TARGETS_FILE" ]; then
    printf 'compute-upgrade-steps: --targets file not found: %s\n' "$TARGETS_FILE" >&2
    exit 2
  fi
  targets_json="$(cat "$TARGETS_FILE")"
fi

for ov in "${OVERRIDES[@]:-}"; do
  [ -n "$ov" ] || continue
  fw="${ov%%=*}"
  maj="${ov#*=}"
  case "$maj" in
    ''|*[!0-9]*)
      printf 'compute-upgrade-steps: --target-%s must be an integer major, got: %s\n' "$fw" "$maj" >&2
      exit 2 ;;
  esac
  targets_json="$(printf '%s' "$targets_json" | jq -c --arg fw "$fw" --argjson maj "$maj" '. + {($fw): $maj}')"
done

# Strictly-sequential one-major hops per framework whose target exceeds its current major.
jq -cn \
  --argjson detect "$detect_json" \
  --argjson targets "$targets_json" '
  [ $detect.frameworks[]
    | select(. != "unknown")
    | . as $fw
    | (($detect.versions[$fw] // "0") | tostring | split(".")[0] | tonumber) as $cur
    | (($targets[$fw] // $cur)) as $tgt
    | range($cur + 1; $tgt + 1)
    | { framework: $fw, from: (. - 1), to: . }
  ]
  | { hops: . }
'
