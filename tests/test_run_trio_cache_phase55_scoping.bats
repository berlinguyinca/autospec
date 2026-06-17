#!/usr/bin/env bats
# prompt-cache-reclaim Phase 2 children C + E (spec
# docs/specs/2026-06-16-prompt-cache-reclaim-design.md §"Phase 2 (deferred)").
#
# C — v2-flow cache wiring: the autospec:v2-flow Phase 4 dispatch must prepend
#     the gen-implementer-prompt.sh cached prefix (the <!-- CACHE BOUNDARY -->
#     static block, cache_control: ephemeral) ahead of the phase4-implementer.md
#     body, so the default v2 path realizes the D3 cached prefix instead of
#     re-reading the static context uncached. Documented in the run trio + the
#     phase4-implementer.md body.
# E — Phase 5.5 round-≥2 scoping: the broad-review --since window narrows to the
#     newly-filed gap PRs on round ≥2; round 1 stays the full batch window.

RUN_TRIO=(
  skills/autospec-run/SKILL.md
  skills/autospec-run/opencode/agent.md
  skills/autospec-run/codex/prompt.md
)

# ---- C: v2-flow cache wiring (run trio prose) ---------------------------------

@test "C: v2-flow dispatch wires the gen-implementer-prompt.sh cached prefix (run trio)" {
  for f in "${RUN_TRIO[@]}"; do
    # The v2-flow selection branch must reference gen-implementer-prompt.sh as
    # the cached-prefix assembler for the phase4-implementer.md body.
    grep -q 'gen-implementer-prompt.sh' "$f"
    grep -q 'autospec:v2-flow' "$f"
  done
}

@test "C: v2-flow prefix carries the cache boundary + ephemeral cache_control (run trio)" {
  for f in "${RUN_TRIO[@]}"; do
    grep -q 'CACHE BOUNDARY' "$f"
    grep -q 'cache_control' "$f"
    grep -qi 'ephemeral' "$f"
  done
}

@test "C: v2-flow no longer dispatches phase4-implementer.md verbatim/uncached (run trio)" {
  for f in "${RUN_TRIO[@]}"; do
    # The old wording dispatched the prompt body "verbatim" with no cached
    # prefix. After wiring, the body is the cached-prefix dynamic body, not a
    # verbatim-and-only prompt.
    ! grep -q 'Use it verbatim as the subagent prompt body' "$f"
  done
}

# ---- C: phase4-implementer.md body documents the prepended cached prefix ------

@test "C: phase4-implementer.md documents the prepended cached prefix" {
  f=skills/autospec-run/prompts/phase4-implementer.md
  grep -q 'gen-implementer-prompt.sh' "$f"
  grep -q 'CACHE BOUNDARY' "$f"
  grep -qi 'cache_control' "$f"
}

# ---- E: Phase 5.5 round-≥2 --since scoping (run trio) -------------------------

@test "E: Phase 5.5 round-1 broad review keeps the full batch window (run trio)" {
  for f in "${RUN_TRIO[@]}"; do
    # round 1 still uses BATCH_START_DATE for the full-batch broad pass.
    grep -q 'BATCH_START_DATE' "$f"
  done
}

@test "E: Phase 5.5 round-≥2 scopes --since to the newly-filed gap PRs (run trio)" {
  for f in "${RUN_TRIO[@]}"; do
    # On round >=2 the --since window narrows to the gap PRs from the prior
    # round rather than re-scanning the whole batch window.
    grep -qi 'round .*2' "$f"
    grep -qi 'newly-filed gap' "$f"
  done
}
