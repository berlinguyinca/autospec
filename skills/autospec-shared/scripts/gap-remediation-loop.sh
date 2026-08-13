#!/usr/bin/env bash
# gap-remediation-loop.sh — file surviving gaps from a gap JSON as auto-implement issues.
#
# Reads the gap JSON emitted by `/autospec-review --remediation --emit-gaps <path>`
# (schema: {gap_id,dimension,severity,file,line,title,body,dedupe_key}), dedupes each
# gap against (a) open issues whose body carries the same `dedupe_key` or whose title
# matches, and (b) open issues carrying an active `docs:drift` self-heal label with a
# matching title. Survivors are filed via `gh issue create --label
# needs-classify,gap-remediation,priority:high`, leaving Rust-backed admission to
# `/autospec-classify`; deterministic model-fit labels are backfilled best-effort.
# Round state is tracked in
# ~/.autospec/gap-round-state.json and capped at AUTOSPEC_GAP_MAX_ROUNDS (default 2).
#
# Usage:
#   gap-remediation-loop.sh --gaps <path> --file    # dedupe + file survivors
#   gap-remediation-loop.sh --help
#
# Environment:
#   AUTOSPEC_STATE_DIR        — state dir (default: ~/.autospec)
#   AUTOSPEC_SCRIPTS_DIR      — sibling scripts dir (default: script dir)
#   AUTOSPEC_GAP_REPO         — repo slug for gh (default: gh repo context)
#   AUTOSPEC_GAP_MAX_ROUNDS   — hard round cap (default: 2)
#
# Output (stdout, last line): "gap-remediation: survivors=<N> filed=<N> round=<N>"
#   When gaps are dropped: "gap-remediation: survivors=<N> filed=<N> round=<N> dropped=<N>"
#
# Exit codes:
#   0  always (best-effort; emits WARN on recoverable problems)
#   2  gh CLI absent (hard fail — cannot file issues)
#   3  input was non-empty but ALL gaps failed schema (nothing filed — fix the producer)
#
# Requires: bash 3.2+, gh, jq

set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"
STATE_DIR="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}"
MAX_ROUNDS="${AUTOSPEC_GAP_MAX_ROUNDS:-2}"
ROUND_STATE="$STATE_DIR/gap-round-state.json"
SKIP_FLAG="$STATE_DIR/no-review.flag"

GAPS_FILE=""
DO_FILE=0

# shellcheck source=gap-json-lib.sh
. "$AUTOSPEC_SCRIPTS_DIR/gap-json-lib.sh"
# gap-json-lib.sh runs `set +e` at file scope; re-assert our strict mode so the
# documented `set -eu` contract holds for the rest of this driver.
set -eu

while [ $# -gt 0 ]; do
  case "$1" in
    --gaps) GAPS_FILE="${2:-}"; shift 2 ;;
    --file) DO_FILE=1; shift ;;
    --help|-h)
      printf 'Usage: gap-remediation-loop.sh --gaps <path> --file\n'
      exit 0
      ;;
    *) shift ;;
  esac
done

_report() {
  local _survivors="$1" _filed="$2" _round="$3" _dropped="${4:-0}"
  if [ "$_dropped" -gt 0 ]; then
    printf 'gap-remediation: survivors=%s filed=%s round=%s dropped=%s\n' \
      "$_survivors" "$_filed" "$_round" "$_dropped"
  else
    printf 'gap-remediation: survivors=%s filed=%s round=%s\n' \
      "$_survivors" "$_filed" "$_round"
  fi
}

# _missing_fields <json-string> — print comma-separated list of missing/null required fields.
_missing_fields() {
  local obj="$1" key missing_list="" sep=""
  for key in gap_id dimension severity file line title body dedupe_key; do
    if ! printf '%s' "$obj" | jq -e --arg k "$key" 'has($k) and (.[$k] != null)' >/dev/null 2>&1; then
      missing_list="${missing_list}${sep}${key}"
      sep=","
    fi
  done
  printf '%s' "$missing_list"
}

# _gap_id_label <json-string> — print "(gap_id=<val>)" if gap_id present, else "".
_gap_id_label() {
  local obj="$1" gid
  gid="$(printf '%s' "$obj" | jq -r '.gap_id // empty' 2>/dev/null || true)"
  if [ -n "$gid" ]; then
    printf ' (gap_id=%s)' "$gid"
  fi
}

# Hard requirement: gh must exist to file issues.
if ! command -v gh >/dev/null 2>&1; then
  printf 'gap-remediation: ERROR gh CLI not found\n' >&2
  exit 2
fi
if ! command -v jq >/dev/null 2>&1; then
  printf 'gap-remediation: WARN jq not found; treating as 0 survivors\n' >&2
  _report 0 0 0
  exit 0
fi

# ── Read current round (default 0) ────────────────────────────────────────────
_current_round=0
if [ -f "$ROUND_STATE" ]; then
  _current_round="$(jq -r '.round // 0' "$ROUND_STATE" 2>/dev/null || echo 0)"
