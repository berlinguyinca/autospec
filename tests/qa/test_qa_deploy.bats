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
# that a valid contract clears the safety floor. With #1294 the runner now also
# executes the (stubbed) stages and writes the deploy: block, so we assert it
# runs end-to-end without tripping any safety category.
@test "valid contract via --repo-dir outside checkout -> passes safety floor, runs + writes deploy block" {
  _require_tools
  _install_contract "valid-simple-deploy.yml"
  _install_stub_bin
  VERDICT="$TMP_REPO/.autospec/qa-verdict.json"
  # $TMP_REPO is a mktemp -d /tmp/... dir (see setup), outside $REPO_ROOT.
  _run_with_stubs
  [ "$status" -eq 0 ]
  # Safety floor passed: none of the violation categories appear.
  [[ "$output" != *"qa_deploy_forbidden_target"* ]]
  [[ "$output" != *"qa_deploy_prod_pattern"* ]]
  [[ "$output" != *"qa_deploy_missing_records_cap"* ]]
  [[ "$output" != *"qa_deploy_invalid_contract"* ]]
  # deploy: block written into the verdict at --repo-dir.
  [ -f "$VERDICT" ]
  run jq -r '.deploy | has("stages_run")' "$VERDICT"
  [ "$output" = "true" ]
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

# ─────────────────────────────────────────────────────────────────────────────
# #1294 — stage execution + health probe + atomic deploy: block write.
#
# Real-services rule: the runner is NOT mocked. Deploy/clone/curl are
# PATH-stubbed via a per-test fixture bin/ whose shims write/serve REAL local
# files. No network. The runner runs end-to-end; only the leaf programs are
# substituted (exactly the spec's "fixture bin/" approach).
# ─────────────────────────────────────────────────────────────────────────────

# Build a fixture bin/ on $TMP_REPO/bin with deploy-stub, clone-stub, and a curl
# shim, then prepend it to PATH for the runner. The curl shim returns 200 for
# any URL except one containing "never-ready" (returns 503) so health-success
# and health-exhaustion are both exercised deterministically.
_install_stub_bin() {
  STUB_BIN="$TMP_REPO/bin"
  mkdir -p "$STUB_BIN"

  cat > "$STUB_BIN/deploy-stub" <<EOF
#!/usr/bin/env bash
# Writes a real local readiness marker so the "deploy" is observable on disk.
echo "deploy ok" > "$TMP_REPO/.autospec/deployed.marker"
echo "deploy-stub: target up"
exit 0
EOF
  chmod +x "$STUB_BIN/deploy-stub"

  cat > "$STUB_BIN/clone-stub" <<'EOF'
#!/usr/bin/env bash
# Emits the records_copied=<int> stdout line the runner parses for data_clone.
echo "cloning prod-readonly into test family"
echo "records_copied=8500"
exit 0
EOF
  chmod +x "$STUB_BIN/clone-stub"

  # curl shim: mimics `curl -s -o /dev/null -w '%{http_code}' <url>` by printing
  # an HTTP status code to stdout. 503 for the never-ready URL, 200 otherwise.
  cat > "$STUB_BIN/curl" <<'EOF'
#!/usr/bin/env bash
url=""
for a in "$@"; do url="$a"; done
case "$url" in
  *never-ready*) printf '503' ;;
  *)             printf '200' ;;
esac
exit 0
EOF
  chmod +x "$STUB_BIN/curl"

  # #1295 teardown shims. teardown-fail-stub exits non-zero (a failing teardown
  # command); teardown-ok-stub exits 0 and drops a real marker so a test can
  # prove it ran (or, under --skip-teardown, that it did NOT).
  cat > "$STUB_BIN/teardown-fail-stub" <<'EOF'
#!/usr/bin/env bash
echo "teardown-fail-stub: cleanup failed"
exit 1
EOF
  chmod +x "$STUB_BIN/teardown-fail-stub"

  cat > "$STUB_BIN/teardown-ok-stub" <<EOF
