#!/usr/bin/env bats
# tests/reviewer/test_reuse_lens.bats — TDD for --reuse-flags in gen-reviewer-prompt.sh
# Issue #1440: reuse reviewer lens, block only with --reuse-flags present

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
BIN="$REPO_ROOT/scripts/gen-reviewer-prompt.sh"
FIX="$REPO_ROOT/tests/fixtures/gen-reviewer-prompt"

# ---------------------------------------------------------------------------
# Helper: baseline output (no --reuse-flags)
# ---------------------------------------------------------------------------
_baseline_output() {
  bash "$BIN" --pr-diff "$FIX/pr-303.diff" --prev-findings "$FIX/findings-empty.json"
}

# ---------------------------------------------------------------------------
# Case 1: byte-identical baseline — absent --reuse-flags changes nothing
# ---------------------------------------------------------------------------
@test "reuse-lens absent: output is byte-identical to baseline" {
  out_base=$(_baseline_output)
  out_no_flag=$(bash "$BIN" --pr-diff "$FIX/pr-303.diff" --prev-findings "$FIX/findings-empty.json")
  [ "$out_base" = "$out_no_flag" ]
}

# ---------------------------------------------------------------------------
# Case 2: absent flag — reuse block must NOT appear
# ---------------------------------------------------------------------------
@test "reuse-lens absent: reuse block does not appear in output" {
  run bash "$BIN" --pr-diff "$FIX/pr-303.diff" --prev-findings "$FIX/findings-empty.json"
  [ "$status" -eq 0 ]
  # The reuse section header must be absent
  printf '%s\n' "$output" | grep -qF 'Build-vs-buy / reuse interrogation' \
    && { echo "FAIL: reuse block appeared without --reuse-flags"; return 1; } \
    || true
}

# ---------------------------------------------------------------------------
# Case 3: empty flags file — reuse block must NOT appear
# ---------------------------------------------------------------------------
@test "reuse-lens empty file: reuse block does not appear in output" {
  empty_flags="$(mktemp)"
  # ensure file is truly empty
  : > "$empty_flags"
  run bash "$BIN" --pr-diff "$FIX/pr-303.diff" --prev-findings "$FIX/findings-empty.json" \
    --reuse-flags "$empty_flags"
  rm -f "$empty_flags"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qF 'Build-vs-buy / reuse interrogation' \
    && { echo "FAIL: reuse block appeared with empty flags file"; return 1; } \
    || true
}

# ---------------------------------------------------------------------------
# Case 4: with --reuse-flags, reuse block appears in dynamic suffix
# ---------------------------------------------------------------------------
@test "reuse-lens present: reuse block appears in output" {
  flags_file="$(mktemp)"
  printf 'REINVENT_REPO_UTIL:scripts/example.sh:42: function parse_json duplicates scripts/lib/json-helpers.sh:parse_json\n' > "$flags_file"
  run bash "$BIN" --pr-diff "$FIX/pr-303.diff" --prev-findings "$FIX/findings-empty.json" \
    --reuse-flags "$flags_file"
  rm -f "$flags_file"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qF 'Build-vs-buy / reuse interrogation'
}

# ---------------------------------------------------------------------------
# Case 5: reuse block names rg/registry search instruction
# ---------------------------------------------------------------------------
@test "reuse-lens present: block instructs rg/registry search" {
  flags_file="$(mktemp)"
  printf 'NEW_DEP_UNJUSTIFIED:package.json:5: lodash added without why: justification\n' > "$flags_file"
  run bash "$BIN" --pr-diff "$FIX/pr-303.diff" \
    --reuse-flags "$flags_file"
  rm -f "$flags_file"
  [ "$status" -eq 0 ]
  # Must reference rg search and registry check
  printf '%s\n' "$output" | grep -qE '\brg\b'
  printf '%s\n' "$output" | grep -qE 'registr|package'
}

# ---------------------------------------------------------------------------
# Case 6: reuse block lands in dynamic suffix, NOT in cached prefix
# The cached prefix ends at the second CACHE BOUNDARY marker.
# The reuse block must appear after that boundary.
# ---------------------------------------------------------------------------
@test "reuse-lens present: block is in dynamic suffix, not cached prefix" {
  flags_file="$(mktemp)"
  printf 'REINVENT_REPO_UTIL:scripts/foo.sh:10: new_helper duplicates scripts/lib/util.sh:new_helper\n' > "$flags_file"
  run bash "$BIN" --pr-diff "$FIX/pr-303.diff" \
    --reuse-flags "$flags_file"
  rm -f "$flags_file"
  [ "$status" -eq 0 ]

  # Write output to a real temp file so we can use awk safely on bash 3.2+
  out_file="$(mktemp)"
  printf '%s\n' "$output" > "$out_file"

  # Extract line number of the 2nd CACHE BOUNDARY (end of cached prefix)
  boundary_line=$(grep -n 'CACHE BOUNDARY' "$out_file" | awk -F: 'NR==2{print $1}')

  # Extract line number of the reuse block header
  reuse_line=$(grep -n 'Build-vs-buy / reuse interrogation' "$out_file" | awk -F: 'NR==1{print $1}')

  rm -f "$out_file"

  # reuse_line must exist and be after boundary_line
  [ -n "$reuse_line" ]
  [ "$reuse_line" -gt "${boundary_line:-0}" ]
}

