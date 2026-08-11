#!/usr/bin/env bash
# Verifies the Phase 4 implementer prompt has all required sections, treats
# independent review as a blocking gate, and does not delegate to Skill calls.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
prompt="$SCRIPT_DIR/skills/autospec-run/prompts/phase4-implementer.md"

if [ ! -f "$prompt" ]; then
    echo "FAIL: prompt file missing at $prompt"
    exit 1
fi

for section in "## Expand" "## Implement" "## Finalize" "## Independent review (blocking)" "## Evaluate findings" "## Lock-step compliance"; do
    if ! grep -qF "$section" "$prompt"; then
        echo "FAIL: required section missing: $section"
        exit 1
    fi
done

# Missing foreground review must fail closed and return the issue to the queue.
for required in "independent-review-adapter.sh" 'Exit `75`' "never skip review" "do not merge the PR"; do
    if ! grep -qiF "$required" "$prompt"; then
        echo "FAIL: independent-review fail-closed contract missing: $required"
        exit 1
    fi
done

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

# UI/browser validation must use the shared three-state vocabulary and require
# remediation issues when harness metadata, not the app, prevents real browser
# verification.
for state in "browser-verified" "fallback-smoke-only" "not-run"; do
    if ! grep -qF "$state" "$prompt"; then
        echo "FAIL: browser verification state missing from prompt: $state"
        exit 1
    fi
done

if ! grep -qiE 'remediation issue.*browser|browser.*remediation issue' "$prompt"; then
    echo "FAIL: browser verification harness skip remediation issue is not documented"
    exit 1
fi

if ! grep -qiE 'redact|saniti[sz]e' "$prompt"; then
    echo "FAIL: browser verification error capture must require redaction before GitHub publication"
    exit 1
fi

if ! grep -qF 'gh pr view <PR> --json body' "$prompt" || ! grep -qF 'browser_state_count' "$prompt"; then
    echo "FAIL: browser verification merge gate must include deterministic PR-body validation"
    exit 1
fi

# UI cleanup/refactor work must audit child chrome before edits so implementers
# remove nested layout artifacts instead of wrapping them in another shell.
for required in \
    "UI cohesion audit" \
    "cards-in-cards" \
    "desktop and mobile screenshots"
do
    if ! grep -qF "$required" "$prompt"; then
        echo "FAIL: UI cohesion audit prompt text missing: $required"
        exit 1
    fi
done

echo "PASS"