#!/usr/bin/env bash
echo "teardown ran" > "$TMP_REPO/.autospec/teardown.marker"
echo "teardown-ok-stub: cleaned up"
exit 0
EOF
  chmod +x "$STUB_BIN/teardown-ok-stub"
}

_run_with_stubs() {
  # Run the runner with the stub bin prepended to PATH and a verdict path.
  run env PATH="$STUB_BIN:$PATH" bash "$RUNNER" \
    --repo-dir "$TMP_REPO" --verdict "$VERDICT"
}

# ── fixture 2: simple deploy -> ordered stages_run + durations + target_env ───
@test "fixture 2: simple deploy -> exit 0, ordered stages_run + stage_durations_ms keys + target_env" {
  _require_tools
  _install_contract "valid-simple-deploy.yml"
  _install_stub_bin
  VERDICT="$TMP_REPO/.autospec/qa-verdict.json"
  _run_with_stubs
  [ "$status" -eq 0 ]
  # Verdict file written (bats 3.2-safe: real file on disk before any [ -f ]).
  [ -f "$VERDICT" ]

  # stages_run is the single stage, in order.
  run jq -c '.deploy.stages_run' "$VERDICT"
  [ "$output" = '["deploy-to-test-family"]' ]

  # stage_durations_ms has a key per stage and the value is an integer (we do
  # NOT assert exact ms — durations are wall-clock and non-deterministic).
  run jq -r '.deploy.stage_durations_ms | has("deploy-to-test-family")' "$VERDICT"
  [ "$output" = "true" ]
  run jq -r '.deploy.stage_durations_ms["deploy-to-test-family"] | type' "$VERDICT"
  [ "$output" = "number" ]

  # target_env recorded.
  run jq -r '.deploy.target_env' "$VERDICT"
  [ -n "$output" ]
  [ "$output" != "null" ]

  # Secret env VALUE never recorded anywhere in the verdict.
  run grep -F "super-secret-value" "$VERDICT"
  [ "$status" -ne 0 ]
}

# ── fixture 6: health check fails after retry -> exit 1 qa_deploy_failed ──────
@test "fixture 6: health check exhausts retries -> exit 1 qa_deploy_failed naming stage" {
  _require_tools
  _install_contract "health-fail.yml"
  _install_stub_bin
  VERDICT="$TMP_REPO/.autospec/qa-verdict.json"
  _run_with_stubs
  [ "$status" -eq 1 ]
  [[ "$output" == *"qa_deploy_failed"* ]]
  # Verdict written with the failed stage named in a finding.
  [ -f "$VERDICT" ]
  run jq -r '.findings[]? | select(.category|test("qa_deploy_failed")) | .summary' "$VERDICT"
  [[ "$output" == *"deploy-to-test-family"* ]]
}

# ── fixture 10: synthetic end-to-end -> deploy: block matches expected ───────
@test "fixture 10: synthetic end-to-end -> deploy: block structure + deterministic values" {
  _require_tools
  _install_contract "valid-e2e.yml"
  _install_stub_bin
  VERDICT="$TMP_REPO/.autospec/qa-verdict.json"
  _run_with_stubs
  [ "$status" -eq 0 ]
  [ -f "$VERDICT" ]

  # Ordered stages_run across both stages.
  run jq -c '.deploy.stages_run' "$VERDICT"
  [ "$output" = '["deploy-to-test-family","clone-production-data"]' ]

  # Durations: a key per stage, integer-typed (not exact ms).
  run jq -r '.deploy.stage_durations_ms | keys_unsorted | sort | join(",")' "$VERDICT"
  [ "$output" = "clone-production-data,deploy-to-test-family" ]
  run jq -r '[.deploy.stage_durations_ms[] | type] | unique | join(",")' "$VERDICT"
  [ "$output" = "number" ]

  # data_clone parsed from the records_copied=<int> stdout line + max_records.
  run jq -r '.deploy.data_clone.records_copied' "$VERDICT"
  [ "$output" = "8500" ]
  run jq -r '.deploy.data_clone.max_records' "$VERDICT"
  [ "$output" = "10000" ]

  # head_sha_deployed: present (short sha) — guarded if not a git repo, but
  # $TMP_REPO is not a git repo so it must be the explicit non-git sentinel.
  run jq -r '.deploy | has("head_sha_deployed")' "$VERDICT"
  [ "$output" = "true" ]

  # target_env recorded.
  run jq -r '.deploy.target_env' "$VERDICT"
  [ -n "$output" ]
  [ "$output" != "null" ]
}

