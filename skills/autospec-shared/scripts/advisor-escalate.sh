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

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
advisor_config() { "$SCRIPT_DIR/advisor-config.sh" --key "$1" 2>/dev/null || printf ''; }

STATE_ROOT="${AUTOSPEC_ADVISOR_STATE_DIR:-$HOME/.autospec/advisor-state}"
# Config-resolved (advisor: block in .autospec/autospec.yml; env = CI/test override).
POLICY="$(advisor_config policy)"; POLICY="${POLICY:-auto}"
MAX_USES="$(advisor_config budget.max_calls_per_issue)"; MAX_USES="${MAX_USES:-3}"
MAX_CHARS="$(advisor_config budget.guidance_char_cap)"; MAX_CHARS="${MAX_CHARS:-2800}"
HARNESS="${AUTOSPEC_HARNESS:-claude}"

# Numeric issue guard — prevents path traversal / injection in the state path.
# Fail-open: a non-numeric issue is a usage error the caller treats as "skip".
case "$ISSUE" in
  ''|*[!0-9]*) printf 'advisor-escalate.sh: --issue must be a positive integer\n' >&2; exit 1 ;;
esac

repo_slug() { printf '%s' "$1" | tr '/' '_'; }

# Read the current per-issue use_count, coercing any corrupted/non-integer value
# to 0 so a hand-edited or truncated state file self-heals instead of crashing.
read_use_count() {
  local sf="$1" v=0
  if [ -f "$sf" ]; then
    v="$(jq -r '.use_count // 0' "$sf" 2>/dev/null || printf '0')"
  fi
  case "$v" in
    ''|*[!0-9]*) v=0 ;;
  esac
  printf '%s' "$v"
}

# The governed active gate set under `policy: auto` (self-tuned by
# advisor-govern.sh). AUTOSPEC_ADVISOR_ACTIVE_GATES is a CI/test override.
governed_active() {
  if [ -n "${AUTOSPEC_ADVISOR_ACTIVE_GATES:-}" ]; then
    printf '%s' "$AUTOSPEC_ADVISOR_ACTIVE_GATES" | tr ',' ' '
    return
  fi
  "$SCRIPT_DIR/advisor-govern.sh" show 2>/dev/null | jq -r '.active | join(" ")' 2>/dev/null || printf 'impl-haiku'
}

# Gate resolution follows the single policy knob, never a lever list:
#   off  → never; on → always; auto → gate is in the governed active set.
gate_enabled() {
  case "$POLICY" in
    off) return 1 ;;
    on) return 0 ;;
    auto)
      local active; active="$(governed_active)"
      case " $active " in *" $GATE "*) return 0 ;; *) return 1 ;; esac
      ;;
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
  # Single gate decision: policy off/on/auto + governed active set.
  if ! gate_enabled; then
    printf '{"decision":"DISABLED"}\n'
    exit 8
  fi

  # Validate inputs BEFORE touching state — a missing payload file must never
  # burn a cap slot. Fail-open: usage error the caller treats as "skip".
  if [ -z "$QUESTION_FILE" ] || [ ! -f "$QUESTION_FILE" ]; then
    printf 'advisor-escalate.sh: --question-file missing or not a file\n' >&2
    exit 1
  fi
  if [ -z "$CONTEXT_FILE" ] || [ ! -f "$CONTEXT_FILE" ]; then
    printf 'advisor-escalate.sh: --context-file missing or not a file\n' >&2
    exit 1
  fi

  local sf dir used next payload
  sf="$(state_file)"
  dir="$(dirname "$sf")"
  mkdir -p "$dir"

  used="$(read_use_count "$sf")"

  if [ "$used" -ge "$MAX_USES" ]; then
    printf '{"decision":"CAP-REACHED"}\n'
    exit 7
  fi

  # Curate the decision-scoped payload FIRST; only burn a cap slot once the
  # payload is fully materialized (so a mid-write failure never spends a slot).
  payload="$(mktemp -t advisor-payload-XXXXXX)"
  {
    printf '## Question\n'
    cat "$QUESTION_FILE"
    printf '\n\n## Context\n'
    cat "$CONTEXT_FILE"
  } > "$payload"

  next=$((used + 1))
  printf '{"use_count":%d}\n' "$next" > "$sf"

  jq -cn --arg h "$HARNESS" --arg cli "$(cli_fallback_for "$HARNESS")" \
        --argjson uc "$next" --arg pf "$payload" \
    '{decision:"GO",harness:$h,cli_fallback:$cli,use_count:$uc,payload_file:$pf}'
  exit 0
}

emit_stop_failsafe() {
  local reason="$1"
  jq -cn --arg g "$reason" '{verdict:"stop",guidance:$g,over_budget:false}'
}

# Append one telemetry record, always built with jq so values containing quotes,
# backslashes, or newlines can never emit an invalid JSONL line (which would
# later crash advisor-report.sh's jq parse).
append_telemetry() {
  local tf="$1" ts="$2" verdict="$3" uc="$4" tokens_out="$5" over="$6" failsafe="$7"
  jq -cn \
    --arg ts "$ts" --arg issue "$ISSUE" --arg repo "$REPO" --arg gate "$GATE" \
    --arg verdict "$verdict" --argjson uc "$uc" --argjson tout "$tokens_out" \
    --argjson ob "$over" --argjson fs "$failsafe" \
    '{ts:$ts,issue:$issue,repo:$repo,gate:$gate,verdict:$verdict,
      tokens_in:0,tokens_out:$tout,use_count:$uc,over_budget:$ob,failsafe:$fs}' \
    >> "$tf"
}

do_record() {
  local ts tf sf used verdict guidance over len out_len out_tokens
  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date +%Y-%m-%dT%H:%M:%SZ)"
  tf="$(telemetry_file)"
  sf="$(state_file)"
  used="$(read_use_count "$sf")"

  # Parse + validate. Any failure → fail-safe stop, exit 2.
  verdict="$(jq -r '.verdict // empty' "$RESPONSE_FILE" 2>/dev/null || printf '')"
  guidance="$(jq -r '.guidance // empty' "$RESPONSE_FILE" 2>/dev/null || printf '')"

  case "$verdict" in
    plan|correction|stop) : ;;
    *)
      emit_stop_failsafe "advisor response unparseable; fail-safe stop"
      append_telemetry "$tf" "$ts" "stop" "$used" 0 false true
      exit 2
      ;;
  esac

  # Strip known tool-call / function-call marker tags (advice-only). Downstream
  # must still treat guidance as untrusted prose — this is defense-in-depth, not
  # a guarantee.
  guidance="$(printf '%s' "$guidance" | sed -E 's#</?(antml:invoke|invoke|function_calls|tool[^>]*)[^>]*>##g')"

  # Budget: char-count proxy (~4 chars/token).
  over=false
  len=${#guidance}
  if [ "$len" -gt "$MAX_CHARS" ]; then
    guidance="${guidance:0:$MAX_CHARS}"
    over=true
  fi
  out_len=${#guidance}
  out_tokens=$(( (out_len + 3) / 4 ))

  jq -cn --arg v "$verdict" --arg g "$guidance" --argjson ob "$over" \
    '{verdict:$v,guidance:$g,over_budget:$ob}'

  append_telemetry "$tf" "$ts" "$verdict" "$used" "$out_tokens" "$over" false
  exit 0
}

case "$PHASE" in
  precheck) do_precheck ;;
  record) do_record ;;
  *) printf 'advisor-escalate.sh: --phase must be precheck or record\n' >&2; exit 1 ;;
esac
