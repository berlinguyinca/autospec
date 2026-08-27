#!/usr/bin/env bats
# tests/unit/test_autospec_sweep_enabled_false.bats
#
# Regression coverage for the yq `//` alternative-operator bug in
# autospec-sweep: `//` treats a literal `false` as absent (same as jq), so
# every `.foo.enabled // true` toggle silently read back as `true` and an
# operator's explicit disable was ignored. `deploy_if_tests_require: false`
# is the worst case in this set — an operator disabling deploy and getting
# one anyway. This proves each toggle now honors an explicit `false`, and
# that an absent key still defaults to enabled.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  RUNNER="$REPO_ROOT/skills/autospec-sweep/scripts/run.sh"
  REVIEWER="$REPO_ROOT/skills/autospec-sweep/scripts/review.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-sweep-enabled-false-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

# Config with every toggle this bug affects explicitly set to false.
write_all_disabled_config() {
  mkdir -p "$TEST_TMPDIR/repo/.autospec"
  cat > "$TEST_TMPDIR/repo/.autospec/autospec.yml" <<'YAML'
project:
  findings:
    commands:
      test: ""
      e2e: ""
      deploy: ""
steps:
  review:
    enabled: false
  run:
    enabled: false
sweep:
  spec_sync:
    enabled: false
  improvement_budget:
    max_issues_per_sweep: 5
continuous_improvement:
  docs:
    enabled: false
  tests:
    enabled: false
  code:
    enabled: false
  loop:
    file_issues: false
    route_fixes_via_autospec_run: false
documentation:
  enabled: false
execution:
  tests:
    run_all_every_sweep: false
  deployment:
    deploy_if_tests_require: false
YAML
}

# Same shape but every `enabled`/toggle key is simply absent, which must
# still default to true (a deliberate, unrelated behavior this fix must not
# break).
write_all_absent_config() {
  mkdir -p "$TEST_TMPDIR/repo/.autospec"
  cat > "$TEST_TMPDIR/repo/.autospec/autospec.yml" <<'YAML'
project:
  findings:
    commands:
      test: ""
      e2e: ""
      deploy: ""
sweep:
  improvement_budget:
    max_issues_per_sweep: 5
YAML
}

@test "sweep run --dry-run reports every explicitly-disabled toggle as false" {
  write_all_disabled_config

  run bash "$RUNNER" run --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  json="$output"
  run bash -c 'printf "%s\n" "$1" | jq -e ".enabled.review == false and .enabled.run == false and .enabled.spec_sync == false and .enabled.tests == false and .execution.run_all_tests_every_sweep == false and .execution.deploy_if_tests_require == false"' _ "$json"
  [ "$status" -eq 0 ]
}

@test "sweep run --dry-run defaults every absent toggle to true" {
  write_all_absent_config

  run bash "$RUNNER" run --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  json="$output"
  run bash -c 'printf "%s\n" "$1" | jq -e ".enabled.review == true and .enabled.run == true and .enabled.spec_sync == true and .enabled.tests == true and .execution.run_all_tests_every_sweep == true and .execution.deploy_if_tests_require == true"' _ "$json"
  [ "$status" -eq 0 ]
}

@test "sweep run does not file gaps when continuous_improvement.loop.file_issues is false" {
  write_all_disabled_config
  # Re-enable run.enabled/tests so this exercises only file_issues, and
  # supply gaps directly so review_cmd is never invoked.
  gaps="$TEST_TMPDIR/gaps.json"
  cat > "$gaps" <<'JSON'
[{"gap_id":"G1","dimension":"docs","severity":"medium","file":"README.md","line":1,"title":"t","body":"b","dedupe_key":"k1"}]
JSON

  mkdir -p "$TEST_TMPDIR/scripts"
  cat > "$TEST_TMPDIR/scripts/gap-remediation-loop.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$TEST_TMPDIR/filed.log"
exit 0
EOF
  chmod +x "$TEST_TMPDIR/scripts/gap-remediation-loop.sh"

  AUTOSPEC_SCRIPTS_DIR="$TEST_TMPDIR/scripts" run bash "$RUNNER" run \
    --repo-root "$TEST_TMPDIR/repo" --gaps "$gaps"

  [ "$status" -eq 0 ]
  # The filing helper must never have been invoked.
  [ ! -f "$TEST_TMPDIR/filed.log" ]
  run jq -e '.gaps.file_status == "skipped"' "$TEST_TMPDIR/repo/.autospec/sweep/latest.json"
  [ "$status" -eq 0 ]
}

