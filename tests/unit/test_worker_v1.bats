#!/usr/bin/env bats
# tests/unit/test_worker_v1.bats — bounded low-risk code worker gates.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  WORKER="$REPO_ROOT/scripts/autospec-worker-v1.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-worker-v1-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_issue_fixture() {
  local repo="$1"
  local issue_id="$2"
  local title="$3"
  local risk="${4:-Low}"
  local labels_json="${5:-[\"autospec:managed\",\"autospec:discovered\"]}"
  local body="${6:-Small helper change. Tests: tests/unit/test_worker_v1.bats. Files: scripts/example.sh tests/unit/test_example.bats.}"
  mkdir -p "$repo/.autospec/reports" "$repo/.autospec/backlog/issues" "$repo/.autospec/templates"
  cat > "$repo/.autospec/reports/issue-plan.json" <<JSON
{
  "version": 1,
  "issues": [
    {
      "issue_id": "$issue_id",
      "title": "$title",
      "summary": "$body",
      "risk": "$risk",
      "suggested_labels": $labels_json,
      "source_gap": {"feature_family":"baseline","capability":"worker"},
      "source_reference": {"baseline":"web baseline","doctrine":"Testing Doctrine"},
      "evidence": ["fixture"],
      "confidence": 0.8,
      "priority": "High",
      "draft_path": ".autospec/backlog/issues/$issue_id.md",
      "implementation_scope": ["$body"],
      "non_goals": ["No broad refactor."],
      "acceptance_criteria": ["Focused validation passes."]
    }
  ]
}
JSON
  cat > "$repo/.autospec/backlog/issues/$issue_id.md" <<MD
# $title

## Summary
$body

## Implementation scope
- $body

## Acceptance criteria
- [ ] Focused validation passes.

