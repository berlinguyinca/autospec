#!/usr/bin/env bats
# tests/unit/test_autospec_sweep_run.bats — executable autospec-sweep runner.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  WIZARD="$REPO_ROOT/skills/autospec-sweep/scripts/wizard.sh"
  RUNNER="$REPO_ROOT/skills/autospec-sweep/scripts/run.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-sweep-run-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

make_configured_repo() {
  mkdir -p "$TEST_TMPDIR/repo"
  bash "$WIZARD" init --repo-root "$TEST_TMPDIR/repo" --answers "$REPO_ROOT/tests/fixtures/autospec-sweep/minimal-answers.yml" >/dev/null
  printf '%s\n' "$TEST_TMPDIR/repo"
}

@test "autospec-sweep run refuses when config is missing" {
  mkdir -p "$TEST_TMPDIR/repo"

  run bash "$RUNNER" run --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 2 ]
  [[ "$output" == *".autospec/autospec.yml"* ]]
}

@test "autospec-sweep run --dry-run emits a machine-readable command plan" {
  repo="$(make_configured_repo)"

  run bash "$RUNNER" run --repo-root "$repo" --dry-run

  [ "$status" -eq 0 ]
  run bash -c "printf '%s' '$output' | jq -e '.mode == \"dry-run\" and .config == \".autospec/autospec.yml\" and .commands.review != null'"
  [ "$status" -eq 0 ]
}

@test "autospec-sweep run validates config through ajv when repo schema is present" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"
  repo="$(make_configured_repo)"
  mkdir -p "$repo/schemas"
  cp "$REPO_ROOT/schemas/autospec-config.schema.json" "$repo/schemas/autospec-config.schema.json"

  run bash "$RUNNER" run --repo-root "$repo" --dry-run

  [ "$status" -eq 0 ]
  [[ "$output" == *"\"mode\": \"dry-run\""* ]]
}

@test "autospec-sweep run writes state and copies provided gaps without filing in --no-file mode" {
  repo="$(make_configured_repo)"
  gaps="$TEST_TMPDIR/gaps.json"
  cat > "$gaps" <<'JSON'
[
  {
    "gap_id": "G1",
    "dimension": "docs",
    "severity": "medium",
    "file": "README.md",
    "line": 1,
    "title": "Document sweep",
    "body": "README.md needs the sweep command.",
    "dedupe_key": "docs-readme-sweep"
  }
]
JSON

  run bash "$RUNNER" run --repo-root "$repo" --gaps "$gaps" --no-file

  [ "$status" -eq 0 ]
  [ -f "$repo/.autospec/sweep/latest.json" ]
  run yq -r '.gaps.count' "$repo/.autospec/sweep/latest.json"
  [ "$output" = "1" ]
}

@test "autospec-sweep run can execute a configured review command that emits gaps" {
  repo="$(make_configured_repo)"
  stub="$TEST_TMPDIR/review-stub.sh"
  cat > "$stub" <<'SH'
#!/usr/bin/env bash
set -eu
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --emit-gaps) out="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cat > "$out" <<'JSON'
[{"dimension":"tests","severity":"high","file":"tests/example.bats","line":7,"title":"Add regression test","body":"Missing regression coverage.","dedupe_key":"tests-regression"}]
JSON
SH
  chmod +x "$stub"

  run env AUTOSPEC_SWEEP_REVIEW_CMD="$stub" bash "$RUNNER" run --repo-root "$repo" --no-file

  [ "$status" -eq 0 ]
  run yq -r '.gaps.count' "$repo/.autospec/sweep/latest.json"
  [ "$output" = "1" ]
}

@test "autospec-sweep run uses bundled executable review wrapper by default" {
  repo="$(make_configured_repo)"

  run bash "$RUNNER" run --repo-root "$repo" --no-file

  [ "$status" -eq 0 ]
  [ -f "$repo/.autospec/sweep/gaps.json" ]
  run jq -r '.[].dedupe_key' "$repo/.autospec/sweep/gaps.json"
  [[ "$output" == *"autospec-config-test-command"* ]]
}

@test "autospec-sweep run executes configured all-test command every time" {
  repo="$(make_configured_repo)"
  cat > "$repo/all-tests.sh" <<'SH'
#!/usr/bin/env bash
set -eu
printf 'all-tests\n' >> sweep-command.log
SH
  chmod +x "$repo/all-tests.sh"
  yq -i '.project.findings.commands.test = "./all-tests.sh"' "$repo/.autospec/autospec.yml"

  run bash "$RUNNER" run --repo-root "$repo" --no-file
  [ "$status" -eq 0 ]
  run bash "$RUNNER" run --repo-root "$repo" --no-file
  [ "$status" -eq 0 ]

  run bash -c "grep -c '^all-tests$' '$repo/sweep-command.log'"
  [ "$output" = "2" ]
  run yq -r '.tests.status' "$repo/.autospec/sweep/latest.json"
  [ "$output" = "pass" ]
}

@test "autospec-sweep run deploys before e2e command when tests require software" {
  repo="$(make_configured_repo)"
  cat > "$repo/deploy.sh" <<'SH'
#!/usr/bin/env bash
set -eu
printf 'deploy\n' >> sweep-command.log
SH
  cat > "$repo/e2e.sh" <<'SH'
#!/usr/bin/env bash
set -eu
printf 'e2e\n' >> sweep-command.log
SH
  chmod +x "$repo/deploy.sh" "$repo/e2e.sh"
  yq -i '.project.findings.commands.test = "true"' "$repo/.autospec/autospec.yml"
  yq -i '.project.findings.commands.e2e = "./e2e.sh"' "$repo/.autospec/autospec.yml"
  yq -i '.project.findings.commands.deploy = "./deploy.sh"' "$repo/.autospec/autospec.yml"

  run bash "$RUNNER" run --repo-root "$repo" --no-file

  [ "$status" -eq 0 ]
  run bash -c "tr '\n' ' ' < '$repo/sweep-command.log'"
  [ "$output" = "deploy e2e " ]
  run yq -r '.deployment.status' "$repo/.autospec/sweep/latest.json"
  [ "$output" = "pass" ]
}
