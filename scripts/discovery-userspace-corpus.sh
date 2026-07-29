#!/usr/bin/env bash
# discovery-userspace-corpus.sh — userspace-corpus harvester (issue #1657).
# Read-only scan of sibling peer repos in the operator's workspace to find
# capabilities (skills/scripts/docs) peer repos have that this repo lacks, or
# patterns to adopt. Never writes anything into a peer repo. Emits trend-signal
# records with source="userspace-corpus" via trend-ledger.sh (§ shared contracts).
#
# Usage:
#   discovery-userspace-corpus.sh <cfg> [--repo-root DIR] [--workspace-root DIR]
#
# Options:
#   --repo-root DIR       Override "this repo" root (default: git rev-parse --show-toplevel)
#   --workspace-root DIR  Override the directory whose sibling entries are scanned as
#                          peer repos (default: dirname of --repo-root)
#
# Honors discovery.userspace.opt_out in <cfg> (opt-out => emit nothing, exit 0).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SAFETY="$SCRIPT_DIR/discovery-safety.sh"
LEDGER_SH="$SCRIPT_DIR/trend-ledger.sh"

usage() {
  echo "usage: discovery-userspace-corpus.sh <cfg> [--repo-root DIR] [--workspace-root DIR]" >&2
  exit 2
}

CFG="${1:-}"
[ -n "$CFG" ] || usage
shift || true
[ -f "$CFG" ] || { echo "config not found: $CFG" >&2; exit 2; }

REPO_ROOT=""
WORKSPACE_ROOT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --repo-root) REPO_ROOT="$2"; shift 2 ;;
    --workspace-root) WORKSPACE_ROOT="$2"; shift 2 ;;
    --help|-h) usage ;;
    *) shift ;;
  esac
done

if [ -z "$REPO_ROOT" ]; then
  REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
fi
REPO_ROOT="$(cd "$REPO_ROOT" && pwd)"

if [ -z "$WORKSPACE_ROOT" ]; then
  WORKSPACE_ROOT="$(dirname "$REPO_ROOT")"
fi
[ -d "$WORKSPACE_ROOT" ] || { echo "workspace root not found: $WORKSPACE_ROOT" >&2; exit 0; }

# ── Read config as JSON (yq for YAML, jq fallback for JSON-only configs) ──────
cfg_to_json() {
  local cfg="$1"
  if command -v yq >/dev/null 2>&1; then
    yq -o=json '.' "$cfg" 2>/dev/null
  else
    jq -e '.' "$cfg" 2>/dev/null
  fi
}

CFG_JSON="$(cfg_to_json "$CFG")"
if [ -z "$CFG_JSON" ]; then
  echo "cannot parse config: $CFG" >&2
  exit 2
fi

OPT_OUT="$(printf '%s' "$CFG_JSON" | jq -r '.discovery.userspace.opt_out // false')"
if [ "$OPT_OUT" = "true" ]; then
  exit 0
fi

# ── Rate gate (fail-open on skip: a rate cap simply means "nothing to emit now") ──
if [ -x "$SAFETY" ]; then
  if ! "$SAFETY" --rate-ok userspace-corpus "$CFG" >/dev/null 2>&1; then
    exit 0
  fi
fi

# ── Capability marker enumeration ─────────────────────────────────────────────
# A capability marker is a namespaced name derived from the repo's skills/,
# scripts/, and top-level docs/ directories. Only the marker *name* is compared
# across repos (not full paths), so a peer repo's differently-nested layout still
# matches an equivalent capability in this repo.
list_capabilities() {
  local root="$1"
  {
    if [ -d "$root/skills" ]; then
      find "$root/skills" -mindepth 1 -maxdepth 1 -type d 2>/dev/null \
        | while IFS= read -r d; do printf 'skill:%s\n' "$(basename "$d")"; done
    fi
    if [ -d "$root/scripts" ]; then
      find "$root/scripts" -type f -name '*.sh' 2>/dev/null \
        | while IFS= read -r f; do printf 'script:%s\n' "$(basename "$f")"; done
    fi
    if [ -d "$root/docs" ]; then
      find "$root/docs" -mindepth 1 -maxdepth 1 -type f -name '*.md' 2>/dev/null \
        | while IFS= read -r f; do printf 'doc:%s\n' "$(basename "$f")"; done
    fi
  } | sort -u
}

THIS_CAPS="$(mktemp)"
list_capabilities "$REPO_ROOT" > "$THIS_CAPS"

cleanup() { rm -f "$THIS_CAPS" "${PEER_CAPS:-}"; }
trap cleanup EXIT

now_ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }

# ── Enumerate sibling peer repos (read-only) ──────────────────────────────────
for peer_dir in "$WORKSPACE_ROOT"/*/; do
  [ -d "$peer_dir" ] || continue
  peer_dir="${peer_dir%/}"
  peer_real="$(cd "$peer_dir" && pwd)"

  # Skip this repo itself and anything that isn't a git repo (not a peer project).
  [ "$peer_real" = "$REPO_ROOT" ] && continue
  [ -e "$peer_real/.git" ] || continue

  peer_name="$(basename "$peer_real")"

  PEER_CAPS="$(mktemp)"
  list_capabilities "$peer_real" > "$PEER_CAPS"

  # peer-only markers: present in peer, absent here.
  while IFS= read -r marker; do
    [ -n "$marker" ] || continue

    kind="${marker%%:*}"
    name="${marker#*:}"
    norm_key="userspace-corpus:${kind}:${name}"

    summary="peer repo '${peer_name}' has ${kind} '${name}' which this repo lacks"
    excerpt="$(printf '%s' "$summary" | "$SAFETY" --sanitize 2>/dev/null || printf '%s' "$summary")"

    existing="$("$LEDGER_SH" --show --json --source userspace-corpus 2>/dev/null \
      | jq -r --arg k "$norm_key" '.[] | select(.norm_key==$k) | .norm_key' 2>/dev/null | head -1)"

    if [ -n "$existing" ]; then
      "$LEDGER_SH" --bump-recurrence "$norm_key" >/dev/null 2>&1 || true
    else
      ts="$(now_ts)"
      record="$(jq -c -n \
        --arg source "userspace-corpus" \
        --arg kind "capability-gap" \
        --arg summary "$summary" \
        --arg norm_key "$norm_key" \
        --arg evidence_ref "workspace:${peer_name}/${kind}/${name}" \
        --arg first_seen "$ts" \
        --arg ts "$ts" \
        --arg excerpt "$excerpt" \
        '{source:$source, kind:$kind, summary:$summary, norm_key:$norm_key,
          evidence_ref:$evidence_ref, first_seen:$first_seen, recurrence:1,
          sanitized_excerpt:$excerpt, ts:$ts}')"
      "$LEDGER_SH" --append "$record" >/dev/null 2>&1 || true
    fi
  done < <(comm -23 "$PEER_CAPS" "$THIS_CAPS")

  rm -f "$PEER_CAPS"
  PEER_CAPS=""
done

exit 0
