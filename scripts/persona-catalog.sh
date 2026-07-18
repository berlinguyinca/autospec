#!/usr/bin/env bash
# persona-catalog.sh — merged read-only bundled + writable user persona catalog.
#
# Usage:
#   bash scripts/persona-catalog.sh list
#   bash scripts/persona-catalog.sh load <id>
#   bash scripts/persona-catalog.sh select-overlay [--title T] [--body B] [--labels L]
#   bash scripts/persona-catalog.sh compose-overlay <base-file> [--title T] [--body B] [--labels L]
#
# Backends:
#   1. user-state: ~/.autospec/personas/ (or $AUTOSPEC_HOME/personas)
#   2. bundled:    personas/catalog/ inside this repository
#
# User-state entries shadow bundled entries with the same front-matter id.
# select-overlay honors AUTOSPEC_PERSONA_MATCH_CMD; command errors fail open.

set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BUNDLED_DIR="$REPO_ROOT/personas/catalog"
USER_DIR="$HOME/.autospec/personas"

usage() {
  cat >&2 <<'EOF_USAGE'
Usage:
  persona-catalog.sh list
  persona-catalog.sh load <id>
  persona-catalog.sh select-overlay [--title T] [--body B] [--labels L]
  persona-catalog.sh compose-overlay <base-file> [--title T] [--body B] [--labels L]
EOF_USAGE
}

warn() { printf 'persona-catalog: %s\n' "$*" >&2; }

front_matter_id() {
  awk '
    BEGIN { closed = 0 }
    NR == 1 && $0 != "---" { exit 1 }
    NR == 1 { in_fm = 1; next }
    in_fm && $0 == "---" { closed = 1; if (id != "") { print id; exit 0 } exit 1 }
    in_fm && $0 ~ /^id:[[:space:]]*/ {
      id = $0; sub(/^id:[[:space:]]*/, "", id); gsub(/^[[:space:]]+|[[:space:]]+$/, "", id)
      if (id !~ /^[A-Za-z0-9._-]+$/) { exit 1 }
    }
    END { if (!closed) { exit 1 } }
  ' "$1"
}