fi

# ── Skip-flag short-circuit ───────────────────────────────────────────────────
if [ -f "$SKIP_FLAG" ]; then
  printf 'gap-remediation: skip (no-review.flag present)\n' >&2
  _report 0 0 "$_current_round"
  exit 0
fi

# ── Round-cap enforcement ─────────────────────────────────────────────────────
if [ "$_current_round" -ge "$MAX_ROUNDS" ]; then
  printf 'gap-remediation: round cap reached (%s/%s); not filing\n' "$_current_round" "$MAX_ROUNDS" >&2
  _report 0 0 "$_current_round"
  exit 0
fi

# ── Parse gaps (malformed/empty → 0 survivors) ────────────────────────────────
if [ -z "$GAPS_FILE" ] || [ ! -f "$GAPS_FILE" ]; then
  printf 'gap-remediation: WARN no gaps file; treating as 0 survivors\n' >&2
  _report 0 0 "$_current_round"
  exit 0
fi
if ! jq -e 'type == "array"' "$GAPS_FILE" >/dev/null 2>&1; then
  printf 'gap-remediation: WARN malformed gap JSON; treating as 0 survivors\n' >&2
  _report 0 0 "$_current_round"
  exit 0
fi

_gap_count="$(jq 'length' "$GAPS_FILE" 2>/dev/null || echo 0)"
if [ "$_gap_count" -eq 0 ]; then
  _report 0 0 "$_current_round"
  exit 0
fi

# ── gh repo args (empty-array safe under set -u on bash 3.2) ───────────────────
_gh_issue_list() {
  if [ -n "${AUTOSPEC_GAP_REPO:-}" ]; then
    gh issue list --repo "$AUTOSPEC_GAP_REPO" "$@"
  else
    gh issue list "$@"
  fi
}
_gh_label_create() {
  if [ -n "${AUTOSPEC_GAP_REPO:-}" ]; then
    gh label create "$1" --repo "$AUTOSPEC_GAP_REPO" "${@:2}"
  else
    gh label create "$@"
  fi
}
_gh_issue_create() {
  if [ -n "${AUTOSPEC_GAP_REPO:-}" ]; then
    gh issue create --repo "$AUTOSPEC_GAP_REPO" "$@"
  else
    gh issue create "$@"
  fi
}
_gh_issue_edit() {
  if [ -n "${AUTOSPEC_GAP_REPO:-}" ]; then
    gh issue edit "$1" --repo "$AUTOSPEC_GAP_REPO" "${@:2}"
  else
    gh issue edit "$@"
  fi
}

# ── Snapshot open issues once (number, title, body, label names) ──────────────
_open_issues="$(_gh_issue_list --state open --limit 500 \
  --json number,title,body,labels 2>/dev/null || echo '[]')"
echo "$_open_issues" | jq -e 'type == "array"' >/dev/null 2>&1 || _open_issues='[]'

