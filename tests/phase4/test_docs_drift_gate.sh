#!/usr/bin/env bash
# tests/phase4/test_docs_drift_gate.sh — Verifies Phase 4 docs drift gate wiring (issue #369).
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

# ── autospec-define SKILL.md: Init mode section ──────────────────────────────

define_skill="$SCRIPT_DIR/skills/autospec-define/SKILL.md"
if [ ! -f "$define_skill" ]; then
    echo "FAIL: autospec-define/SKILL.md missing"
    exit 1
fi

if ! grep -qF "## Init mode" "$define_skill"; then
    echo "FAIL: autospec-define/SKILL.md missing '## Init mode' section"
    exit 1
fi

if ! grep -q "gen-docs-from-spec\|auto-docs\|doc-PR" "$define_skill"; then
    echo "FAIL: autospec-define/SKILL.md missing auto-docs step documentation"
    exit 1
fi

# ── autospec-run SKILL.md: Docs drift gate section ───────────────────────────

run_skill="$SCRIPT_DIR/skills/autospec-run/SKILL.md"
if ! grep -qF "## Docs drift gate" "$run_skill"; then
    echo "FAIL: autospec-run/SKILL.md missing '## Docs drift gate' section"
    exit 1
fi

if ! grep -q "check-doc-drift.sh" "$run_skill"; then
    echo "FAIL: autospec-run/SKILL.md missing check-doc-drift.sh reference"
    exit 1
fi

# ── phase4-implementer.md: drift gate call ───────────────────────────────────

phase4_prompt="$SCRIPT_DIR/skills/autospec-run/prompts/phase4-implementer.md"
if ! grep -q "check-doc-drift.sh" "$phase4_prompt"; then
    echo "FAIL: phase4-implementer.md missing check-doc-drift.sh reference"
    exit 1
fi

echo "OK: Phase 4 docs drift gate wiring verified"