# ── additive write: existing verdict keys preserved, deploy: merged in ───────
@test "deploy: block is additive -> pre-existing verdict keys preserved" {
  _require_tools
  _install_contract "valid-simple-deploy.yml"
  _install_stub_bin
  VERDICT="$TMP_REPO/.autospec/qa-verdict.json"
  # Seed a pre-existing verdict the audit would later own.
  printf '{"verdict":"PASS","findings":[{"category":"x","summary":"keep me"}]}\n' > "$VERDICT"
  [ -f "$VERDICT" ]
  _run_with_stubs
  [ "$status" -eq 0 ]
  run jq -r '.verdict' "$VERDICT"
  [ "$output" = "PASS" ]
  run jq -r '.findings[0].summary' "$VERDICT"
  [ "$output" = "keep me" ]
  run jq -r '.deploy | has("stages_run")' "$VERDICT"
  [ "$output" = "true" ]
}

# ── forbidden token in stage STDOUT at run time -> exit 3 mid-run ─────────────
@test "forbidden token appearing in stage stdout at run time -> exit 3 qa_deploy_forbidden_target" {
  _require_tools
  _install_stub_bin
  # A stage whose stub prints a forbidden token to stdout (clears the static
  # command/url floor, trips the run-time stdout check).
  cat > "$STUB_BIN/leak-stub" <<'EOF'
#!/usr/bin/env bash
echo "connecting to www.tidyboard.org now"
exit 0
EOF
  chmod +x "$STUB_BIN/leak-stub"
  cat > "$TMP_REPO/.autospec/qa-deploy.yml" <<'EOF'
required: true
stages:
  - name: deploy-to-test-family
    command: leak-stub
    timeout: 30
target_envs:
  allowed: [test.tidyboard.org]
  forbidden: [www.tidyboard.org, app.tidyboard.org]
EOF
  VERDICT="$TMP_REPO/.autospec/qa-verdict.json"
  _run_with_stubs
  [ "$status" -eq 3 ]
  [[ "$output" == *"qa_deploy_forbidden_target"* ]]
}

# ─────────────────────────────────────────────────────────────────────────────
# #1295 — post-verdict teardown + --skip-teardown + PASS->PARTIAL downgrade.
#
# Teardown runs AFTER the deploy: block is written (verdict final). Each
# teardown command is subject to the same safety floor as deploy stages and is
# PATH-stubbed (no network). on_failure: warn logs only; on_failure: fail
# downgrades a PASS verdict to PARTIAL (never a FAIL / already-PARTIAL verdict).
# All verdict updates are atomic (.tmp+os.replace) and additive.
# ─────────────────────────────────────────────────────────────────────────────

