#!/usr/bin/env bash
# discovery-source-proposal.sh — admits LLM-proposed candidate sources to the
# discovery engine's probation tier. The proposal JSON is untrusted DATA — it
# only ever *proposes* names; it never authorizes an action on its own. Every
# survivor still passes the discovery-blocklist.sh forbidden-class gate, and
# admissions are capped at discovery.max_new_sources_per_round. A new source
# NEVER goes straight to full weight: it lands in
# .autospec/trends/probation.txt (one name per line, no weight encoding —
# discovery-blocklist.sh --allowed reads probation as a plain allowlist next
# to seed_sources), and #1652-class harvesters pick it up next round; the
# explore outcome ledger is the only thing that ever promotes a probation
# source to full weight.
#
# Usage:
#   discovery-source-proposal.sh <proposal.json> <cfg>
#
# Proposal JSON shape (fixture-driven in tests, no live LLM/network call here):
#   {"sources": ["name1", "name2", ...]}
#
# Fail-closed 5-attempt adaptive-retry parse: each failed attempt appends a
# directive (fed back to the LLM caller in the real pipeline); unparseable
# proposal JSON after MAX_ATTEMPTS refuses to admit anything and exits
# non-zero. Mirrors growth-candidate-verify.sh's fail-closed pattern.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BLOCKLIST="$SCRIPT_DIR/discovery-blocklist.sh"

usage() {
  echo "usage: discovery-source-proposal.sh <proposal.json> <cfg>" >&2
  exit 2
}

PROPOSAL="${1:-}"; [ -n "$PROPOSAL" ] || usage
CFG="${2:-}"; [ -n "$CFG" ] || usage

PROBATION_FILE="${AUTOSPEC_DISCOVERY_PROBATION:-.autospec/trends/probation.txt}"
MAX_ATTEMPTS=5

cfg_to_json() {
  local cfg="$1"
  if command -v yq >/dev/null 2>&1; then
    yq -o=json '.' "$cfg" 2>/dev/null
  else
    jq -e '.' "$cfg" 2>/dev/null
  fi
}

# ---------------------------------------------------------------------------
# parse: 5-attempt adaptive retry. Each attempt validates the proposal shape
# ({"sources": [string, ...]}); a failure appends a human-readable directive
# describing what was wrong. This is the feedback that a real caller would
# hand back to the LLM before regenerating the proposal; the script itself
# only ever re-reads the same file path, so a persistently malformed proposal
# exhausts all attempts and refutes (non-zero exit, nothing admitted).
# ---------------------------------------------------------------------------
directives=""
parsed=0
attempt=0
while [ "$attempt" -lt "$MAX_ATTEMPTS" ]; do
  attempt=$((attempt + 1))
  if [ ! -f "$PROPOSAL" ]; then
    directives="${directives}attempt ${attempt}: proposal file not found: ${PROPOSAL}
"
    continue
  fi
  if jq -e '(.sources? // empty) as $s
            | ($s | type) == "array"
            and (($s | length) == 0 or ($s | all(type == "string")))' \
      "$PROPOSAL" >/dev/null 2>&1; then
    parsed=1
    break
  fi
  directives="${directives}attempt ${attempt}: malformed proposal JSON, expected {\"sources\":[string,...]}
"
done

if [ "$parsed" -ne 1 ]; then
  printf '%s' "$directives" >&2
  echo "discovery-source-proposal: proposal unparseable after ${MAX_ATTEMPTS} attempts, refusing to admit any source" >&2
  exit 1
fi

names="$(jq -r '.sources[]' "$PROPOSAL" 2>/dev/null || true)"

# ---------------------------------------------------------------------------
# forbidden-class gate: never admit a proposed name that is itself one of the
# discovery engine's forbidden source classes (builtin ∪ config extensions).
# Blocklist wins over any LLM proposal.
# ---------------------------------------------------------------------------
forbidden="$("$BLOCKLIST" --effective "$CFG")"

# ---------------------------------------------------------------------------
# cap: discovery.max_new_sources_per_round (default 0 == admit nothing).
# ---------------------------------------------------------------------------
cfg_json="$(cfg_to_json "$CFG")"
cap=0
if [ -n "$cfg_json" ]; then
  cap="$(printf '%s' "$cfg_json" | jq -r '.discovery.max_new_sources_per_round // 0' 2>/dev/null || echo 0)"
fi
case "$cap" in
  ''|*[!0-9]*) cap=0 ;;
esac

mkdir -p "$(dirname "$PROBATION_FILE")"
touch "$PROBATION_FILE"
existing="$(cat "$PROBATION_FILE")"

admitted=0
admitted_names=""
while IFS= read -r name; do
  [ -n "$name" ] || continue
  if [ "$admitted" -ge "$cap" ]; then
    break
  fi
  if printf '%s\n' "$forbidden" | grep -qFx "$name"; then
    echo "discovery-source-proposal: rejected forbidden-class source: $name" >&2
    continue
  fi
  if printf '%s\n' "$existing" | grep -qFx "$name"; then
    continue
  fi
  if printf '%s\n' "$admitted_names" | grep -qFx "$name"; then
    continue
  fi
  echo "$name" >> "$PROBATION_FILE"
  admitted_names="${admitted_names}${name}
"
  admitted=$((admitted + 1))
done <<EOF
$names
EOF

if [ -n "$admitted_names" ]; then
  printf '%s' "$admitted_names" | awk 'NF'
fi
exit 0
