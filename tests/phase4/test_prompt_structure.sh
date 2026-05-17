#!/usr/bin/env bash
# Verifies the Phase 4 implementer prompt has all required sections, documents
# the codex-absent graceful-skip path, and does not delegate to Skill calls.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
prompt="$SCRIPT_DIR/skills/autospec-run/prompts/phase4-implementer.md"

if [ ! -f "$prompt" ]; then
    echo "FAIL: prompt file missing at $prompt"
    exit 1
fi

for section in "## Expand" "## Implement" "## Finalize" "## Peer-review" "## Evaluate findings" "## Lock-step compliance"; do
    if ! grep -qF "$section" "$prompt"; then
        echo "FAIL: required section missing: $section"
        exit 1
    fi
done

# Must reference graceful Codex skip.
if ! grep -qi "codex not on PATH\|codex.*skip\|graceful" "$prompt"; then
    echo "FAIL: graceful Codex skip not documented"
    exit 1
fi

# Must NOT invoke Skill tool inside (the prompt is self-contained by design).
if grep -qE 'Skill tool|invoke the .* skill|/turboplan|/draft-spec|/peer-review' "$prompt"; then
    if ! grep -qE 'Do not invoke any Skill tool' "$prompt"; then
        echo "FAIL: prompt references Skill invocations (must be self-contained)"
        exit 1
    fi
fi

# Must reference the v2-flow label that gates this path.
if ! grep -qF "autospec:v2-flow" "$prompt"; then
    echo "FAIL: prompt does not reference the autospec:v2-flow label"
    exit 1
fi

echo "PASS"
