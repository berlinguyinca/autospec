#!/usr/bin/env bash
# Validate issue #1556's Phase-4 autonomous platform source-of-truth contract.
set -eu

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
contains() { grep -q "$2" "$1" || fail "$1 missing: $2"; }

spec="docs/specs/2026-07-06-autospec-autonomous-platform-design.md"
[ -f "$spec" ] || fail "$spec missing"
contains "$spec" '^# /autospec-autonomous — Never-Idle, Never-Ask Autonomous Software Architecture Platform (design spec)'
for phrase in \
  'Never converge-stop' 'Never ask' 'Fences are async quarantine' \
  'Every improvement carries a measurable before/after signal' \
  'AUTOSPEC_VALUE_FLOOR' 'capability detection' 'No target-repo specifics'; do
  contains "$spec" "$phrase"
done

for doc in \
  docs/specs/2026-06-25-autospec-autonomous-design.md \
  docs/specs/2026-06-26-autospec-autonomous-phase2-design.md \
  docs/specs/2026-06-26-autospec-autonomous-phase3-design.md; do
  contains "$doc" 'Phase-4 supersession note (2026-07-06)'
  contains "$doc" '2026-07-06-autospec-autonomous-platform-design.md'
done

for f in \
  skills/autospec-autonomous/SKILL.md \
  skills/autospec-autonomous/codex/prompt.md \
  skills/autospec-autonomous/opencode/agent.md; do
  contains "$f" 'Phase-4 never-idle / never-ask reconciliation'
  contains "$f" 'AUTOSPEC_VALUE_FLOOR'
  contains "$f" 'idle-rescan'
  contains "$f" 'async quarantine-and-continue'
  contains "$f" 'must not emit `AskUserQuestion`'
  contains "$f" 'Every tier must emit a measured before/after or ranking signal'
  contains "$f" 'No target-repo-specific heuristic may be hardcoded'
  contains "$f" 'resource-park and notify'
  ! grep -q 'waterfall:park:all tiers dry' "$f" \
    || fail "$f still contains stale all-tiers-dry park"
done

printf 'PASS: autonomous Phase-4 design contract\n'
