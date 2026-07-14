#!/usr/bin/env bats
# tests/define/test_define_base_flag.bats — TDD for issue #1100 (B1).
#
# Verifies the `--base <branch>` flag (default `main`) is documented in the
# autospec-define and autospec-split "Existing spec mode" prose, that the
# spec-tracking precheck and child-issue blob URL parameterize on <base>
# instead of hard-coding `origin/main`/`blob/main`, and that the default-main
# path stays byte-unchanged (the literal default token `main` is still present).
# No mocks: asserts directly against the committed skill sources + lockstep.

DEFINE_SKILL="${BATS_TEST_DIRNAME}/../../skills/autospec-define/SKILL.md"
DEFINE_CODEX="${BATS_TEST_DIRNAME}/../../skills/autospec-define/codex/prompt.md"
DEFINE_OPENCODE="${BATS_TEST_DIRNAME}/../../skills/autospec-define/opencode/agent.md"
SPLIT_SKILL="${BATS_TEST_DIRNAME}/../../skills/autospec-split/SKILL.md"
SPLIT_CODEX="${BATS_TEST_DIRNAME}/../../skills/autospec-split/codex/prompt.md"
SPLIT_OPENCODE="${BATS_TEST_DIRNAME}/../../skills/autospec-split/opencode/agent.md"

# strip_body mirrors autospec validate: body is everything after the 2nd `---`.
strip_body() {
  awk '/^---$/{c++; next} c>=2' "$1"
}

@test "autospec-define documents --base <branch> with default main" {
  run grep -qE -- '--base <branch>' "$DEFINE_SKILL"
  [ "$status" -eq 0 ]
  # Default must be stated as main so current behavior is byte-unchanged.
  run grep -qE 'default .*main' "$DEFINE_SKILL"
  [ "$status" -eq 0 ]
}

@test "autospec-split documents --base <branch> with default main" {
  run grep -qE -- '--base <branch>' "$SPLIT_SKILL"
  [ "$status" -eq 0 ]
  run grep -qE 'default .*main' "$SPLIT_SKILL"
  [ "$status" -eq 0 ]
}

@test "autospec-define spec-tracking precheck parameterizes on <base>" {
  # The gate must verify against <base>, not hard-coded origin/main.
  run grep -qF 'git cat-file -e <base>:<spec-path>' "$DEFINE_SKILL"
  [ "$status" -eq 0 ]
}

@test "autospec-split spec-tracking precheck parameterizes on <base>" {
  run grep -qF 'git cat-file -e <base>:<spec-path>' "$SPLIT_SKILL"
  [ "$status" -eq 0 ]
}

@test "autospec-define spec_url blob link resolves against <base>" {
  run grep -qF 'blob/<base>' "$DEFINE_SKILL"
  [ "$status" -eq 0 ]
}

@test "autospec-split spec_url blob link resolves against <base>" {
  run grep -qF 'blob/<base>' "$SPLIT_SKILL"
  [ "$status" -eq 0 ]
}

@test "autospec-define no longer hard-codes origin/main in the existing-spec gate" {
  # The cat-file gate line must not pin origin/main.
  run grep -qF 'git cat-file -e origin/main:<spec-path>' "$DEFINE_SKILL"
  [ "$status" -ne 0 ]
}

@test "autospec-split no longer hard-codes origin/main in the existing-spec gate" {
  run grep -qF 'git cat-file -e origin/main:<spec-path>' "$SPLIT_SKILL"
  [ "$status" -ne 0 ]
}

@test "autospec-define guardrail: non-main base never targets main for PR/issue" {
  # The guardrail names the non-main base and states no PR/issue targets main.
  run grep -qF 'under a non-main `--base <branch>`, no PR or issue targets' "$DEFINE_SKILL"
  [ "$status" -eq 0 ]
}

@test "autospec-define trio stays in lockstep (SKILL body == codex == opencode body)" {
  run diff <(strip_body "$DEFINE_SKILL") "$DEFINE_CODEX"
  [ "$status" -eq 0 ]
  run diff <(strip_body "$DEFINE_SKILL") <(strip_body "$DEFINE_OPENCODE")
  [ "$status" -eq 0 ]
}

@test "autospec-split trio stays in lockstep (SKILL body == codex == opencode body)" {
  run diff <(strip_body "$SPLIT_SKILL") "$SPLIT_CODEX"
  [ "$status" -eq 0 ]
  run diff <(strip_body "$SPLIT_SKILL") <(strip_body "$SPLIT_OPENCODE")
  [ "$status" -eq 0 ]
}