## Validation
\`\`\`bash
bats tests/unit/test_worker_v1.bats
\`\`\`
MD
}

enable_code_mode() {
  local repo="$1"
  mkdir -p "$repo/.autospec"
  cat > "$repo/.autospec/autospec.yml" <<'YAML'
autonomy:
  worker:
    allow_code_changes: true
    code_change_mode: low_risk_only
    max_files_changed: 8
    max_code_files_changed: 4
    max_lines_changed: 300
    max_test_files_changed: 4
    max_new_dependencies: 0
    forbidden_paths:
      - .env
      - .env.*
      - "**/*secret*"
      - "**/*credential*"
      - "**/migrations/**"
      - ".github/workflows/**"
    require_tests_for_code: true
    require_validation: true
project:
  findings:
    commands:
      test: "bash scripts/validate.sh"
YAML
}

init_git_repo() {
  local repo="$1"
  git -C "$repo" init -q
  git -C "$repo" config user.email test@example.com
  git -C "$repo" config user.name "Test User"
  mkdir -p "$repo/scripts" "$repo/tests/unit"
  printf '#!/usr/bin/env bash\nprintf old\\n\n' > "$repo/scripts/example.sh"
  printf '#!/usr/bin/env bats\n@test \"old\" { true; }\n' > "$repo/tests/unit/test_example.bats"
  git -C "$repo" add .
  git -C "$repo" commit -q -m initial
}

@test "docs-only issue still follows non-code v0-compatible path" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_issue_fixture "$TEST_TMPDIR/repo" "001-docs-update-worker-runbook" "docs: update worker runbook" "Low" "[\"autospec:managed\",\"autospec:documentation\"]" "Update docs/worker-v1.md only."

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  run jq -r '.classification' "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "docs-only" ]
  run jq -r '.code_change_eligible' "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "false" ]
  grep -q '## Risk classification' "$TEST_TMPDIR/repo/.autospec/state/implementation-packet.md"
}

@test "low-risk code issue is eligible when code mode is enabled" {
  mkdir -p "$TEST_TMPDIR/repo"
  enable_code_mode "$TEST_TMPDIR/repo"
  write_issue_fixture "$TEST_TMPDIR/repo" "001-fix-report-formatting" "fix: improve report formatting helper" "Low"

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  run jq -r '.classification' "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "low-risk-code" ]
  run jq -r '.code_change_eligible' "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "true" ]
}

@test "low-risk code issue is refused when allow_code_changes is false by default" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_issue_fixture "$TEST_TMPDIR/repo" "001-fix-report-formatting" "fix: improve report formatting helper" "Low"

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  run jq -r '.classification' "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "needs-guidance" ]
  grep -q 'allow_code_changes is false' "$TEST_TMPDIR/repo/.autospec/reports/worker-stuck-handoff.md"
}

@test "auth and migration and dependency issues are refused as high-risk or unsupported" {
  mkdir -p "$TEST_TMPDIR/auth" "$TEST_TMPDIR/migration" "$TEST_TMPDIR/dependency"
  enable_code_mode "$TEST_TMPDIR/auth"
  enable_code_mode "$TEST_TMPDIR/migration"
  enable_code_mode "$TEST_TMPDIR/dependency"
  write_issue_fixture "$TEST_TMPDIR/auth" "001-auth-change" "feat: change authorization permissions" "High"
  write_issue_fixture "$TEST_TMPDIR/migration" "001-db-migration" "feat: add database migration" "High"
  write_issue_fixture "$TEST_TMPDIR/dependency" "001-upgrade-framework" "chore: major dependency upgrade" "High"

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/auth" --dry-run
  [ "$status" -eq 0 ]
  run jq -r '.classification' "$TEST_TMPDIR/auth/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "high-risk-code" ]

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/migration" --dry-run
  [ "$status" -eq 0 ]
  run jq -r '.classification' "$TEST_TMPDIR/migration/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "high-risk-code" ]

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/dependency" --dry-run
  [ "$status" -eq 0 ]
  run jq -r '.classification' "$TEST_TMPDIR/dependency/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "unsupported" ]
}

@test "patch budget exceeded and forbidden path changes produce stuck handoffs" {
  mkdir -p "$TEST_TMPDIR/repo"
  enable_code_mode "$TEST_TMPDIR/repo"
  write_issue_fixture "$TEST_TMPDIR/repo" "001-fix-report-formatting" "fix: improve report formatting helper" "Low"
  init_git_repo "$TEST_TMPDIR/repo"
  printf 'token=secret\n' > "$TEST_TMPDIR/repo/.env"
  python3 - <<PY
from pathlib import Path
p=Path("$TEST_TMPDIR/repo/scripts/example.sh")
p.write_text("#!/usr/bin/env bash\\n" + "\\n".join(f"echo {i}" for i in range(400)) + "\\n")
PY

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  run jq -r '.pr_creation_allowed' "$TEST_TMPDIR/repo/.autospec/reports/worker-diff-review.json"
  [ "$output" = "false" ]
  grep -q '.env' "$TEST_TMPDIR/repo/.autospec/reports/worker-stuck-handoff.md"
  grep -q 'patch budget' "$TEST_TMPDIR/repo/.autospec/reports/worker-stuck-handoff.md"
}

@test "no test plan produces needs-guidance for low-risk code" {
  mkdir -p "$TEST_TMPDIR/repo"
  enable_code_mode "$TEST_TMPDIR/repo"
  write_issue_fixture "$TEST_TMPDIR/repo" "001-fix-helper" "fix: improve helper parser" "Low" "[\"autospec:managed\"]" "Change scripts/example.sh. No test path is named."
  cat > "$TEST_TMPDIR/repo/.autospec/backlog/issues/001-fix-helper.md" <<'MD'
# fix: improve helper parser

## Summary
Change scripts/example.sh. No focused validation path is named.

## Acceptance criteria
- [ ] Helper parser behavior is corrected.
MD

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  run jq -r '.classification' "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "needs-guidance" ]
  grep -q 'No focused test path' "$TEST_TMPDIR/repo/.autospec/reports/worker-stuck-handoff.md"
}

@test "validation plan and diff review are generated for bounded code diff" {
  mkdir -p "$TEST_TMPDIR/repo"
  enable_code_mode "$TEST_TMPDIR/repo"
  write_issue_fixture "$TEST_TMPDIR/repo" "001-fix-report-formatting" "fix: improve report formatting helper" "Low"
  init_git_repo "$TEST_TMPDIR/repo"
  printf '#!/usr/bin/env bash\nprintf new\\n\n' > "$TEST_TMPDIR/repo/scripts/example.sh"
  printf '#!/usr/bin/env bats\n@test \"new\" { true; }\n' > "$TEST_TMPDIR/repo/tests/unit/test_example.bats"

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/worker-validation-plan.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/worker-diff-review.md" ]
  run jq -r '.focused_validation[0]' "$TEST_TMPDIR/repo/.autospec/reports/worker-validation-plan.json"
  [[ "$output" == *"bats tests/unit/test_example.bats"* ]]
  run jq -r '.pr_creation_allowed' "$TEST_TMPDIR/repo/.autospec/reports/worker-diff-review.json"
  [ "$output" = "true" ]
}

@test "PR body evidence includes risk validation diff and processes exactly one issue" {
  mkdir -p "$TEST_TMPDIR/repo"
  enable_code_mode "$TEST_TMPDIR/repo"
  write_issue_fixture "$TEST_TMPDIR/repo" "001-fix-report-formatting" "fix: improve report formatting helper" "Low"
  python3 - <<PY
import json
p="$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json"
d=json.load(open(p))
d["issues"].append(dict(d["issues"][0], issue_id="002-fix-other-helper", title="fix: improve other helper", draft_path=".autospec/backlog/issues/002-fix-other-helper.md"))
json.dump(d, open(p, "w"), indent=2)
PY
  init_git_repo "$TEST_TMPDIR/repo"
  printf '#!/usr/bin/env bash\nprintf new\\n\n' > "$TEST_TMPDIR/repo/scripts/example.sh"
  printf '#!/usr/bin/env bats\n@test \"new\" { true; }\n' > "$TEST_TMPDIR/repo/tests/unit/test_example.bats"

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/worker-pr-body.md" ]
  grep -q '## Risk classification' "$TEST_TMPDIR/repo/.autospec/reports/worker-pr-body.md"
  grep -q '## Validation evidence' "$TEST_TMPDIR/repo/.autospec/reports/worker-pr-body.md"
  grep -q '## Diff safety review' "$TEST_TMPDIR/repo/.autospec/reports/worker-pr-body.md"
  run jq -r '.processed_issue_id' "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "001-fix-report-formatting" ]
}