# ---------------------------------------------------------------------------
# Case 7: reuse block contains the flagged item text
# ---------------------------------------------------------------------------
@test "reuse-lens present: flagged item text appears in reuse block output" {
  flags_file="$(mktemp)"
  printf 'NEW_ABSTRACTION_SINGLE_CALLER:scripts/my-manager.sh:1: new abstraction with single call site\n' > "$flags_file"
  run bash "$BIN" --pr-diff "$FIX/pr-303.diff" \
    --reuse-flags "$flags_file"
  rm -f "$flags_file"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qF 'NEW_ABSTRACTION_SINGLE_CALLER'
}

# ---------------------------------------------------------------------------
# Case 8: --reuse-flags with nonexistent file exits non-zero
# ---------------------------------------------------------------------------
@test "reuse-lens nonexistent file: exits non-zero" {
  run bash "$BIN" --pr-diff "$FIX/pr-303.diff" \
    --reuse-flags /tmp/does-not-exist-reuse-flags-xyz.txt
  [ "$status" -ne 0 ]
}

# ---------------------------------------------------------------------------
# Case 9: BLOCK/ADVISE/PASS verdict instructions present in reuse block
# ---------------------------------------------------------------------------
@test "reuse-lens present: BLOCK/ADVISE/PASS verdict format is instructed" {
  flags_file="$(mktemp)"
  printf 'REINVENT_REPO_UTIL:scripts/bar.sh:7: foo_parse duplicates scripts/lib/parse.sh:foo_parse\n' > "$flags_file"
  run bash "$BIN" --pr-diff "$FIX/pr-303.diff" \
    --reuse-flags "$flags_file"
  rm -f "$flags_file"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -qF 'BLOCK'
  printf '%s\n' "$output" | grep -qF 'ADVISE'
  printf '%s\n' "$output" | grep -qF 'PASS'
}

# ---------------------------------------------------------------------------
# Case 10: wire-in grep — SKILL.md fused reviewer path passes --reuse-flags
# ---------------------------------------------------------------------------
@test "wire-in: SKILL.md passes --reuse-flags to gen-reviewer-prompt.sh" {
  run grep -q -- '--reuse-flags' "$REPO_ROOT/skills/autospec-run/SKILL.md"
  [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
# Case 11: --reuse-flags appears in gen-reviewer-prompt.sh usage/arg parser
# ---------------------------------------------------------------------------
@test "wire-in: --reuse-flags option is documented in gen-reviewer-prompt.sh" {
  run grep -q -- '--reuse-flags' "$BIN"
  [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
# Issue #1441: consequence + refute pass + anti-gold-plating in SKILL.md trio
# ---------------------------------------------------------------------------

# Case 12: refute pass prose is present in both SKILL.md guardian blocks
@test "refute-pass: SKILL.md describes the reuse-BLOCK refute pass" {
  for f in "$REPO_ROOT/skills/autospec-run/SKILL.md" "$REPO_ROOT/skills/autospec/SKILL.md"; do
    grep -qF 'Reuse-BLOCK refute pass' "$f" \
      || { echo "FAIL: refute pass prose missing in $f"; return 1; }
    grep -qF 'Majority rules' "$f" \
      || { echo "FAIL: majority-rules prose missing in $f"; return 1; }
  done
}

# Case 13: simplicity axis is documented as ADVISE-only (anti-gold-plating)
@test "anti-gold-plating: simplicity axis is ADVISE-only in SKILL.md" {
  for f in "$REPO_ROOT/skills/autospec-run/SKILL.md" "$REPO_ROOT/skills/autospec/SKILL.md"; do
    grep -qF 'Simplicity axis is ADVISE-only' "$f" \
      || { echo "FAIL: ADVISE-only simplicity prose missing in $f"; return 1; }
    grep -qF 'never halt the commit' "$f" \
      || { echo "FAIL: never-block-toward-more-code prose missing in $f"; return 1; }
  done
}

# Case 14: refute pass + ADVISE-only prose propagated to all 6 trio mirrors
@test "trio: refute-pass + ADVISE-only prose present in all 6 trio files" {
  for f in \
    "$REPO_ROOT/skills/autospec-run/SKILL.md" \
    "$REPO_ROOT/skills/autospec-run/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-run/opencode/agent.md" \
    "$REPO_ROOT/skills/autospec/SKILL.md" \
    "$REPO_ROOT/skills/autospec/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec/opencode/agent.md"; do
    grep -qF 'Reuse-BLOCK refute pass' "$f" \
      || { echo "FAIL: refute pass prose missing in mirror $f"; return 1; }
    grep -qF 'Simplicity axis is ADVISE-only' "$f" \
      || { echo "FAIL: ADVISE-only prose missing in mirror $f"; return 1; }
  done
}