write_route_run_false_config() {
  mkdir -p "$TEST_TMPDIR/repo/.autospec"
  cat > "$TEST_TMPDIR/repo/.autospec/autospec.yml" <<'YAML'
project:
  findings:
    commands:
      test: ""
      e2e: ""
      deploy: ""
steps:
  review:
    enabled: false
  run:
    enabled: true
sweep:
  improvement_budget:
    max_issues_per_sweep: 5
continuous_improvement:
  loop:
    file_issues: false
    route_fixes_via_autospec_run: false
YAML
}

@test "sweep run does not recommend /autospec-run when route_fixes_via_autospec_run is false" {
  write_route_run_false_config
  # run.enabled is true for the recommendation to even be considered; only
  # route_fixes_via_autospec_run gates it here.
  gaps="$TEST_TMPDIR/gaps.json"
  cat > "$gaps" <<'JSON'
[{"gap_id":"G1","dimension":"docs","severity":"medium","file":"README.md","line":1,"title":"t","body":"b","dedupe_key":"k1"}]
JSON
  mkdir -p "$TEST_TMPDIR/scripts"
  cat > "$TEST_TMPDIR/scripts/gap-remediation-loop.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$TEST_TMPDIR/filed.log"
exit 0
EOF
  chmod +x "$TEST_TMPDIR/scripts/gap-remediation-loop.sh"

  AUTOSPEC_SCRIPTS_DIR="$TEST_TMPDIR/scripts" run bash "$RUNNER" run \
    --repo-root "$TEST_TMPDIR/repo" --gaps "$gaps" --no-file

  [ "$status" -eq 0 ]
  run bash -c 'printf "%s" "$1" | grep -q "next step: /autospec-run"' _ "$output"
  [ "$status" -ne 0 ]
}

# ── review.sh gap-gating toggles ────────────────────────────────────────────

write_review_config() {
  local spec_sync="$1" deploy_if_tests_require="$2" require_scope="$3"
  local docs_enabled="${4:-true}" documentation_enabled="${5:-true}" \
        tests_enabled="${6:-true}" code_enabled="${7:-false}"
  mkdir -p "$TEST_TMPDIR/repo/.autospec" "$TEST_TMPDIR/repo/docs"
  cat > "$TEST_TMPDIR/repo/docs/user.md" <<'MD'
# User docs
No scope marker here.
MD
  cat > "$TEST_TMPDIR/repo/.autospec/autospec.yml" <<YAML
project:
  findings:
    commands:
      test: ""
      e2e: "echo run-e2e"
      deploy: ""
sweep:
  spec_sync:
    enabled: $spec_sync
continuous_improvement:
  docs:
    enabled: $docs_enabled
  tests:
    enabled: $tests_enabled
  code:
    enabled: $code_enabled
documentation:
  enabled: $documentation_enabled
  audiences:
    - id: user
      label: User Guide
      path: docs/user.md
      focus: users
      require_scope: $require_scope
  scopes:
    - id: backend
      label: Backend Scope
      path: docs/user.md
      focus: backend
      require_scope: $require_scope
execution:
  deployment:
    deploy_if_tests_require: $deploy_if_tests_require
YAML
}

@test "review.sh does not emit spec_sync gap when sweep.spec_sync.enabled is false" {
  write_review_config false true true
  out="$TEST_TMPDIR/gaps.json"

  run bash "$REVIEWER" --repo-root "$TEST_TMPDIR/repo" --emit-gaps "$out"

  [ "$status" -eq 0 ]
  run jq -e 'any(.[]; .dedupe_key == "autospec-specs-directory")' "$out"
  [ "$status" -ne 0 ]
}

@test "review.sh does not emit the deploy-command gap when deploy_if_tests_require is false" {
  write_review_config true false true
  out="$TEST_TMPDIR/gaps.json"

  run bash "$REVIEWER" --repo-root "$TEST_TMPDIR/repo" --emit-gaps "$out"

  [ "$status" -eq 0 ]
  run jq -e 'any(.[]; .dedupe_key == "autospec-config-deploy-command")' "$out"
  [ "$status" -ne 0 ]
}

