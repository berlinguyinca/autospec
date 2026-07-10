#!/usr/bin/env bash
# grooming-govern.sh — telemetry-driven self-governance of the backlog-grooming
# active-gate set. Mirrors advisor-govern.sh's ratchet: promotes the next gate
# in a fixed safety order only when the groomed clean-merge rate is >= the
# baseline clean-merge rate over a minimum-sample floor, and retracts the
# last-added gate on regression. eligible-promote is the seed and is never
# retracted.
#
# Usage:
#   grooming-govern.sh show
#   grooming-govern.sh tick --observed <json> [--min-samples N]
#
# --observed JSON shape (from grooming-observe.sh):
#   {"groomed_clean_merge_rate":F,"baseline_clean_merge_rate":F,"samples":N}
#
# State: ${AUTOSPEC_GROOMING_STATE_DIR:-$HOME/.autospec/grooming-state}/active-gates.json
set -eu

# Fixed safety order — eligible-promote is the seed.
ORDER="eligible-promote template-promote"
SEED="eligible-promote"

CMD="${1:-}"; [ $# -gt 0 ] && shift || true

OBSERVED="" MIN_SAMPLES=20
while [ $# -gt 0 ]; do
  case "$1" in
    --observed) OBSERVED="${2:-}"; shift 2 ;;
    --min-samples) MIN_SAMPLES="${2:-}"; shift 2 ;;
    --json) shift ;;   # JSON is the only output form; accepted for symmetry
    --help|-h) printf 'Usage: grooming-govern.sh show|tick ...\n'; exit 0 ;;
    *) printf 'grooming-govern.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

STATE_ROOT="${AUTOSPEC_GROOMING_STATE_DIR:-$HOME/.autospec/grooming-state}"
STATE_FILE="$STATE_ROOT/active-gates.json"

# Emit a JSON array of the active gate list from a space-separated string.
active_json() { printf '%s\n' "$1" | tr ' ' '\n' | grep -v '^$' | jq -R . | jq -cs .; }

# Read the active set, sanitized: keep only known gates, in ORDER, and always
# include the seed. This self-heals a corrupted/hand-edited active-gates.json
# (unknown names, wrong order, non-array).
read_active() {
  local raw="$SEED" a
  if [ -f "$STATE_FILE" ]; then
    a="$(jq -r '.active // [] | join(" ")' "$STATE_FILE" 2>/dev/null || printf '')"
    [ -n "$a" ] && raw="$a"
  fi
  local out="" g
  for g in $ORDER; do
    case " $raw " in *" $g "*) out="${out:+$out }$g" ;; esac
  done
  case " $out " in *" $SEED "*) : ;; *) out="$SEED${out:+ $out}" ;; esac
  printf '%s' "$out"
}

write_active() {
  mkdir -p "$STATE_ROOT"
  local tmp
  tmp="$(mktemp "${STATE_FILE}.XXXXXX")"
  jq -cn --argjson a "$(active_json "$1")" '{active:$a}' > "$tmp"
  mv "$tmp" "$STATE_FILE"   # atomic swap — no torn read under a concurrent tick
}

# Next gate in ORDER not yet in the active set; empty if the set is full.
next_gate() {
  local active="$1" g
  for g in $ORDER; do
    case " $active " in *" $g "*) : ;; *) printf '%s' "$g"; return ;; esac
  done
}

case "$CMD" in
  show)
    jq -cn --argjson a "$(active_json "$(read_active)")" '{active:$a}'
    exit 0
    ;;
  tick)
    if [ -z "$OBSERVED" ]; then
      printf 'grooming-govern.sh: --observed is required\n' >&2
      exit 1
    fi
    # Require well-formed JSON with numeric fields — an invalid --observed
    # would otherwise abort under set -e or silently promote on garbage.
    if ! printf '%s' "$OBSERVED" | jq -e '
          (.groomed_clean_merge_rate | type) == "number"
          and (.baseline_clean_merge_rate | type) == "number"
          and (.samples | type) == "number"
        ' >/dev/null 2>&1; then
      printf 'grooming-govern.sh: --observed must be JSON with numeric groomed_clean_merge_rate, baseline_clean_merge_rate, samples\n' >&2
      exit 1
    fi
    case "$MIN_SAMPLES" in
      ''|*[!0-9]*) printf 'grooming-govern.sh: --min-samples must be a non-negative integer: %s\n' "$MIN_SAMPLES" >&2; exit 1 ;;
    esac

    active="$(read_active)"
    samples="$(printf '%s' "$OBSERVED" | jq -r '.samples')"
    samples_int="$(printf '%s' "$OBSERVED" | jq -r '.samples | floor')"

    promote="$(printf '%s' "$OBSERVED" | jq '
      (.groomed_clean_merge_rate >= .baseline_clean_merge_rate)')"

    action="hold"
    if [ "$samples_int" -lt "$MIN_SAMPLES" ]; then
      action="hold"
    elif [ "$promote" = "true" ]; then
      ng="$(next_gate "$active")"
      if [ -n "$ng" ]; then
        active="$active $ng"
        action="promote"
      fi
    else
      # regression: drop the last-added gate, never the seed
      last="$(printf '%s' "$active" | awk '{print $NF}')"
      if [ "$last" != "$SEED" ]; then
        active="$(printf '%s' "$active" | sed "s/ ${last}\$//")"
        action="retract"
      fi
    fi

    write_active "$active"
    jq -cn --argjson a "$(active_json "$active")" --arg act "$action" \
           --argjson s "$samples" '{active:$a,action:$act,samples:$s}'
    exit 0
    ;;
  *)
    printf 'grooming-govern.sh: command must be show or tick\n' >&2
    exit 1
    ;;
esac
