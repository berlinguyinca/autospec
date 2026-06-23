#!/usr/bin/env bats
#
# test_qa_deploy.bats — runner CORE coverage for scripts/qa-deploy-runner.sh
# (autospec issue #1293, part of #694).
#
# Scope of THIS test (parse + safety floor + no-op only; stage execution,
# health probes, and verdict-write are #1294/#1295):
#   fixture 1 — absent contract        -> exit 0, no-op, writes nothing
#   fixture 3 — forbidden URL/command  -> exit 3 qa_deploy_forbidden_target
#   fixture 4 — production pattern      -> exit 3 qa_deploy_prod_pattern
#   fixture 5 — clone w/o max_records   -> exit 3 qa_deploy_missing_records_cap
#   plus: invalid contract (malformed YAML / missing required) -> exit 2
#
# bats 3.2-safe: every contract is copied to a REAL temp file/dir before the
# runner reads it (no `[ -f <(...) ]` process substitution). Tests skip with a
# message — never silently false-pass — when yq/jq/ajv/node are unavailable.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  RUNNER="$REPO_ROOT/scripts/qa-deploy-runner.sh"
  FIXTURES="$REPO_ROOT/tests/qa/fixtures"
  TMP_REPO="$(mktemp -d /tmp/autospec-qa-deploy-runner-XXXXXX)"
  mkdir -p "$TMP_REPO/.autospec"
}

teardown() {
  rm -rf "$TMP_REPO"
}

# Require the full toolchain or skip with a message (no false pass).
_require_tools() {
  command -v yq   >/dev/null 2>&1 || skip "yq not available (brew install yq)"
  command -v jq   >/dev/null 2>&1 || skip "jq not available (brew install jq)"
  command -v ajv  >/dev/null 2>&1 || skip "ajv CLI not available (npm install -g ajv-cli)"
  command -v node >/dev/null 2>&1 || skip "node not available"
}

# Copy a fixture into the temp repo's real .autospec/qa-deploy.yml on disk.
_install_contract() {
  cp "$FIXTURES/$1" "$TMP_REPO/.autospec/qa-deploy.yml"
  [ -f "$TMP_REPO/.autospec/qa-deploy.yml" ]   # bats 3.2-safe: real file
}

@test "runner exists and is executable" {
  [ -f "$RUNNER" ]
  [ -x "$RUNNER" ]
}

# ── fixture 1: absent contract -> exit 0, no-op, writes nothing ───────────────
@test "fixture 1: absent contract -> exit 0 no-op, writes nothing" {
  # No .autospec/qa-deploy.yml present.
  rm -f "$TMP_REPO/.autospec/qa-deploy.yml"
  run bash "$RUNNER" --repo-dir "$TMP_REPO"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  # Nothing written into .autospec.
  run ls -A "$TMP_REPO/.autospec"
  [ -z "$output" ]
}

# ── valid cross-directory: --repo-dir outside the checkout passes safety floor ─
# Proves the runner reads its contract from --repo-dir (a mktemp dir under /tmp,
# a different directory tree from $REPO_ROOT) rather than SCRIPT_DIR/.., and
# that a valid contract clears the safety floor. Scope is parse + safety-floor +
# no-op only: no stage execution, health probes, or verdict-write is asserted.
@test "valid contract via --repo-dir outside checkout -> passes safety floor, writes nothing" {
  _require_tools
  _install_contract "valid.yml"
  # $TMP_REPO is a mktemp -d /tmp/... dir (see setup), outside $REPO_ROOT.
  run bash "$RUNNER" --repo-dir "$TMP_REPO"
  [ "$status" -eq 0 ]
  # Safety floor passed: none of the violation categories appear.
  [[ "$output" != *"qa_deploy_forbidden_target"* ]]
  [[ "$output" != *"qa_deploy_prod_pattern"* ]]
  [[ "$output" != *"qa_deploy_missing_records_cap"* ]]
  [[ "$output" != *"qa_deploy_invalid_contract"* ]]
  # No artifact written into .autospec beyond the installed contract.
  run ls -A "$TMP_REPO/.autospec"
  [ "$output" = "qa-deploy.yml" ]
}

