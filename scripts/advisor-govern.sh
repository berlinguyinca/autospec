#!/usr/bin/env bash
# advisor-govern.sh — telemetry-driven self-governance of the advisor gate set.
#
# Under `policy: auto`, autospec decides which gates are active rather than the
# operator listing them. This is the ratchet the sweep loop ticks: it promotes
# the next gate in a fixed safety order only when the promotion signal
# (quality >= baseline AND cost <= baseline) holds over a minimum-sample floor,
# and retracts the last-added gate on regression. impl-haiku is the seed and is
# never retracted.
#
# Spec: docs/specs/2026-07-08-autospec-advisor-pattern-design.md §Self-governance
#
# Usage:
#   advisor-govern.sh show [--json]
#   advisor-govern.sh tick --telemetry <jsonl> \
#       --baseline-lgtm <f> --observed-lgtm <f> \
#       --baseline-cost <n> --observed-cost <n> [--min-samples N] [--json]
#
# State: ${AUTOSPEC_ADVISOR_STATE_DIR:-$HOME/.autospec/advisor-state}/active-gates.json
set -eu

# Fixed safety order — cheapest/lowest-risk first. impl-haiku is the seed.
ORDER="impl-haiku retry reviewer impl-decision"
SEED="impl-haiku"

CMD="${1:-}"; [ $# -gt 0 ] && shift || true

TELEMETRY="" MIN_SAMPLES=20
BL_LGTM="" OB_LGTM="" BL_COST="" OB_COST=""
while [ $# -gt 0 ]; do
  case "$1" in
    --telemetry) TELEMETRY="${2:-}"; shift 2 ;;
    --min-samples) MIN_SAMPLES="${2:-}"; shift 2 ;;
    --baseline-lgtm) BL_LGTM="${2:-}"; shift 2 ;;
    --observed-lgtm) OB_LGTM="${2:-}"; shift 2 ;;
    --baseline-cost) BL_COST="${2:-}"; shift 2 ;;
    --observed-cost) OB_COST="${2:-}"; shift 2 ;;
    --json) shift ;;   # JSON is the only output form; accepted for symmetry
    --help|-h) printf 'Usage: advisor-govern.sh show|tick ...\n'; exit 0 ;;
    *) printf 'advisor-govern.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

STATE_ROOT="${AUTOSPEC_ADVISOR_STATE_DIR:-$HOME/.autospec/advisor-state}"
STATE_FILE="$STATE_ROOT/active-gates.json"

# Emit a JSON array of the active gate list from a space-separated string.
active_json() { printf '%s\n' "$1" | tr ' ' '\n' | grep -v '^$' | jq -R . | jq -cs .; }

# Read the active set, sanitized: keep only known gates, in ORDER, and always
# include the seed. This self-heals a corrupted/hand-edited active-gates.json
# (unknown names, wrong order, non-array) and guarantees the retract `sed` only
# ever operates on metachar-free ORDER names.
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
    # Require a well-formed non-negative number (reject '', lone '.', '1.', '.5',
    # multi-dot) — an invalid --argjson would otherwise abort under set -e.
    for v in "$BL_LGTM" "$OB_LGTM" "$BL_COST" "$OB_COST"; do
      case "$v" in
        ''|*[!0-9.]*|*.*.*|.*|*.) printf 'advisor-govern.sh: baselines must be numeric: %s\n' "$v" >&2; exit 1 ;;
      esac
    done
    [ -f "$TELEMETRY" ] || { printf 'advisor-govern.sh: telemetry not found: %s\n' "$TELEMETRY" >&2; exit 1; }

    active="$(read_active)"
    # grep -c prints 0 AND exits 1 on an empty file; swallow the status without
    # emitting a second line (which would make `samples` non-numeric and, under
    # set -e, fall through to promote on zero evidence).
    samples="$(grep -c . "$TELEMETRY" 2>/dev/null || true)"
    samples="${samples:-0}"

    promote="$(jq -n --argjson bl "$BL_LGTM" --argjson ol "$OB_LGTM" \
                     --argjson bc "$BL_COST" --argjson oc "$OB_COST" \
                     '($ol >= $bl) and ($oc <= $bc)')"

    action="hold"
    if [ "$samples" -lt "$MIN_SAMPLES" ]; then
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
    printf 'advisor-govern.sh: command must be show or tick\n' >&2
    exit 1
    ;;
esac
