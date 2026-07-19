#!/usr/bin/env bash
# autonomous-priority-match.sh — F4 Tier-C priority matcher.
#
# Given a proposal JSON object on stdin, the operator priorities file, and the
# effective persona, decides whether the proposal advances a stated/inferred
# priority.  If so, emits the proposal with "priority":true; otherwise emits
# the proposal unchanged (always exit 0 — fail-open).
#
# ABSOLUTE INVARIANT — no second ranker, no parallel filing:
#   This script sets the "priority" field to true and emits the modified JSON.
#   It NEVER files issues, NEVER calls gh, NEVER re-ranks.  The existing
#   clamped boost in explore-research-cycle.sh (AUTOSPEC_PRIORITY_BOOST,
#   floor 1.0 + score cap at max_non_boosted × multiplier) is the ONLY
#   mechanism that acts on the flag.  A wrong match can reorder but cannot
#   starve or drop a candidate because the boost is bounded.
#
# Usage:
#   printf '%s' '{"title":"...","score":0.4,...}' | bash autonomous-priority-match.sh
#
# Env:
#   AUTOSPEC_PRIORITIES_FILE    Priorities file
#                               (default: ~/.autospec/autonomous-priorities.md)
#   AUTOSPEC_PERSONA_FILE       Effective persona file
#                               (default: ~/.autospec/operator-persona.md)
#   AUTOSPEC_PRIORITY_MATCH_CMD Override LLM seam (subprocess).
#                               Receives combined context on stdin.
#                               Must output "match" or "no-match" on stdout.
#                               When unset: real Tier-C claude call (or
#                               conservative "no-match" if claude is absent).
#
# Output: proposal JSON on stdout (with "priority":true when matched).
# Exit:   always 0 (fail-open — any error emits proposal unchanged).
#
# Engineering rules (AGENTS.md):
#   set -eu; if/then/fi (no one-sided &&); no RETURN traps (inline cleanup);
#   jq capture()/== not interpolated test() — this script uses python3 for
#   JSON manipulation; no RETURN traps.

set -eu

PRIORITIES_FILE="${AUTOSPEC_PRIORITIES_FILE:-${HOME}/.autospec/autonomous-priorities.md}"
PERSONA_FILE="${AUTOSPEC_PERSONA_FILE:-${HOME}/.autospec/operator-persona.md}"

# ---------------------------------------------------------------------------
# Read proposal from stdin into a temp file so we can emit it on any exit.
# Using a real temp file (not process substitution) for bash 3.2 compat.
# ---------------------------------------------------------------------------
_tmp_dir="$(mktemp -d -t priority-match.XXXXXX)"
_proposal_file="$_tmp_dir/proposal.json"
_context_file="$_tmp_dir/context.txt"

# Inline cleanup on every exit path (no RETURN trap — they leak under set -u).
_cleanup_priority_match() {
    rm -rf "$_tmp_dir" 2>/dev/null || true
}

# Read stdin.
if ! cat > "$_proposal_file"; then
    printf '[priority-match] WARN: failed to read proposal from stdin\n' >&2
    _cleanup_priority_match
    exit 0
fi

# Empty stdin → emit nothing and return.
if [ ! -s "$_proposal_file" ]; then
    _cleanup_priority_match
    exit 0
fi

# ---------------------------------------------------------------------------
# Validate that stdin is valid JSON.  Fail-open: bad JSON emitted unchanged.
# ---------------------------------------------------------------------------
if ! python3 -c 'import json,sys; json.load(sys.stdin)' < "$_proposal_file" 2>/dev/null; then
    printf '[priority-match] WARN: proposal is not valid JSON — emitting unchanged\n' >&2
    cat "$_proposal_file"
    _cleanup_priority_match
    exit 0
fi

# ---------------------------------------------------------------------------
# Read priorities file.  If absent or empty, emit unchanged — no priorities
# means the matcher cannot match anything (fail-open, not fail-closed).
# ---------------------------------------------------------------------------
_priorities_content=""
if [ -f "$PRIORITIES_FILE" ] && [ -s "$PRIORITIES_FILE" ]; then
    _priorities_content="$(cat "$PRIORITIES_FILE")"
fi

if [ -z "$_priorities_content" ]; then
    cat "$_proposal_file"
    _cleanup_priority_match
    exit 0
fi

# ---------------------------------------------------------------------------
# Read persona file (fail-open if missing).
# ---------------------------------------------------------------------------
_persona_content=""
if [ -f "$PERSONA_FILE" ] && [ -s "$PERSONA_FILE" ]; then
    _persona_content="$(cat "$PERSONA_FILE")"
fi

# ---------------------------------------------------------------------------
# Build the combined context for the Tier-C LLM seam.
# ---------------------------------------------------------------------------
{
    printf 'OPERATOR PRIORITIES:\n%s\n\n' "$_priorities_content"
    printf 'OPERATOR PERSONA:\n%s\n\n' "${_persona_content:-[no persona available]}"
    printf 'PROPOSAL (JSON):\n%s\n' "$(cat "$_proposal_file")"
} > "$_context_file"

# ---------------------------------------------------------------------------
# priority_match_llm — Tier-C LLM seam.
# Reads context on stdin; outputs "match" or "no-match".
# ---------------------------------------------------------------------------
priority_match_llm() {
    if [ -n "${AUTOSPEC_PRIORITY_MATCH_CMD:-}" ]; then
        # shellcheck disable=SC2086
        eval "${AUTOSPEC_PRIORITY_MATCH_CMD}"
        return $?
    fi
    # Default real seam: call claude with the combined context.
    local _ctx
    _ctx="$(cat)"
    local _system_prompt
    _system_prompt="You are a priority matcher for the autospec conductor. Given the operator's stated priorities, persona, and a proposal JSON object, decide whether the proposal clearly advances one of the stated or inferred priorities. Output ONLY the word 'match' or the word 'no-match'. No explanation, no punctuation."
    if command -v claude >/dev/null 2>&1; then
        printf '%s\n\n%s' "$_system_prompt" "$_ctx" | claude --print 2>/dev/null || printf 'no-match'
    else
        # No claude binary — conservative no-match (fail-open: do not spuriously boost).
        printf 'no-match'
    fi
}

# ---------------------------------------------------------------------------
# Invoke the seam.
# ---------------------------------------------------------------------------
_decision=""
_decision="$(priority_match_llm < "$_context_file" 2>/dev/null || true)"
# Normalise: lowercase, strip whitespace.
_decision="$(printf '%s' "$_decision" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')"

# ---------------------------------------------------------------------------
# Emit the proposal, conditionally setting priority:true.
# Fail-open: if python3 is absent or errors, emit unchanged.
# ---------------------------------------------------------------------------
if [ "$_decision" = "match" ]; then
    _result="$(python3 -c '
import json, sys
p = json.load(sys.stdin)
p["priority"] = True
print(json.dumps(p))
' < "$_proposal_file" 2>/dev/null || cat "$_proposal_file")"
    printf '%s\n' "$_result"
else
    cat "$_proposal_file"
fi

_cleanup_priority_match
exit 0
