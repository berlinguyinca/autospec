#!/usr/bin/env bats
# tests/bundle-static-context-reviewer.bats — TDD for --role reviewer in
# skills/autospec-shared/scripts/bundle-static-context.sh (issue #404)

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/bundle-static-context.sh"

@test "bundle-static-context.sh --role reviewer exits 0" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
}

@test "bundle-static-context.sh --role reviewer emits opening CACHE BOUNDARY marker" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | head -1 | grep -q "CACHE BOUNDARY"
}

@test "bundle-static-context.sh --role reviewer emits two CACHE BOUNDARY markers (prefix framed; per-issue content below the second)" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  count="$(printf '%s\n' "$output" | grep -c 'CACHE BOUNDARY' || true)"
  [ "$count" = "2" ]
}

@test "bundle-static-context.sh --role reviewer output contains Implementation-quality contract" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "Implementation-quality contract"
}

@test "bundle-static-context.sh --role reviewer output contains RULE_ID table" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "OUT_OF_SCOPE"
}

@test "bundle-static-context.sh --role reviewer output contains reviewer-verdict-format block" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qi "verdict\|LGTM"
}

@test "bundle-static-context.sh --role reviewer output contains lint-implementation.sh schema" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "lint-implementation.sh"
}

@test "bundle-static-context.sh --role reviewer output is idempotent" {
  out1=$("$SCRIPT" --role reviewer)
  out2=$("$SCRIPT" --role reviewer)
  [ "$out1" = "$out2" ]
}

# ── Phase 1 (prompt-cache reclaim): reviewer prefix slim + cache stabilization ──

# Helper: emit only the cached prefix (everything up to and including the closing
# CACHE BOUNDARY marker — i.e. the part the harness caches).
_reviewer_prefix() {
  "$SCRIPT" --role reviewer "$@" | awk '
    /<!-- CACHE BOUNDARY -->/ { n++; print; if (n==2) exit; next }
    { print }
  '
}

@test "reviewer prefix no longer injects the full autospec-run SKILL.md" {
  out="$("$SCRIPT" --role reviewer)"
  # The verbose injection header used by the old composition must be gone.
  if printf '%s\n' "$out" | grep -q 'SKILL.md (reviewer role)'; then
    return 1
  fi
}

@test "reviewer prefix no longer carries the monitor-loop bash" {
  prefix="$(_reviewer_prefix)"
  # A representative monitor-loop construct from autospec-run/SKILL.md.
  if printf '%s\n' "$prefix" | grep -q 'heartbeat-write.sh'; then
    return 1
  fi
}

@test "reviewer prefix sources the reviewer-contract.md rubric" {
  run "$SCRIPT" --role reviewer
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q 'Reviewer contract'
  printf '%s\n' "$output" | grep -qi 'Guardian rubric'
}

@test "reviewer prefix contains the RULE_ID table exactly once" {
  prefix="$(_reviewer_prefix)"
  count="$(printf '%s\n' "$prefix" | grep -c '^### RULE_ID table' || true)"
  [ "$count" = "1" ]
}

@test "reviewer prefix carries the full LLM-tier RULE_ID set the fused reviewer applies" {
  prefix="$(_reviewer_prefix)"
  for rid in HALLUCINATED_API DUPLICATE_CODE STRING_MATCH_DOMAIN_LOGIC \
             REPEATED_STRUCTURE_AS_CODE DOC_OUT_OF_SYNC INVENTED_CONFIG; do
    printf '%s\n' "$prefix" | grep -q "$rid" \
      || { echo "missing RULE_ID in prefix: $rid"; return 1; }
  done
}

@test "reviewer prefix carries every AGENTS.md canonical RULE_ID (no dropped rule)" {
  agents="${BATS_TEST_DIRNAME}/../AGENTS.md"
  prefix="$(_reviewer_prefix)"
  rule_ids="$(awk '
    /^### RULE_ID table/ { in_table=1; next }
    /^### / && in_table { exit }
    /^## / && in_table { exit }
    in_table && /^\| `[A-Z_]+`/ {
      line=$0; sub(/^\| `/,"",line); sub(/`.*/,"",line); print line
    }
  ' "$agents" | sort -u)"
  [ -n "$rule_ids" ]
  for rid in $rule_ids; do
    printf '%s\n' "$prefix" | grep -q "$rid" \
      || { echo "AGENTS.md RULE_ID dropped from reviewer prefix: $rid"; return 1; }
  done
}

