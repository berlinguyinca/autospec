#!/usr/bin/env bash
# assert.sh — signal-property checks for a determinized-step output.
#
# Invoked by scripts/quality-differential.sh as:
#     bash assert.sh <det-output-file> <llm-golden-file>
#
# These are SIGNAL-PROPERTY checks, NOT string equality. They verify that the
# deterministic output carries the same useful signal a human/LLM refinement
# would, without demanding byte identity:
#
#   1. Content-token coverage: the deterministic output references >=2 distinct
#      "content tokens" (lowercased words >=4 chars, excluding stopwords) drawn
#      from the llm-golden reference. Proves the output is ABOUT the right
#      thing, not generic.
#   2. No canned boilerplate: the output contains none of the known canned
#      boilerplate markers that a template-only path emits regardless of input
#      (e.g. the literal "What happens on empty input?"). Their presence is the
#      tell-tale of a deterministic path that pads instead of reasoning.
#
# Exit 0 = pass (signal present, no boilerplate); non-zero = fail.

set -eu

DET_OUT="${1:?det-output file required}"
GOLDEN="${2:?llm-golden file required}"

# Canned boilerplate markers emitted verbatim by the deterministic refine
# lenses regardless of the input prompt. Any of these in the output is a fail.
CANNED_MARKERS=(
    "What happens on empty input?"
    "What happens on malformed input?"
    "What forbidden paths could the change accidentally touch?"
)

fail() {
    printf 'assert: FAIL — %s\n' "$*" >&2
    exit 1
}

# ── Check 2 first (cheap, decisive): no canned boilerplate markers. ──
for marker in "${CANNED_MARKERS[@]}"; do
    if grep -qF "$marker" "$DET_OUT"; then
        fail "canned boilerplate marker present: \"$marker\""
    fi
done

# ── Check 1: content-token coverage vs the llm-golden. ──
# Stopwords kept short and generic so the check stays signal-focused.
STOPWORDS=" the and that this with from your into when then should must will have been your shall does each onto over under more most some such only also they them their there here what which while where -- "

# Build the set of content tokens from the golden (lowercased, alnum, >=4 chars,
# de-duplicated, stopwords removed).
golden_tokens="$(tr '[:upper:]' '[:lower:]' < "$GOLDEN" \
    | tr -cs 'a-z0-9' '\n' \
    | awk 'length($0) >= 4' \
    | sort -u)"

matches=0
while IFS= read -r tok; do
    if [ -z "$tok" ]; then
        continue
    fi
    case "$STOPWORDS" in
        *" $tok "*) continue ;;
    esac
    if grep -qiwF "$tok" "$DET_OUT"; then
        matches=$((matches + 1))
    fi
    if [ "$matches" -ge 2 ]; then
        break
    fi
done <<EOF
$golden_tokens
EOF

if [ "$matches" -lt 2 ]; then
    fail "content-token coverage below 2 (got $matches) — output not grounded in the input/golden"
fi

printf 'assert: PASS — %d+ content tokens, no canned boilerplate\n' "$matches" >&2
exit 0
