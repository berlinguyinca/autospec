#!/usr/bin/env bats
# tests/validate-parallel.bats — TDD for validate.sh `--jobs N` parallel fan-out
# (issue #1123).
#
# Contracts under test (Shared contracts block, issue #1123):
#   - `--jobs N` parses; default (no flag) stays fully serial = byte-unchanged.
#   - `--jobs N` runs the independent per-skill checks concurrently, each worker
#     in its OWN temp file (no collision).
#   - `--jobs N` produces the SAME pass/fail verdict and the SAME sorted failure
#     output as serial on a seeded failing check.
#   - exit code is non-zero iff at least one worker failed.
#   - `--jobs` composes with `--changed` (parallelize only the selected checks).
#
# Real bats/subprocesses, no mocks (AGENTS.md: bash 3.2 safe; no `wait -n`,
# no `[ -f <(...) ]`).

REPO="${BATS_TEST_DIRNAME}/.."

setup() {
  TMP="$(mktemp -d)"
  CHANGED_FILE="$TMP/changed.txt"
}

teardown() {
  rm -rf "$TMP"
}

# ── arg parsing / default-serial guard ───────────────────────────────────────

@test "bare validate.sh (no --jobs) stays serial and never prints a jobs banner" {
  cd "$REPO"
  run bash scripts/validate.sh --fast
  [ "$status" -eq 0 ]
  ! printf '%s\n' "$output" | grep -qi 'parallel'
  ! printf '%s\n' "$output" | grep -q 'jobs:'
}

@test "default (no --jobs) byte-identical to --jobs 1" {
  cd "$REPO"
  run bash scripts/validate.sh --fast
  [ "$status" -eq 0 ]
  serial_out="$output"
  run bash scripts/validate.sh --fast --jobs 1
  [ "$status" -eq 0 ]
  # --jobs 1 takes the serial path → output identical to bare/default.
  [ "$output" = "$serial_out" ]
}

# ── parallel == serial verdict (green case) ──────────────────────────────────

@test "--jobs N on a clean tree is green, same verdict as serial" {
  cd "$REPO"
  run bash scripts/validate.sh --fast
  [ "$status" -eq 0 ]
  run bash scripts/validate.sh --fast --jobs 4
  [ "$status" -eq 0 ]
  # final OK line present in both.
  printf '%s\n' "$output" | grep -q 'OK — all validation checks passed.'
}

# ── deterministic sorted per-skill output under parallelism ──────────────────

@test "--jobs N per-skill lock-step lines are emitted in sorted (deterministic) order" {
  cd "$REPO"
  run bash scripts/validate.sh --fast --jobs 4
  [ "$status" -eq 0 ]
  # Extract the per-skill lock-step lines and confirm they are sorted —
  # independent of worker completion order.
  lines="$(printf '%s\n' "$output" | grep 'validate: lock-step: ')"
  sorted="$(printf '%s\n' "$lines" | LC_ALL=C sort)"
  [ "$lines" = "$sorted" ]
}

# ── seeded failure: parallel reproduces serial verdict + failure output ───────

@test "--jobs N on a seeded failing skill exits non-zero, same failure as serial" {
  # Create a throwaway clone of skills/ with one broken skill so a per-skill
  # check fails. Use AUTOSPEC override-free real tree: copy the repo skill set
  # into a sandbox is heavy; instead break lock-step in a temp skill mirror via
  # a copied checkout is overkill. We seed the failure deterministically by
  # corrupting one skill's codex/prompt.md in a worktree copy.
  WORK="$TMP/repo"
  cp -R "$REPO" "$WORK"
  cd "$WORK"
  # Pick a real multi-harness skill and break its lock-step (prompt drift).
  skill="$(ls -d skills/*/ | while read -r d; do
    [ -f "$d/SKILL.md" ] && [ -f "$d/opencode/agent.md" ] && [ -f "$d/codex/prompt.md" ] && { basename "$d"; break; }
  done)"
  [ -n "$skill" ]
  printf '\nDRIFT-INJECTED-LINE\n' >> "skills/$skill/codex/prompt.md"

  run bash scripts/validate.sh --fast
  serial_status="$status"
  serial_out="$output"
  [ "$serial_status" -ne 0 ]

  run bash scripts/validate.sh --fast --jobs 4
  [ "$status" -ne 0 ]
  # Both report the SAME failing skill's lock-step failure.
  printf '%s\n' "$serial_out" | grep -q "lock-step: $skill"
  printf '%s\n' "$output" | grep -q "lock-step: $skill"
}

# ── no temp-file collision across workers ────────────────────────────────────

@test "--jobs N workers do not collide on temp files (high concurrency green)" {
  cd "$REPO"
  # A high job count exercises many simultaneous workers; if temp paths
  # collided, output would corrupt and the verdict would flake.
  run bash scripts/validate.sh --fast --jobs 16
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q 'OK — all validation checks passed.'
}

# ── composes with --changed ──────────────────────────────────────────────────

@test "--changed --jobs N parallelizes only the selected per-skill checks" {
  cd "$REPO"
  export AUTOSPEC_VALIDATE_CHANGED_OVERRIDE="$CHANGED_FILE"
  printf 'skills/autospec-run/SKILL.md\n' > "$CHANGED_FILE"
  run bash scripts/validate.sh --changed --fast --jobs 4
  [ "$status" -eq 0 ]
  # Selected skill ran ...
  printf '%s\n' "$output" | grep -q 'lock-step: autospec-run'
  # ... unrelated skill did NOT (scoping preserved under parallelism).
  ! printf '%s\n' "$output" | grep -q 'lock-step: autospec-qa'
  # scoped line still emitted.
  printf '%s\n' "$output" | grep -q 'scoped: ran'
}

@test "--jobs auto resolves to a positive job count (green)" {
  cd "$REPO"
  run bash scripts/validate.sh --fast --jobs auto
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q 'OK — all validation checks passed.'
}
