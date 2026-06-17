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
  # Guard: default (no-flag) behavior must be byte-for-byte unchanged. We
  # actually materialize the pre-feature script (the parent of the first commit
  # that touched scripts/lib/validate-affected.sh — i.e. before --changed/--jobs
  # landed) and diff its bare --fast output against the current script's. Any
  # byte difference in the structural-check stream (stdout or stderr) fails. We
  # use --fast to avoid bats recursion and keep the comparison deterministic.
  cd "${BATS_TEST_DIRNAME}/.."
  # First commit that introduced the affected-set lib; its parent is the
  # pre-feature snapshot. If history is unavailable (shallow clone), skip.
  feat_commit="$(git log --reverse --format=%H -- scripts/lib/validate-affected.sh 2>/dev/null | head -1)"
  [ -n "$feat_commit" ] || skip "validate-affected.sh history unavailable"
  pre_ref="${feat_commit}~1"
  pre_script="$TMP/validate-pre.sh"
  git show "${pre_ref}:scripts/validate.sh" > "$pre_script" 2>/dev/null \
    || skip "pre-feature validate.sh unavailable at ${pre_ref}"
  # Run the pre-feature script from the scripts/ dir context (it resolves
  # REPO_ROOT via dirname "$0"/..), then the current one, and diff both streams.
  cp "$pre_script" scripts/.validate-pre-snapshot.sh
  pre_out="$(bash scripts/.validate-pre-snapshot.sh --fast 2>"$TMP/pre.err")"; pre_rc=$?
  pre_err="$(cat "$TMP/pre.err")"
  rm -f scripts/.validate-pre-snapshot.sh
  cur_out="$(bash scripts/validate.sh --fast 2>"$TMP/cur.err")"; cur_rc=$?
  cur_err="$(cat "$TMP/cur.err")"
  [ "$pre_rc" -eq "$cur_rc" ]
  [ "$pre_out" = "$cur_out" ]
  [ "$pre_err" = "$cur_err" ]
  # The scoped line must NEVER appear in bare/default mode.
  ! printf '%s\n' "$cur_out" | grep -q 'scoped: ran'
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
