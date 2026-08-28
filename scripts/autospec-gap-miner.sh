#!/usr/bin/env bash
# autospec-gap-miner.sh — mine repeated autospec misses into gap-remediation drafts.
#
# Input is a text stream containing one gap event per line. Recognized event
# markers are REQUEST_CHANGES, FIX_COMMIT, and CI_BLOCKER. The miner normalizes
# each event to a stable dedupe key, updates a markdown repeat-count ledger, and
# emits draft GitHub issue payloads. With --file it dedupes via `gh issue list
# --search` before filing each draft with `gh issue create`.

set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
project_sync_issue() {
  helper="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR/../skills/autospec-shared/scripts}/project-sync-issue.sh"
  bash "$helper" "$1" "$PWD"
}

INPUT=""
LEDGER="docs/memory/autospec-gap-ledger.md"
REPO=""
MODE="dry-run"

usage() {
  cat <<'USAGE'
Usage: scripts/autospec-gap-miner.sh --input <path|-> [--ledger <path>] [--repo owner/repo] [--dry-run|--file]

Recognized line markers: REQUEST_CHANGES, FIX_COMMIT, CI_BLOCKER.
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --input) INPUT="${2:-}"; shift 2 ;;
    --ledger) LEDGER="${2:-}"; shift 2 ;;
    --repo) REPO="${2:-}"; shift 2 ;;
    --dry-run) MODE="dry-run"; shift ;;
    --file) MODE="file"; shift ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'autospec-gap-miner: unknown argument: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

if [ -z "$INPUT" ]; then
  printf 'autospec-gap-miner: --input is required\n' >&2
  usage >&2
  exit 2
fi
if ! command -v jq >/dev/null 2>&1; then
  printf 'autospec-gap-miner: jq is required\n' >&2
  exit 2
fi
if [ "$MODE" = "file" ] && ! command -v gh >/dev/null 2>&1; then
  printf 'autospec-gap-miner: gh is required for --file\n' >&2
  exit 2
fi

STDIN_TMP=""
if [ "$INPUT" = "-" ]; then
  STDIN_TMP="$(mktemp)"
  cat > "$STDIN_TMP"
  INPUT="$STDIN_TMP"
  trap 'rm -f "$STDIN_TMP"' EXIT
fi
if [ ! -f "$INPUT" ]; then
  printf 'autospec-gap-miner: input not found: %s\n' "$INPUT" >&2
  exit 2
fi

TODAY="$(date -u +%Y-%m-%d)"

ensure_ledger() {
  mkdir -p "$(dirname "$LEDGER")"
  if [ ! -f "$LEDGER" ]; then
    cat > "$LEDGER" <<'LEDGER'
# Autospec gap ledger

| dedupe_key | kind | area | repeat_count | priority | last_seen |
| --- | --- | --- | ---: | --- | --- |
LEDGER
  elif ! grep -q '^| dedupe_key | kind | area | repeat_count | priority | last_seen |$' "$LEDGER"; then
    tmp="$(mktemp)"
    {
      printf '# Autospec gap ledger\n\n'
      printf '| dedupe_key | kind | area | repeat_count | priority | last_seen |\n'
      printf '| --- | --- | --- | ---: | --- | --- |\n'
      cat "$LEDGER"
    } > "$tmp"
    mv "$tmp" "$LEDGER"
  fi
}

trim() {
  sed 's/^[[:space:]]*//;s/[[:space:]]*$//'
}

slugify() {
  printf '%s' "$1" \
    | tr '[:upper:]' '[:lower:]' \
    | sed 's/`//g; s#[^a-z0-9][^a-z0-9]*#-#g; s/^-//; s/-$//; s/--*/-/g' \
    | cut -c 1-120
}

ledger_count() {
  key="$1"
  awk -F'|' -v key="$key" '
    $0 ~ /^\|/ {
      k=$2; c=$5;
      gsub(/^ +| +$/, "", k); gsub(/^ +| +$/, "", c);
      if (k == key && c ~ /^[0-9]+$/) { print c; found=1; exit }
    }
    END { if (!found) print 0 }
  ' "$LEDGER"
}

ledger_upsert() {
  key="$1"; kind="$2"; area="$3"; count="$4"; priority="$5"
  tmp="$(mktemp)"
  awk -F'|' -v key="$key" '
    $0 ~ /^\|/ {
      k=$2; gsub(/^ +| +$/, "", k);
      if (k == key) next;
    }
    { print }
  ' "$LEDGER" > "$tmp"
  printf '| %s | %s | %s | %s | %s | %s |\n' "$key" "$kind" "$area" "$count" "$priority" "$TODAY" >> "$tmp"
  mv "$tmp" "$LEDGER"
}