@test "reviewer per-issue memory + diff live BELOW the cache boundary" {
  full="$("$SCRIPT" --role reviewer)"
  prefix="$(_reviewer_prefix)"
  # The saved-memory block is per-issue → must not be inside the cached prefix.
  if printf '%s\n' "$prefix" | grep -q 'Project rules (saved memory)'; then
    return 1
  fi
  # But it must still be present somewhere in the full output (below the boundary).
  printf '%s\n' "$full" | grep -q 'Project rules (saved memory)'
}

@test "reviewer prefix is BYTE-IDENTICAL across two differing-issue dispatches" {
  # The prefix is cached, so it must not vary with issue labels.
  p1="$(_reviewer_prefix --issue-labels 'skill:autospec-run,ctx:120k')"
  p2="$(_reviewer_prefix --issue-labels 'bug,ctx:32k,reasoning:deep')"
  [ "$p1" = "$p2" ]
}

@test "reviewer prefix token estimate drops >=50% vs the full SKILL.md prefix" {
  # wc-based token estimate: words ~= tokens (conservative). Compare the new slim
  # prefix against the pre-Phase-1 baseline (full autospec-run/SKILL.md prefix).
  skill_md="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
  agents="${BATS_TEST_DIRNAME}/../AGENTS.md"
  old_tokens=$(cat "$skill_md" "$agents" | wc -w | tr -d ' ')
  new_tokens=$(_reviewer_prefix | wc -w | tr -d ' ')
  # Assert at least a 50% reduction.
  [ "$new_tokens" -le $((old_tokens / 2)) ] \
    || { echo "old=$old_tokens new=$new_tokens (need <= $((old_tokens/2)))"; return 1; }
}

# ── Memory-injection cap (AUTOSPEC_MAX_MEMORY_FILES, default 6) ──────────────────

@test "memory injection is capped at AUTOSPEC_MAX_MEMORY_FILES" {
  memdir="$(mktemp -d)"
  manifest="$(mktemp)"
  # Build 10 memory files all sharing the tag 'autospec-run' so all match.
  : > "$manifest"
  for i in $(seq 1 10); do
    f="feedback_cap_$i.md"
    printf 'CAP_MARKER_%02d body\n' "$i" > "$memdir/$f"
    printf '%s:\n  tags: [autospec-run, cap]\n' "$f" >> "$manifest"
  done
  run env AUTOSPEC_MEMORY_DIR="$memdir" AUTOSPEC_MANIFEST="$manifest" \
    AUTOSPEC_MAX_MEMORY_FILES=3 \
    "$SCRIPT" --role reviewer --issue-labels 'autospec-run'
  rm -rf "$memdir" "$manifest"
  [ "$status" -eq 0 ]
  injected="$(printf '%s\n' "$output" | grep -c 'CAP_MARKER_' || true)"
  [ "$injected" -le 3 ] || { echo "injected=$injected (cap was 3)"; return 1; }
}

@test "memory cap defaults to 6 when AUTOSPEC_MAX_MEMORY_FILES unset" {
  memdir="$(mktemp -d)"
  manifest="$(mktemp)"
  : > "$manifest"
  for i in $(seq 1 10); do
    f="feedback_cap_$i.md"
    printf 'CAP_MARKER_%02d body\n' "$i" > "$memdir/$f"
    printf '%s:\n  tags: [autospec-run, cap]\n' "$f" >> "$manifest"
  done
  run env -u AUTOSPEC_MAX_MEMORY_FILES AUTOSPEC_MEMORY_DIR="$memdir" \
    AUTOSPEC_MANIFEST="$manifest" \
    "$SCRIPT" --role reviewer --issue-labels 'autospec-run'
  rm -rf "$memdir" "$manifest"
  [ "$status" -eq 0 ]
  injected="$(printf '%s\n' "$output" | grep -c 'CAP_MARKER_' || true)"
  [ "$injected" -le 6 ] || { echo "injected=$injected (default cap 6)"; return 1; }
}

@test "decomposer/classifier roles drop the duplicate RULE_ID re-extraction block" {
  # The standalone '## RULE_ID table' re-extraction header must be gone; the
  # canonical table still rides inside AGENTS.md verbatim.
  for role in decomposer classifier; do
    out="$("$SCRIPT" --role "$role")"
    if printf '%s\n' "$out" | grep -q '^## RULE_ID table'; then
      echo "$role still emits duplicate '## RULE_ID table'"
      return 1
    fi
    # Canonical RULE_IDs still reachable (via AGENTS.md verbatim).
    printf '%s\n' "$out" | grep -q 'OUT_OF_SCOPE'
  done
}
