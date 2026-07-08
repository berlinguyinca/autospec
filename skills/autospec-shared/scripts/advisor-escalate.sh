#!/usr/bin/env bash
# advisor-escalate.sh — deterministic bookkeeping for the autospec advisor pattern.
#
# The model call is NOT made here; dispatch is the executor's prose contract
# (see the `## Advisor escalation` section in autospec-run/SKILL.md). This script
# owns only the deterministic parts: flag/gate gating, the per-issue cap, context
# curation, response validation, and telemetry.
#
# Spec: docs/specs/2026-07-08-autospec-advisor-pattern-design.md
#
# Usage:
#   advisor-escalate.sh --phase precheck --issue <N> --repo <owner/repo> --gate <id> \
#                       --question-file <f> --context-file <f> [--json]
#   advisor-escalate.sh --phase record   --issue <N> --repo <owner/repo> --gate <id> \
#                       --response-file <f> [--json]
#
# Exit codes:
#   precheck: 0 GO · 7 cap-reached · 8 gate-disabled
#   record:   0 valid guidance · 2 invalid/fail-open (emits fail-safe stop)
# Any non-zero exit means the caller should proceed WITHOUT an advisor.
set -eu

PHASE="" ISSUE="" REPO="" GATE=""
QUESTION_FILE="" CONTEXT_FILE="" RESPONSE_FILE=""
JSON_MODE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --phase) PHASE="${2:-}"; shift 2 ;;
    --issue) ISSUE="${2:-}"; shift 2 ;;
    --repo) REPO="${2:-}"; shift 2 ;;
    --gate) GATE="${2:-}"; shift 2 ;;
    --question-file) QUESTION_FILE="${2:-}"; shift 2 ;;
    --context-file) CONTEXT_FILE="${2:-}"; shift 2 ;;
    --response-file) RESPONSE_FILE="${2:-}"; shift 2 ;;
    --json) JSON_MODE=1; shift ;;
    --help|-h) printf 'Usage: advisor-escalate.sh --phase precheck|record ...\n'; exit 0 ;;
    *) printf 'advisor-escalate.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

STATE_ROOT="${AUTOSPEC_ADVISOR_STATE_DIR:-$HOME/.autospec/advisor-state}"
MAX_USES="${AUTOSPEC_ADVISOR_MAX_USES:-3}"
MAX_CHARS="${AUTOSPEC_ADVISOR_MAX_CHARS:-2800}"
ENABLED="${AUTOSPEC_ADVISOR:-off}"
GATES="${AUTOSPEC_ADVISOR_GATES:-impl-haiku}"
HARNESS="${AUTOSPEC_HARNESS:-claude}"

repo_slug() { printf '%s' "$1" | tr '/' '_'; }

gate_enabled() {
  # word-boundary membership test on a comma list, no regex injection
  case ",${GATES}," in
    *",${GATE},"*) return 0 ;;
    *) return 1 ;;
  esac
}

cli_fallback_for() {
  case "$1" in
    codex) printf 'codex exec' ;;
    opencode) printf 'opencode run' ;;
    *) printf 'claude -p --model opus' ;;
  esac
}

state_file() {
  printf '%s/%s/%s.json' "$STATE_ROOT" "$(repo_slug "$REPO")" "$ISSUE"
}

telemetry_file() {
  local dir="${AUTOSPEC_TELEMETRY_DIR:-.autospec/telemetry}"
  mkdir -p "$dir"
  printf '%s/advisor-escalate.jsonl' "$dir"
}

do_precheck() {
  if [ "$ENABLED" != "on" ]; then
    printf '{"decision":"DISABLED"}\n'
    exit 8
  fi
  if ! gate_enabled; then
    printf '{"decision":"DISABLED"}\n'
    exit 8
  fi

  local sf dir used next payload
  sf="$(state_file)"
  dir="$(dirname "$sf")"
  mkdir -p "$dir"

  used=0
  if [ -f "$sf" ]; then
    used="$(jq -r '.use_count // 0' "$sf" 2>/dev/null || printf '0')"
  fi

  if [ "$used" -ge "$MAX_USES" ]; then
    printf '{"decision":"CAP-REACHED"}\n'
    exit 7
  fi

  next=$((used + 1))
  printf '{"use_count":%d}\n' "$next" > "$sf"

  # Curate a decision-scoped payload: question first, then context.
  payload="$(mktemp -t advisor-payload-XXXXXX)"
  {
    printf '## Question\n'
    cat "$QUESTION_FILE"
    printf '\n\n## Context\n'
    cat "$CONTEXT_FILE"
  } > "$payload"

  jq -cn --arg h "$HARNESS" --arg cli "$(cli_fallback_for "$HARNESS")" \
        --argjson uc "$next" --arg pf "$payload" \
    '{decision:"GO",harness:$h,cli_fallback:$cli,use_count:$uc,payload_file:$pf}'
  exit 0
}

emit_stop_failsafe() {
  local reason="$1"
  jq -cn --arg g "$reason" '{verdict:"stop",guidance:$g,over_budget:false}'
}

do_record() {
  local ts tf verdict guidance over len out_len
  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date +%Y-%m-%dT%H:%M:%SZ)"
  tf="$(telemetry_file)"

  # Parse + validate. Any failure → fail-safe stop, exit 2.
  verdict="$(jq -r '.verdict // empty' "$RESPONSE_FILE" 2>/dev/null || printf '')"
  guidance="$(jq -r '.guidance // empty' "$RESPONSE_FILE" 2>/dev/null || printf '')"

  case "$verdict" in
    plan|correction|stop) : ;;
    *)
      emit_stop_failsafe "advisor response unparseable; fail-safe stop"
      printf '{"ts":"%s","issue":"%s","repo":"%s","gate":"%s","verdict":"stop","tokens_in":0,"tokens_out":0,"over_budget":false,"failsafe":true}\n' \
        "$ts" "$ISSUE" "$REPO" "$GATE" >> "$tf"
      exit 2
      ;;
  esac

  # Strip anything that looks like a tool call marker (advice-only).
  guidance="$(printf '%s' "$guidance" | sed -e 's/<[^>]*tool[^>]*>//g')"

  # Budget: char-count proxy (~4 chars/token).
  over=false
  len=${#guidance}
  if [ "$len" -gt "$MAX_CHARS" ]; then
    guidance="${guidance:0:$MAX_CHARS}"
    over=true
  fi
  out_len=${#guidance}

  jq -cn --arg v "$verdict" --arg g "$guidance" --argjson ob "$over" \
    '{verdict:$v,guidance:$g,over_budget:$ob}'

  printf '{"ts":"%s","issue":"%s","repo":"%s","gate":"%s","verdict":"%s","tokens_in":0,"tokens_out":%d,"over_budget":%s}\n' \
    "$ts" "$ISSUE" "$REPO" "$GATE" "$verdict" "$out_len" "$over" >> "$tf"
  exit 0
}

case "$PHASE" in
  precheck) do_precheck ;;
  record) do_record ;;
  *) printf 'advisor-escalate.sh: --phase must be precheck or record\n' >&2; exit 1 ;;
esac