# _is_dup <dedupe_key> <title> → 0 if a matching open issue already exists.
_is_dup() {
  local dk="$1" title="$2"
  # (a) any open issue body contains the dedupe_key, OR title matches exactly.
  echo "$_open_issues" | jq -e --arg dk "$dk" --arg t "$title" '
    any(.[];
      ((.body // "") | contains($dk))
      or (.title == $t)
    )' >/dev/null 2>&1 && return 0
  # (b) any open issue with an active docs:drift label whose title matches.
  echo "$_open_issues" | jq -e --arg t "$title" '
    any(.[];
      (.title == $t)
      and ((.labels // []) | any(.name == "docs:drift"))
    )' >/dev/null 2>&1 && return 0
  return 1
}

# ── Walk gaps, dedupe, file survivors ─────────────────────────────────────────
# _seen_keys accumulates dedupe_key + title of survivors already counted this run
# so two identical gaps in the same JSON are not double-filed (the open-issue
# snapshot is taken once and does not reflect mid-run filings).
_seen_keys=""
_seen_in_run() {
  case "$_seen_keys" in
    *"<<$1>>"*) return 0 ;;
    *"<<$2>>"*) return 0 ;;
  esac
  return 1
}
_filed=0
_survivors=0
_dropped=0
_i=0
while [ "$_i" -lt "$_gap_count" ]; do
  _gap="$(jq -c ".[$_i]" "$GAPS_FILE")"
  _i=$((_i + 1))

  # Schema gate — skip invalid objects with a detailed warning.
  if ! gap_validate_object "$_gap"; then
    _dropped=$((_dropped + 1))
    _mf="$(_missing_fields "$_gap")"
    _gid_label="$(_gap_id_label "$_gap")"
    printf 'gap-remediation: WARN gap %s%s failed schema; missing: %s\n' \
      "$_i" "$_gid_label" "$_mf" >&2
    continue
  fi

  _dk="$(printf '%s' "$_gap" | jq -r '.dedupe_key')"
  _title="$(printf '%s' "$_gap" | jq -r '.title')"
  _body="$(printf '%s' "$_gap" | jq -r '.body')"
  _file="$(printf '%s' "$_gap" | jq -r '.file')"
  _line="$(printf '%s' "$_gap" | jq -r '.line')"
  _dim="$(printf '%s' "$_gap" | jq -r '.dimension')"
  _sev="$(printf '%s' "$_gap" | jq -r '.severity')"

  if _is_dup "$_dk" "$_title" || _seen_in_run "$_dk" "$_title"; then
    continue
  fi
  # Record this survivor so a later identical gap in the same JSON is deduped.
  _seen_keys="$_seen_keys<<$_dk>><<$_title>>"
  _survivors=$((_survivors + 1))

  [ "$DO_FILE" -eq 1 ] || continue

  # Compose the issue body with the dedupe_key embedded so future rounds match it.
  _issue_body="$(printf '%s\n\n---\n- dimension: %s\n- severity: %s\n- file: %s:%s\n- dedupe_key: %s\n' \
    "$_body" "$_dim" "$_sev" "$_file" "$_line" "$_dk")"

  # Ensure labels exist (idempotent, mirror classify idiom).
  _gh_label_create gap-remediation --color d4c5f9 --force >/dev/null 2>&1 || true
  _gh_label_create priority:high   --color e11d21 --force >/dev/null 2>&1 || true
  _gh_label_create needs-classify  --color fbca04 --force >/dev/null 2>&1 || true
  # origin:self provenance (issue #1785): idempotent, best-effort label
  _gh_label_create origin:self      --color 8250df --force >/dev/null 2>&1 || true

  # File the issue, retry once on failure (per spec error handling).
  _url=""
  _attempt=0
  while [ "$_attempt" -lt 2 ]; do
    _attempt=$((_attempt + 1))
    _url="$(_gh_issue_create \
      --title "$_title" \
      --body "$_issue_body" \
      --label "needs-classify,gap-remediation,priority:high,origin:self" 2>/dev/null || true)"
    [ -n "$_url" ] && break
  done

  if [ -z "$_url" ]; then
    printf 'gap-remediation: WARN failed to file gap "%s" after retry\n' "$_title" >&2
    continue
  fi
  _filed=$((_filed + 1))

  # Backfill ctx:*/reasoning:* via classify (best-effort; non-fatal).
  _num="$(printf '%s' "$_url" | grep -oE '[0-9]+$' || true)"
  if [ -n "$_num" ] && [ -x "$AUTOSPEC_SCRIPTS_DIR/classify-model-fit.sh" ]; then
    _tmp_body="$(mktemp)"
    printf '%s' "$_issue_body" > "$_tmp_body"
    _cls="$(bash "$AUTOSPEC_SCRIPTS_DIR/classify-model-fit.sh" "$_tmp_body" --json 2>/dev/null || true)"
    rm -f "$_tmp_body"
    if [ -n "$_cls" ]; then
      _ctx="$(printf '%s' "$_cls" | jq -r '.ctx // empty' 2>/dev/null || true)"
      _rsn="$(printf '%s' "$_cls" | jq -r '.reasoning // empty' 2>/dev/null || true)"
      if [ -n "$_ctx" ] && [ -n "$_rsn" ]; then
        _gh_issue_edit "$_num" --add-label "ctx:$_ctx,reasoning:$_rsn" >/dev/null 2>&1 || true
      fi
    fi
  fi
done

# ── All-dropped exit-3: input non-empty but every gap failed schema ────────────
if [ "$_gap_count" -gt 0 ] && [ "$_dropped" -eq "$_gap_count" ]; then
  printf 'gap-remediation: ERROR all %s gaps failed schema; nothing filed — fix the producer (see emit-gaps.sh) and re-run\n' \
    "$_gap_count" >&2
  _report 0 0 "$_current_round" "$_dropped"
  exit 3
fi

# ── Advance round state once a remediation round was attempted ────────────────
# Advance whenever filing was requested AND there were survivors to file —
# regardless of gh success — so persistent `gh issue create` failures still
# count down toward AUTOSPEC_GAP_MAX_ROUNDS and the loop is guaranteed to
# terminate. (Convergence with 0 survivors does not consume a round.)
if [ "$DO_FILE" -eq 1 ] && [ "$_survivors" -gt 0 ]; then
  mkdir -p "$STATE_DIR" 2>/dev/null || true
  _new_round=$((_current_round + 1))
  printf '{"round": %s, "max_rounds": %s, "last_filed": %s}\n' \
    "$_new_round" "$MAX_ROUNDS" "$_filed" > "$ROUND_STATE"
  _current_round="$_new_round"
fi

_report "$_survivors" "$_filed" "$_current_round" "$_dropped"
exit 0
