#!/usr/bin/env bash
# scripts/refine-prompt-lens-llm.sh — LLM-driven lens dispatcher (issue #684).
#
# Per-lens LLM call. Args:
#   --lens <name>           one of repo-grounding|clarity-ac|sizing|adversarial
#   --prompt <text>         the prompt to refine (operator input or prior round)
#   --context-file <path>   file containing repo context (AGENTS.md, recent
#                           specs, recent git diffs). May be empty / omitted.
#   --llm-binary <path>     optional override; defaults to claude or codex.
#
# Calls `claude -p` or `codex exec --reasoning-effort medium`. Captures stdout.
# Validates output non-empty. Echoes refined prompt to stdout, exits 0.
# On LLM error or empty output, exits 1 (caller falls back to deterministic).
#
# Path-safety (mirrors scripts/refine-prompt.sh's handoff dispatcher rule):
# reject `claude`/`codex` resolved under /tmp/, /private/tmp/, /var/tmp/,
# /var/folders/, or $TMPDIR unless AUTOSPEC_LLM_DISPATCHER=1.

set -u

usage() {
    cat <<'EOF'
Usage: refine-prompt-lens-llm.sh --lens <name> --prompt <text> [--context-file <path>] [--llm-binary <path>]

Lenses: repo-grounding, clarity-ac, sizing, adversarial.
Exit:
  0  — refined prompt on stdout
  1  — LLM call failed / empty output / unsafe dispatcher / bad args
EOF
}

LENS=""
PROMPT_IN=""
CONTEXT_FILE=""
LLM_BINARY=""

while [ $# -gt 0 ]; do
    case "$1" in
        --lens) LENS="$2"; shift ;;
        --prompt) PROMPT_IN="$2"; shift ;;
        --context-file) CONTEXT_FILE="$2"; shift ;;
        --llm-binary) LLM_BINARY="$2"; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "refine-prompt-lens-llm: unknown arg: $1" >&2; usage >&2; exit 1 ;;
    esac
    shift
done

if [ -z "$LENS" ] || [ -z "$PROMPT_IN" ]; then
    echo "refine-prompt-lens-llm: --lens and --prompt required" >&2
    exit 1
fi

case "$LENS" in
    repo-grounding|clarity-ac|sizing|adversarial) ;;
    *) echo "refine-prompt-lens-llm: unknown lens: $LENS" >&2; exit 1 ;;
esac

# ── per-lens prompts (issue #684 contracts) ───────────────────────
lens_prompt_text() {
    case "$1" in
        repo-grounding)
            cat <<'EOF'
You are the autospec-refine "repo-grounding" lens. Read the operator's prompt
below. Read the repo context (AGENTS.md + recent specs + recent git diffs).
Rewrite the prompt to use concrete file paths, conventions, naming, and
constraints that align with this repo. Do not add scope. Do not invent file
paths. Return only the rewritten prompt.
EOF
            ;;
        clarity-ac)
            cat <<'EOF'
You are the autospec-refine "clarity-ac" lens. Read the operator's prompt.
Extract every implicit acceptance criterion. Rewrite the prompt with a
`## Acceptance criteria` section that is a checkbox list. Disambiguate
hedging language (`should probably`, `might`, `could try` → flat
statements). Return only the rewritten prompt.
EOF
            ;;
        sizing)
            cat <<'EOF'
You are the autospec-refine "sizing" lens. Read the operator's prompt. If
implementation would exceed the autospec small-LLM caps (body ≤400 words,
≤3 files per child issue, ≤30 lines of implementation outline), split into
a parent + child sequence with linked prompt fragments. Return only the
rewritten prompt(s).
EOF
            ;;
        adversarial)
            cat <<'EOF'
You are the autospec-refine "adversarial" lens. Read the operator's prompt.
Critically ask "what could still go wrong even if the obvious tests pass?"
For each high-risk answer that's actionable within the scope, add a test
requirement or scope clarification. Do not add unrelated risks. Return
only the rewritten prompt.
EOF
            ;;
    esac
}

# ── dispatcher path-safety ────────────────────────────────────────
_dispatcher_safe() {
    local resolved="$1"
    [ -n "$resolved" ] || return 1
    if [ -n "${AUTOSPEC_LLM_DISPATCHER:-}" ]; then
        return 0
    fi
    case "$resolved" in
        /tmp/*|/private/tmp/*|/var/tmp/*|/var/folders/*) return 1 ;;
    esac
    if [ -n "${TMPDIR:-}" ]; then
        case "$resolved" in
            "$TMPDIR"*|"${TMPDIR%/}"/*) return 1 ;;
        esac
    fi
    return 0
}

# ── resolve dispatcher ────────────────────────────────────────────
DISPATCHER=""
DISPATCHER_KIND=""
if [ -n "$LLM_BINARY" ]; then
    DISPATCHER="$LLM_BINARY"
    case "$(basename "$DISPATCHER")" in
        claude*) DISPATCHER_KIND="claude" ;;
        codex*)  DISPATCHER_KIND="codex" ;;
        *)       DISPATCHER_KIND="claude" ;;
    esac
elif command -v claude >/dev/null 2>&1; then
    DISPATCHER="$(command -v claude)"
    DISPATCHER_KIND="claude"
elif command -v codex >/dev/null 2>&1; then
    DISPATCHER="$(command -v codex)"
    DISPATCHER_KIND="codex"
else
    echo "refine-prompt-lens-llm: no claude/codex on PATH" >&2
    exit 1
fi

if ! _dispatcher_safe "$DISPATCHER"; then
    echo "refine-prompt-lens-llm: ERROR — refusing LLM dispatch: dispatcher in tmpdir: $DISPATCHER (set AUTOSPEC_LLM_DISPATCHER=1 to override)" >&2
    exit 1
fi

# ── assemble full instruction ─────────────────────────────────────
INSTRUCTION="$(lens_prompt_text "$LENS")"

CONTEXT_TEXT=""
if [ -n "$CONTEXT_FILE" ] && [ -f "$CONTEXT_FILE" ]; then
    CONTEXT_TEXT="$(cat "$CONTEXT_FILE")"
fi

FULL_INPUT=""
FULL_INPUT+="$INSTRUCTION"$'\n\n'
if [ -n "$CONTEXT_TEXT" ]; then
    FULL_INPUT+="## Repo context"$'\n'"$CONTEXT_TEXT"$'\n\n'
fi
FULL_INPUT+="## Operator prompt"$'\n'"$PROMPT_IN"$'\n'

# ── call dispatcher ───────────────────────────────────────────────
OUTPUT=""
RC=0
case "$DISPATCHER_KIND" in
    claude)
        OUTPUT="$("$DISPATCHER" -p "$FULL_INPUT" 2>/dev/null)" || RC=$?
        ;;
    codex)
        OUTPUT="$("$DISPATCHER" exec --reasoning-effort medium "$FULL_INPUT" 2>/dev/null)" || RC=$?
        ;;
esac

if [ "$RC" -ne 0 ]; then
    echo "refine-prompt-lens-llm: dispatcher rc=$RC" >&2
    exit 1
fi

# Validate non-empty (strip whitespace).
TRIMMED="$(printf '%s' "$OUTPUT" | tr -d '[:space:]')"
if [ -z "$TRIMMED" ]; then
    echo "refine-prompt-lens-llm: empty LLM output" >&2
    exit 1
fi

printf '%s' "$OUTPUT"
exit 0
