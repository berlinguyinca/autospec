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

@test "bare validate.sh (default mode) never emits the scoped: line" {
  # Merge-gate invariant: the scoped `scoped: ran N/TOTAL` accounting line is a
  # --changed-only feature. Bare/default invocation must run the FULL check set
  # and must NOT emit the scoped line — otherwise the gate could silently skip
  # checks on a clean tree.
  #
  # REDESIGN NOTE (was: "byte-identical to pre-change snapshot"): the original
  # test diffed current `--fast` output against the validate.sh from the parent
  # of the first commit that touched validate-affected.sh. Every legitimate
  # check added since `--changed`/`--jobs` landed (dozens) changed that output,
  # so the byte-for-byte assertion has been red on origin/main for a long time
  # by construction — it asserted "validate.sh has not changed since <ancient
  # commit>", which is the opposite of what we want. The durable intent worth
  # guarding is the scoped-line invariant below; the per-mode scoping behavior
  # (which checks run) is already covered by tests 12-15. --fast avoids bats
  # recursion and keeps the run fast/deterministic.
  cd "${BATS_TEST_DIRNAME}/.."
  run bash scripts/validate.sh --fast
  [ "$status" -eq 0 ]
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

@test "validate.sh --changed global count includes the always-run installer syntax checks" {
  # Regression (Phase 5.5 audit, issue #1124): the scoped global-check count must
  # cover EVERY unconditionally-run global check, including the two trailing
  # check_bash_syntax install.sh / uninstall.sh calls. An empty diff runs zero
  # per-skill checks, so RAN == TOTAL == the global block size, and that size must
  # equal the number of global `check_*` CALL lines actually executed in main()
  # (no off-by-N undercount that makes the ratio look more-skipped than reality).
  cd "${BATS_TEST_DIRNAME}/.."
  export AUTOSPEC_VALIDATE_CHANGED_OVERRIDE="$CHANGED_FILE"
  : > "$CHANGED_FILE"   # empty diff → no skill runs, only the always-run globals
  run bash scripts/validate.sh --changed --fast
  [ "$status" -eq 0 ]
  scoped_line="$(printf '%s\n' "$output" | grep -o 'scoped: ran [0-9]*/[0-9]* checks')"
  ran="$(printf '%s\n' "$scoped_line" | sed -E 's#scoped: ran ([0-9]+)/[0-9]+ checks#\1#')"
  total="$(printf '%s\n' "$scoped_line" | sed -E 's#scoped: ran [0-9]+/([0-9]+) checks#\1#')"
  # Empty diff: zero per-skill checks run, so RAN is exactly the always-run global
  # block; the skipped skills still inflate TOTAL (that's the point of N/TOTAL),
  # so RAN <= TOTAL here, not necessarily equal.
  [ "$ran" -le "$total" ]
  # RAN must match the literal count of global check_* CALL lines executed: the
  # block from check_startup_preflight through the trailing check_bash_syntax
  # install/uninstall calls (every one is always-run in scoped mode).
  expected="$(awk '/^    # Scoped accounting/{exit} f && /^    check_/{n++} /^    check_startup_preflight$/{f=1; n++} END{print n+0}' scripts/validate.sh)"
  [ "$ran" -eq "$expected" ]
  # The two trailing installer syntax checks must be inside that count (guards the
  # off-by-2 the audit found): the count strictly exceeds the block that stops at
  # the "# Top-level installer" comment.
  pre_installer="$(awk '/^    # Top-level installer/{exit} f && /^    check_/{n++} /^    check_startup_preflight$/{f=1; n++} END{print n+0}' scripts/validate.sh)"
  [ "$expected" -eq "$((pre_installer + 2))" ]
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