# ── fixture 3: forbidden token in command/url -> exit 3 forbidden_target ──────
@test "fixture 3: forbidden target token in command -> exit 3 qa_deploy_forbidden_target, zero stages run" {
  _require_tools
  _install_contract "invalid-forbidden-in-command.yml"
  run bash "$RUNNER" --repo-dir "$TMP_REPO"
  [ "$status" -eq 3 ]
  [[ "$output" == *"qa_deploy_forbidden_target"* ]]
  # Zero stages run: this core never executes stages, and no verdict/output
  # artifact is written into .autospec beyond the contract we installed.
  run ls -A "$TMP_REPO/.autospec"
  [ "$output" = "qa-deploy.yml" ]
}

# ── fixture 4: production pattern -> exit 3 prod_pattern ──────────────────────
@test "fixture 4: production pattern in command -> exit 3 qa_deploy_prod_pattern" {
  _require_tools
  _install_contract "invalid-prod-pattern.yml"
  run bash "$RUNNER" --repo-dir "$TMP_REPO"
  [ "$status" -eq 3 ]
  [[ "$output" == *"qa_deploy_prod_pattern"* ]]
}

# ── fixture 5: clone stage missing max_records -> exit 3 missing_records_cap ──
@test "fixture 5: data-clone stage missing max_records -> exit 3 qa_deploy_missing_records_cap" {
  _require_tools
  _install_contract "invalid-clone-missing-max-records.yml"
  run bash "$RUNNER" --repo-dir "$TMP_REPO"
  [ "$status" -eq 3 ]
  [[ "$output" == *"qa_deploy_missing_records_cap"* ]]
}

# ── invalid contract: malformed YAML -> exit 2 ───────────────────────────────
@test "malformed YAML contract -> exit 2 qa_deploy_invalid_contract" {
  _require_tools
  _install_contract "invalid-malformed-yaml.yml"
  run bash "$RUNNER" --repo-dir "$TMP_REPO"
  [ "$status" -eq 2 ]
  [[ "$output" == *"qa_deploy_invalid_contract"* ]]
}

# ── invalid contract: missing required (schema) -> exit 2 ────────────────────
@test "contract missing required target_envs.forbidden -> exit 2 qa_deploy_invalid_contract" {
  _require_tools
  _install_contract "invalid-missing-forbidden.yml"
  run bash "$RUNNER" --repo-dir "$TMP_REPO"
  [ "$status" -eq 2 ]
  [[ "$output" == *"qa_deploy_invalid_contract"* ]]
}

# ── missing tool path is reachable + actionable (only meaningful w/o ajv) ─────
@test "missing tool -> exit 2 with actionable brew/npm install message" {
  command -v yq >/dev/null 2>&1 || skip "yq not available (brew install yq)"
  _install_contract "valid.yml"
  # Build a sanitized PATH dir that contains every tool the script needs to even
  # start (bash, mktemp, cat, grep, jq, node) EXCEPT yq, so the first dependency
  # check (yq) fires exit 2 with an install hint. This exercises the
  # missing-tool branch without nuking PATH entirely (which would break bash).
  local stub="$TMP_REPO/stubbin"
  mkdir -p "$stub"
  for t in bash sh env mktemp cat grep sed dirname jq node ls printf; do
    p="$(command -v "$t" 2>/dev/null || true)"
    if [ -n "$p" ]; then ln -sf "$p" "$stub/$t"; fi
  done
  # Deliberately do NOT link yq into $stub.
  run env PATH="$stub" bash "$RUNNER" --repo-dir "$TMP_REPO"
  [ "$status" -eq 2 ]
  [[ "$output" == *"qa_deploy_invalid_contract"* ]]
  [[ "$output" == *"install"* ]]
  [[ "$output" == *"yq"* ]]
}