json_draft() {
  jq -n \
    --arg source_type "$1" \
    --arg kind "$2" \
    --arg dedupe_key "$3" \
    --arg title "$4" \
    --arg body "$5" \
    --arg area "$6" \
    --arg priority "$7" \
    '{source_type:$source_type, kind:$kind, dedupe_key:$dedupe_key, title:$title, body:$body, priority:$priority, labels:["auto-implement","gap-remediation",$area,$priority]}'
}

line_kind() {
  line="$1"
  case "$line" in
    *REQUEST_CHANGES*) printf 'request_changes|REQUEST_CHANGES|area:review|Review gap' ;;
    *FIX_COMMIT*) printf 'fix_commit|FIX_COMMIT|area:code|Fix-commit gap' ;;
    *CI_BLOCKER*) printf 'ci_blocker|CI_BLOCKER|area:ci|CI blocker gap' ;;
    *) return 1 ;;
  esac
}

strip_marker() {
  marker="$1"; line="$2"
  printf '%s' "$line" | sed "s/.*${marker}[[:space:]]*:[[:space:]]*//; s/.*${marker}[[:space:]]\{1,\}//" | trim
}

_gh_issue_list() {
  if [ -n "$REPO" ]; then
    gh issue list --repo "$REPO" "$@"
  else
    gh issue list "$@"
  fi
}

_gh_issue_create() {
  if [ -n "$REPO" ]; then
    gh issue create --repo "$REPO" "$@"
  else
    gh issue create "$@"
  fi
}

_gh_label_create() {
  if [ -n "$REPO" ]; then
    gh label create "$1" --repo "$REPO" "${@:2}"
  else
    gh label create "$@"
  fi
}

issue_exists() {
  key="$1"; title="$2"
  # Acceptance-critical: use gh issue list --search before any create attempt.
  result="$(_gh_issue_list --state open --limit 20 --search "$key in:body" --json number,title 2>/dev/null || printf '[]')"
  printf '%s' "$result" | jq -e --arg title "$title" '
    type == "array" and ((length > 0) or any(.[]; ((.title // "") == $title)))
  ' >/dev/null 2>&1
}

file_draft() {
  draft="$1"
  key="$(printf '%s' "$draft" | jq -r '.dedupe_key')"
  title="$(printf '%s' "$draft" | jq -r '.title')"
  body="$(printf '%s' "$draft" | jq -r '.body + "\n\n---\n- dedupe_key: " + .dedupe_key')"
  labels="$(printf '%s' "$draft" | jq -r '.labels | join(",")')"
  if issue_exists "$key" "$title"; then
    return 0
  fi
  _gh_label_create gap-remediation --color d4c5f9 --force >/dev/null 2>&1 || true
  area_label="$(printf '%s' "$draft" | jq -r '.labels[] | select(startswith("area:"))' | head -1)"
  [ -n "$area_label" ] && _gh_label_create "$area_label" --color 5319e7 --force >/dev/null 2>&1 || true
  priority_label="$(printf '%s' "$draft" | jq -r '.priority')"
  _gh_label_create "$priority_label" --color e11d21 --force >/dev/null 2>&1 || true
  issue_url="$(_gh_issue_create --title "$title" --body "$body" --label "$labels")"
  project_sync_issue "$issue_url"
}

ensure_ledger
OUT_JSONL="$(mktemp)"
SEEN=""
trap 'rm -f "$STDIN_TMP" "$OUT_JSONL"' EXIT

while IFS= read -r raw_line || [ -n "$raw_line" ]; do
  parsed="$(line_kind "$raw_line" || true)"
  [ -n "$parsed" ] || continue
  kind="$(printf '%s' "$parsed" | cut -d'|' -f1)"
  source_type="$(printf '%s' "$parsed" | cut -d'|' -f2)"
  area="$(printf '%s' "$parsed" | cut -d'|' -f3)"
  prefix="$(printf '%s' "$parsed" | cut -d'|' -f4)"
  summary="$(strip_marker "$source_type" "$raw_line")"
  [ -n "$summary" ] || summary="$raw_line"
  key="$(slugify "${kind}-${summary}")"
  [ -n "$key" ] || continue
  case "$SEEN" in *"<<$key>>"*) continue ;; esac
  SEEN="$SEEN<<$key>>"
  prior="$(ledger_count "$key")"
  count="$((prior + 1))"
  priority="priority:medium"
  if [ "$count" -ge 2 ]; then
    priority="priority:high"
  fi
  ledger_upsert "$key" "$kind" "$area" "$count" "$priority"
  title="$prefix: $summary"
  body="Autospec mined a repeatable ${source_type} miss. Remediate the gap and add validation so future runs catch: ${summary}."
  draft="$(json_draft "$source_type" "$kind" "$key" "$title" "$body" "$area" "$priority")"
  printf '%s\n' "$draft" >> "$OUT_JSONL"
  if [ "$MODE" = "file" ]; then
    file_draft "$draft"
  fi
done < "$INPUT"

if [ -s "$OUT_JSONL" ]; then
  jq -s '.' "$OUT_JSONL"
else
  printf '[]\n'
fi
