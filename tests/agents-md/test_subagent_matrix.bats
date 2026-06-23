#!/usr/bin/env bats
#
# Issue #729: AGENTS.md ships a canonical Subagent-vs-inline decision matrix
# (9 work-shape rows). Every autospec skill trio (SKILL.md + codex/prompt.md +
# opencode/agent.md) carries a Subagent-dispatch-policy row in its Required-
# capabilities table pointing back to the matrix.

setup() {
  REPO_ROOT="$(git rev-parse --show-toplevel)"
  cd "$REPO_ROOT"
}

@test "AGENTS.md declares the Subagent vs inline decision matrix section" {
  grep -q '^## Subagent vs inline decision matrix$' AGENTS.md
}

@test "AGENTS.md matrix contains all 9 work-shape rows (exact text)" {
  for anchor in \
    'Read-only exploration across many files' \
    '2+ independent tasks with disjoint scopes' \
    'Single-purpose long-running work' \
    'Risky / quarantine-worthy work' \
    'Orchestration / decision-making' \
    'One-shot tool calls' \
    'Short edits (1-3 file modifications)' \
    'Routing / control flow' \
    'Multi-area fan-out'
  do
    grep -q "$anchor" AGENTS.md \
      || { echo "missing matrix row: $anchor"; return 1; }
  done
}

@test "missing-row variant fails the validator" {
  tmp="$(mktemp -d)"
  cp -r AGENTS.md scripts "$tmp/"
  mkdir -p "$tmp/skills"
  # Strip one canonical row to simulate drift.
  grep -v 'Multi-area fan-out' AGENTS.md > "$tmp/AGENTS.md"
  pushd "$tmp" >/dev/null
  run bash -c '
    set +e
    source scripts/validate.sh 2>/dev/null
    check_agents_md_subagent_matrix
  '
  popd >/dev/null
  rm -rf "$tmp"
  [ "$status" -ne 0 ]
}

@test "every autospec skill trio file references the matrix" {
  fail_count=0
  for d in skills/autospec*/; do
    [ -d "$d" ] || continue
    for f in "${d}SKILL.md" "${d}codex/prompt.md" "${d}opencode/agent.md"; do
      [ -f "$f" ] || continue
      grep -q '^## Required capabilities & harness adapter' "$f" || continue
      # Accept EITHER the literal dispatch row OR the harness-adapter-core block
      # marker (the marker is single-sourced and expands to that exact row at
      # install time). Mirrors scripts/validate.sh check_agents_md_subagent_matrix
      # so the raw-grep test and the validator agree on what satisfies the gate.
      if grep -q -F '<!-- autospec-block:harness-adapter-core -->' "$f"; then
        continue
      fi
      if ! grep -q '^| Subagent dispatch policy' "$f"; then
        echo "missing dispatch-policy row: $f"
        fail_count=$((fail_count + 1))
      elif ! grep -q 'per AGENTS.md decision matrix' "$f"; then
        echo "row missing matrix reference: $f"
        fail_count=$((fail_count + 1))
      fi
    done
  done
  [ "$fail_count" -eq 0 ]
}

@test "lockstep: SKILL.md and codex/prompt.md dispatch-policy rows byte-identical" {
  mismatch=0
  for d in skills/autospec*/; do
    skill="${d}SKILL.md"
    codex="${d}codex/prompt.md"
    [ -f "$skill" ] && [ -f "$codex" ] || continue
    grep -q '^## Required capabilities & harness adapter' "$skill" || continue
    a="$(grep '^| Subagent dispatch policy' "$skill" || true)"
    b="$(grep '^| Subagent dispatch policy' "$codex" || true)"
    if [ "$a" != "$b" ]; then
      echo "lockstep drift in $d (SKILL.md vs codex)"
      mismatch=$((mismatch + 1))
    fi
  done
  [ "$mismatch" -eq 0 ]
}

@test "lockstep: SKILL.md and opencode/agent.md dispatch-policy rows byte-identical" {
  mismatch=0
  for d in skills/autospec*/; do
    skill="${d}SKILL.md"
    oc="${d}opencode/agent.md"
    [ -f "$skill" ] && [ -f "$oc" ] || continue
    grep -q '^## Required capabilities & harness adapter' "$skill" || continue
    a="$(grep '^| Subagent dispatch policy' "$skill" || true)"
    b="$(grep '^| Subagent dispatch policy' "$oc" || true)"
    if [ "$a" != "$b" ]; then
      echo "lockstep drift in $d (SKILL.md vs opencode)"
      mismatch=$((mismatch + 1))
    fi
  done
  [ "$mismatch" -eq 0 ]
}