@test "review.sh does not emit the doc-scope gap when require_scope is false" {
  write_review_config true true false
  out="$TEST_TMPDIR/gaps.json"

  run bash "$REVIEWER" --repo-root "$TEST_TMPDIR/repo" --emit-gaps "$out"

  [ "$status" -eq 0 ]
  run jq -e 'any(.[]; .dedupe_key == "autospec-doc-scope-marker-audience-user")' "$out"
  [ "$status" -ne 0 ]
}

@test "review.sh emits the doc-scope gap by default when require_scope is absent" {
  write_review_config true true true
  out="$TEST_TMPDIR/gaps.json"

  run bash "$REVIEWER" --repo-root "$TEST_TMPDIR/repo" --emit-gaps "$out"

  [ "$status" -eq 0 ]
  run jq -e 'any(.[]; .dedupe_key == "autospec-doc-scope-marker-audience-user")' "$out"
  [ "$status" -eq 0 ]
}

@test "review.sh does not emit the scope doc-scope gap when require_scope is false (scopes list)" {
  write_review_config true true false
  out="$TEST_TMPDIR/gaps.json"

  run bash "$REVIEWER" --repo-root "$TEST_TMPDIR/repo" --emit-gaps "$out"

  [ "$status" -eq 0 ]
  run jq -e 'any(.[]; .dedupe_key == "autospec-doc-scope-marker-scope-backend")' "$out"
  [ "$status" -ne 0 ]
}

@test "review.sh does not emit the README gap when continuous_improvement.docs.enabled is false" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec"
  write_review_config true true true false true true false
  rm -f "$TEST_TMPDIR/repo/README.md"
  out="$TEST_TMPDIR/gaps.json"

  run bash "$REVIEWER" --repo-root "$TEST_TMPDIR/repo" --emit-gaps "$out"

  [ "$status" -eq 0 ]
  run jq -e 'any(.[]; .dedupe_key == "autospec-readme-missing")' "$out"
  [ "$status" -ne 0 ]
}

@test "review.sh does not emit documentation target gaps when documentation.enabled is false" {
  write_review_config true true true true false true false
  out="$TEST_TMPDIR/gaps.json"

  run bash "$REVIEWER" --repo-root "$TEST_TMPDIR/repo" --emit-gaps "$out"

  [ "$status" -eq 0 ]
  run jq -e 'any(.[]; .dedupe_key == "autospec-doc-scope-marker-audience-user")' "$out"
  [ "$status" -ne 0 ]
}

@test "review.sh does not emit the test-command gap when continuous_improvement.tests.enabled is false" {
  write_review_config true true true true true false false
  out="$TEST_TMPDIR/gaps.json"

  run bash "$REVIEWER" --repo-root "$TEST_TMPDIR/repo" --emit-gaps "$out"

  [ "$status" -eq 0 ]
  run jq -e 'any(.[]; .dedupe_key == "autospec-config-test-command")' "$out"
  [ "$status" -ne 0 ]
}

@test "review.sh does not emit the TODO-marker gap when continuous_improvement.code.enabled is false" {
  command -v rg >/dev/null 2>&1 || skip "ripgrep (rg) not available"
  write_review_config true true true true true true false
  printf '# TODO: fix this\n' > "$TEST_TMPDIR/repo/marker.txt"
  out="$TEST_TMPDIR/gaps.json"

  run bash "$REVIEWER" --repo-root "$TEST_TMPDIR/repo" --emit-gaps "$out"

  [ "$status" -eq 0 ]
  run jq -e 'any(.[]; .dedupe_key == "autospec-code-todo-markers")' "$out"
  [ "$status" -ne 0 ]
}

@test "review.sh emits the TODO-marker gap by default when code.enabled is absent" {
  command -v rg >/dev/null 2>&1 || skip "ripgrep (rg) not available"
  write_review_config true true true true true true true
  printf '# TODO: fix this\n' > "$TEST_TMPDIR/repo/marker.txt"
  out="$TEST_TMPDIR/gaps.json"

  run bash "$REVIEWER" --repo-root "$TEST_TMPDIR/repo" --emit-gaps "$out"

  [ "$status" -eq 0 ]
  run jq -e 'any(.[]; .dedupe_key == "autospec-code-todo-markers")' "$out"
  [ "$status" -eq 0 ]
}