# ── fixture 7: teardown warn-failure -> verdict UNCHANGED, name recorded ──────
@test "fixture 7: teardown on_failure:warn failure -> verdict unchanged, name in teardown_failures" {
  _require_tools
  _install_contract "teardown-warn-fail.yml"
  _install_stub_bin
  VERDICT="$TMP_REPO/.autospec/qa-verdict.json"
  # Seed a PASS verdict the audit would own; a warn teardown must NOT touch it.
  printf '{"verdict":"PASS","findings":[]}\n' > "$VERDICT"
  [ -f "$VERDICT" ]
  _run_with_stubs
  # A warn-only teardown failure does not change the runner's success exit.
  [ "$status" -eq 0 ]
  # Verdict left unchanged (still PASS).
  run jq -r '.verdict' "$VERDICT"
  [ "$output" = "PASS" ]
  # Teardown ran.
  run jq -r '.deploy.teardown_run' "$VERDICT"
  [ "$output" = "true" ]
  # The failing teardown name is recorded.
  run jq -c '.deploy.teardown_failures' "$VERDICT"
  [ "$output" = '["snapshot-test-state"]' ]
}

# ── fixture 8: teardown fail-failure on a PASS -> PASS becomes PARTIAL ────────
@test "fixture 8: teardown on_failure:fail failure -> PASS downgraded to PARTIAL, name recorded" {
  _require_tools
  _install_contract "teardown-fail-downgrade.yml"
  _install_stub_bin
  VERDICT="$TMP_REPO/.autospec/qa-verdict.json"
  printf '{"verdict":"PASS","findings":[{"category":"x","summary":"keep me"}]}\n' > "$VERDICT"
  [ -f "$VERDICT" ]
  _run_with_stubs
  [ "$status" -eq 0 ]
  # PASS -> PARTIAL downgrade.
  run jq -r '.verdict' "$VERDICT"
  [ "$output" = "PARTIAL" ]
  # Additive: pre-existing findings preserved.
  run jq -r '.findings[0].summary' "$VERDICT"
  [ "$output" = "keep me" ]
  run jq -c '.deploy.teardown_failures' "$VERDICT"
  [ "$output" = '["cleanup-test-family"]' ]
  run jq -r '.deploy.teardown_run' "$VERDICT"
  [ "$output" = "true" ]
}

# ── teardown fail-failure must NOT alter a FAIL verdict (only PASS->PARTIAL) ──
@test "teardown on_failure:fail failure does NOT alter a FAIL verdict" {
  _require_tools
  _install_contract "teardown-fail-downgrade.yml"
  _install_stub_bin
  VERDICT="$TMP_REPO/.autospec/qa-verdict.json"
  # Seed a FAIL verdict; a fail teardown must leave it FAIL (never -> PARTIAL).
  printf '{"verdict":"FAIL","findings":[]}\n' > "$VERDICT"
  [ -f "$VERDICT" ]
  _run_with_stubs
  [ "$status" -eq 0 ]
  run jq -r '.verdict' "$VERDICT"
  [ "$output" = "FAIL" ]
  # The failure is still recorded.
  run jq -c '.deploy.teardown_failures' "$VERDICT"
  [ "$output" = '["cleanup-test-family"]' ]
}

# ── fixture 9: --skip-teardown -> teardown skipped, teardown_run:false ────────
@test "fixture 9: --skip-teardown -> teardown skipped entirely, deploy.teardown_run:false" {
  _require_tools
  _install_contract "teardown-ok.yml"
  _install_stub_bin
  VERDICT="$TMP_REPO/.autospec/qa-verdict.json"
  printf '{"verdict":"PASS","findings":[]}\n' > "$VERDICT"
  [ -f "$VERDICT" ]
  run env PATH="$STUB_BIN:$PATH" bash "$RUNNER" \
    --repo-dir "$TMP_REPO" --verdict "$VERDICT" --skip-teardown
  [ "$status" -eq 0 ]
  # Verdict unchanged.
  run jq -r '.verdict' "$VERDICT"
  [ "$output" = "PASS" ]
  # teardown_run recorded as false.
  run jq -r '.deploy.teardown_run' "$VERDICT"
  [ "$output" = "false" ]
  # The teardown command never ran (no marker written by teardown-ok-stub).
  [ ! -f "$TMP_REPO/.autospec/teardown.marker" ]
}