scan_backend() {
  _backend="$1"; _dir="$2"
  [ -d "$_dir" ] || return 0
  for _file in "$_dir"/*.md; do
    [ -f "$_file" ] || continue
    if _id="$(front_matter_id "$_file")"; then
      printf '%s\t%s\t%s\n' "$_id" "$_backend" "$_file"
    else
      warn "warning: malformed front matter: $_file"
    fi
  done
}

entry_path_for_id() { awk -F '\t' -v id="$2" '$1 == id { print $3; exit 0 }' "$1"; }
entry_has_id() { awk -F '\t' -v id="$2" '$1 == id { found = 1 } END { exit found ? 0 : 1 }' "$1"; }

with_entries() {
  scan_backend "user-state" "$USER_DIR" | sort -t "$(printf '\t')" -k1,1 -k3,3 > "$1"
  scan_backend "bundled" "$BUNDLED_DIR" | sort -t "$(printf '\t')" -k1,1 -k3,3 > "$2"
}

list_ids() {
  _tmp_dir="$(mktemp -d -t persona-catalog.XXXXXX)"; _user_entries="$_tmp_dir/user.tsv"; _bundled_entries="$_tmp_dir/bundled.tsv"; _ids="$_tmp_dir/ids.txt"
  with_entries "$_user_entries" "$_bundled_entries"; : > "$_ids"
  awk -F '\t' '{ print $1 }' "$_user_entries" >> "$_ids"
  while IFS="$(printf '\t')" read -r _id _backend _path; do
    if entry_has_id "$_user_entries" "$_id"; then warn "user-state shadows bundled id: $_id"; else printf '%s\n' "$_id" >> "$_ids"; fi
  done < "$_bundled_entries"
  sort -u "$_ids"; rm -rf "$_tmp_dir"
}

load_id() {
  _id="$1"; _tmp_dir="$(mktemp -d -t persona-catalog.XXXXXX)"; _user_entries="$_tmp_dir/user.tsv"; _bundled_entries="$_tmp_dir/bundled.tsv"
  with_entries "$_user_entries" "$_bundled_entries"
  _user_path="$(entry_path_for_id "$_user_entries" "$_id")"; _bundled_path="$(entry_path_for_id "$_bundled_entries" "$_id")"
  if [ -n "$_user_path" ]; then
    [ -z "$_bundled_path" ] || warn "user-state shadows bundled id: $_id"
    cat "$_user_path"; rm -rf "$_tmp_dir"; return 0
  fi
  if [ -n "$_bundled_path" ]; then cat "$_bundled_path"; rm -rf "$_tmp_dir"; return 0; fi
  warn "id not found: $_id"; rm -rf "$_tmp_dir"; return 1
}

parse_issue_args() {
  ISSUE_TITLE="${AUTOSPEC_PERSONA_ISSUE_TITLE:-}"; ISSUE_BODY="${AUTOSPEC_PERSONA_ISSUE_BODY:-}"; ISSUE_LABELS="${AUTOSPEC_PERSONA_ISSUE_LABELS:-}"
  while [ $# -gt 0 ]; do
    case "$1" in
      --title) ISSUE_TITLE="$2"; shift 2 ;;
      --body) ISSUE_BODY="$2"; shift 2 ;;
      --labels) ISSUE_LABELS="$2"; shift 2 ;;
      *) usage; exit 1 ;;
    esac
  done
}

run_match_override() {
  _match_out="$(AUTOSPEC_PERSONA_ISSUE_TITLE="$ISSUE_TITLE" AUTOSPEC_PERSONA_ISSUE_BODY="$ISSUE_BODY" AUTOSPEC_PERSONA_ISSUE_LABELS="$ISSUE_LABELS" sh -c "$AUTOSPEC_PERSONA_MATCH_CMD")" || return 1
  printf '%s\n' "$_match_out" | sed -n '1p' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//'
}

PERSONA_OVERLAY_PY="$(cat <<'PY_OVERLAY'
import os, re, sys
bundled, user_dir, title, body, labels = sys.argv[1:6]
issue = f"{title} {labels} {body}"
if not issue.strip():
    sys.exit(0)
stop = set("the and for with that this when into from issue task add fix use uses need needs follow up".split())
def words(text):
    return {w for w in re.findall(r"[a-z0-9]+", text.lower()) if len(w) >= 3 and w not in stop}
def front(path):
    meta, lines = {}, open(path, encoding="utf-8").read().splitlines()
    if not lines or lines[0] != "---":
        return meta
    for line in lines[1:]:
        if line == "---":
            break
        if ":" in line:
            k, v = line.split(":", 1); meta[k.strip()] = v.strip()
    return meta
def entries(root):
    out = {}
    if os.path.isdir(root):
        for name in sorted(os.listdir(root)):
            if name.endswith(".md"):
                path = os.path.join(root, name); mid = front(path).get("id", "")
                if re.match(r"^[A-Za-z0-9._-]+$", mid):
                    out[mid] = path
    return out
user, base = entries(user_dir), entries(bundled)
effective = {k: v for k, v in base.items() if k not in user}
issue_lc = issue.lower()
def alias(mid):
    patterns = {
        "security-hardener": r"(^|[^a-z0-9])(security|sec|privacy|credential|credentials|secret|secrets|token|tokens|trust|audit|auth|vulnerab)",
        "performance-engineer": r"(^|[^a-z0-9])(perf|performance|latency|throughput|memory|startup|speed|slow|cache|cost|optimi[sz])",
        "docs-writer": r"(^|[^a-z0-9])(docs|documentation|readme|guide|runbook)",
        "test-strengthener": r"(^|[^a-z0-9])(test|tests|coverage|regression|bats|playwright)",
        "refactorer": r"(^|[^a-z0-9])(cleanup|clean-up|refactor|simplify|deslop)",
    }
    return 5 if mid in patterns and re.search(patterns[mid], issue_lc) else 0
def score(mid, path):
    text = open(path, encoding="utf-8").read(); meta = front(path)
    haystack = " ".join([mid, meta.get("title", ""), meta.get("applies_when", ""), text])
    return len(words(issue) & words(haystack)) + alias(mid)
def best(candidates, threshold):
    scored = sorted(((score(mid, path), mid) for mid, path in candidates.items()), reverse=True)
    return scored[0][1] if scored and scored[0][0] >= threshold else ""
match = best(effective, 2) or best(user, 2)
if match:
    print(match); sys.exit(0)
os.makedirs(user_dir, exist_ok=True)
slug_words = list(words(issue))[:5] or ["issue", "overlay"]
mid = "generated-" + "-".join(slug_words)
path = os.path.join(user_dir, mid + ".md")
if not os.path.exists(path):
    with open(path, "w", encoding="utf-8") as f:
        f.write(f"---\nid: {mid}\ntitle: Generated issue archetype\napplies_when: {title} {labels}\nautospec_generated: true\n---\n# Generated Issue Archetype\n\nUse this archetype when a task resembles: {title}.\n\nLabels: {labels or 'none'}\n\nBias the work toward the smallest issue-specific overlay that preserves the base operator persona unchanged after the current issue completes.\n")
print(mid)
PY_OVERLAY
)"

select_overlay() {
  parse_issue_args "$@"
  _issue_text="${ISSUE_TITLE} ${ISSUE_LABELS} ${ISSUE_BODY}"
  [ -n "$(printf '%s' "$_issue_text" | tr -d '[:space:]')" ] || return 0
  if [ -n "${AUTOSPEC_PERSONA_MATCH_CMD:-}" ]; then
    if _override_id="$(run_match_override 2>/dev/null)"; then
      case "$_override_id" in ""|none|no-match) return 0 ;; *) printf '%s\n' "$_override_id"; return 0 ;; esac
    fi
    return 0
  fi
  command -v python3 >/dev/null 2>&1 || return 0
  python3 -c "$PERSONA_OVERLAY_PY" "$BUNDLED_DIR" "$USER_DIR" "$ISSUE_TITLE" "$ISSUE_BODY" "$ISSUE_LABELS"
}

compose_overlay() {
  _base_file="$1"; shift; cat "$_base_file"
  _overlay_id="$(select_overlay "$@" 2>/dev/null || true)"
  if [ -n "$_overlay_id" ]; then
    printf '
## Issue archetype overlay

_Selected persona archetype: `%s`._

' "$_overlay_id"
    load_id "$_overlay_id" 2>/dev/null || true; printf '
'
  fi
}

if [ $# -lt 1 ]; then usage; exit 1; fi
case "$1" in
  list) [ $# -eq 1 ] || { usage; exit 1; }; list_ids ;;
  load) [ $# -eq 2 ] || { usage; exit 1; }; load_id "$2" ;;
  select-overlay) shift; select_overlay "$@" ;;
  compose-overlay) [ $# -ge 2 ] || { usage; exit 1; }; _base_file="$2"; shift 2; compose_overlay "$_base_file" "$@" ;;
  -h|--help) usage ;;
  *) usage; exit 1 ;;
esac
