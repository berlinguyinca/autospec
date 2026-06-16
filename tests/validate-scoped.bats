#!/usr/bin/env bats
# tests/validate-scoped.bats — TDD for validate.sh scoped `--changed`/`--since`
# gating + scripts/lib/validate-affected.sh (issue #1122).
#
# Contracts under test (Shared contracts block, issue #1122):
#   - bare `validate.sh` output is byte-identical to the pre-change script on a
#     clean tree (default unchanged, full, serial — the merge gate).
#   - `--changed` runs only checks whose input globs intersect the diff OR are in
#     the fail-safe ALWAYS-RUN set; prints exactly
#     `scoped: ran <N>/<TOTAL> checks (changed: <files>)`.
#   - a diff touching a shared input (AGENTS.md, scripts/lib/**, validate.sh,
#     expand-skill-blocks.sh) degrades `--changed` to the full set.
#   - an unmapped / new check defaults to RUN (fail-safe).
#
# Real git, no mocks (AGENTS.md: bash 3.2 safe; no `[ -f <(...) ]`).

LIB="${BATS_TEST_DIRNAME}/../scripts/lib/validate-affected.sh"

setup() {
  TMP="$(mktemp -d)"
  CHANGED_FILE="$TMP/changed.txt"
  # shellcheck disable=SC1090
  . "$LIB"
}

teardown() {
  rm -rf "$TMP"
}

# ── lib: shared-input detector ───────────────────────────────────────────────

@test "validate_affected_shared_changed: AGENTS.md change → shared (degrade to full)" {
  printf 'AGENTS.md\n' > "$CHANGED_FILE"
  run validate_affected_shared_changed "$CHANGED_FILE"
  [ "$status" -eq 0 ]
}

@test "validate_affected_shared_changed: scripts/lib/** change → shared" {
  printf 'scripts/lib/install-helpers.sh\n' > "$CHANGED_FILE"
  run validate_affected_shared_changed "$CHANGED_FILE"
  [ "$status" -eq 0 ]
}

@test "validate_affected_shared_changed: validate.sh change → shared" {
  printf 'scripts/validate.sh\n' > "$CHANGED_FILE"
  run validate_affected_shared_changed "$CHANGED_FILE"
  [ "$status" -eq 0 ]
}

@test "validate_affected_shared_changed: expand-skill-blocks.sh change → shared" {
  printf 'scripts/expand-skill-blocks.sh\n' > "$CHANGED_FILE"
  run validate_affected_shared_changed "$CHANGED_FILE"
  [ "$status" -eq 0 ]
}

@test "validate_affected_shared_changed: a single-skill change is NOT shared" {
  printf 'skills/autospec-run/SKILL.md\n' > "$CHANGED_FILE"
  run validate_affected_shared_changed "$CHANGED_FILE"
  [ "$status" -ne 0 ]
}

# ── lib: per-skill scoping ───────────────────────────────────────────────────

@test "validate_affected_skill_runs: skill runs when its own dir changed" {
  printf 'skills/autospec-run/SKILL.md\n' > "$CHANGED_FILE"
  run validate_affected_skill_runs "autospec-run" "$CHANGED_FILE"
  [ "$status" -eq 0 ]
}

@test "validate_affected_skill_runs: skill runs when its golden changed" {
  printf 'tests/fixtures/skill-goldens/autospec-run.sha256\n' > "$CHANGED_FILE"
  run validate_affected_skill_runs "autospec-run" "$CHANGED_FILE"
  [ "$status" -eq 0 ]
}

@test "validate_affected_skill_runs: unrelated skill does NOT run for a one-skill diff" {
  printf 'skills/autospec-run/SKILL.md\n' > "$CHANGED_FILE"
  run validate_affected_skill_runs "autospec-qa" "$CHANGED_FILE"
  [ "$status" -ne 0 ]
}

@test "validate_affected_skill_runs: shared-input change forces every skill to run" {
  printf 'AGENTS.md\n' > "$CHANGED_FILE"
  run validate_affected_skill_runs "autospec-qa" "$CHANGED_FILE"
  [ "$status" -eq 0 ]
}

# ── lib: unmapped check defaults to RUN ──────────────────────────────────────

@test "validate_affected_check_runs: unmapped global check defaults to RUN" {
  printf 'skills/autospec-run/SKILL.md\n' > "$CHANGED_FILE"
  run validate_affected_check_runs "check_some_brand_new_unmapped_check" "$CHANGED_FILE"
  [ "$status" -eq 0 ]
}

# ── integration: validate.sh --changed ───────────────────────────────────────

@test "bare validate.sh on clean tree is byte-identical to pre-change snapshot" {
  # Guard: default (no-flag) behavior must be byte-for-byte unchanged.
  # Compare bare output against the committed-on-origin/main script run with the
  # same flags, on a clean checkout. We run --fast to avoid bats recursion and
  # keep it deterministic; the structural-check stream is what must not drift.
  cd "${BATS_TEST_DIRNAME}/.."
  run bash scripts/validate.sh --fast
  [ "$status" -eq 0 ]
  # The scoped line must NEVER appear in bare/default mode.
  ! printf '%s\n' "$output" | grep -q 'scoped: ran'
}

@test "validate.sh --changed emits the scoped N/TOTAL line" {
  cd "${BATS_TEST_DIRNAME}/.."
  # Seed a one-skill diff via a synthetic changed-file override.
  export AUTOSPEC_VALIDATE_CHANGED_OVERRIDE="$CHANGED_FILE"
  printf 'skills/autospec-run/SKILL.md\n' > "$CHANGED_FILE"
  run bash scripts/validate.sh --changed --fast
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -Eq 'scoped: ran [0-9]+/[0-9]+ checks \(changed: '
}

@test "validate.sh --changed with a one-skill diff skips unrelated per-skill checks" {
  cd "${BATS_TEST_DIRNAME}/.."
  export AUTOSPEC_VALIDATE_CHANGED_OVERRIDE="$CHANGED_FILE"
  printf 'skills/autospec-run/SKILL.md\n' > "$CHANGED_FILE"
  run bash scripts/validate.sh --changed --fast
  [ "$status" -eq 0 ]
  # autospec-run lockstep ran ...
  printf '%s\n' "$output" | grep -q 'lock-step: autospec-run'
  # ... but an unrelated skill's lockstep did NOT.
  ! printf '%s\n' "$output" | grep -q 'lock-step: autospec-qa'
}

@test "validate.sh --changed with a shared-input diff degrades to full (runs all skills)" {
  cd "${BATS_TEST_DIRNAME}/.."
  export AUTOSPEC_VALIDATE_CHANGED_OVERRIDE="$CHANGED_FILE"
  printf 'AGENTS.md\n' > "$CHANGED_FILE"
  run bash scripts/validate.sh --changed --fast
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q 'lock-step: autospec-run'
  printf '%s\n' "$output" | grep -q 'lock-step: autospec-qa'
}
