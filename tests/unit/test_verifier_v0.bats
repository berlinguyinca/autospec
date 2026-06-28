#!/usr/bin/env bats
# tests/unit/test_verifier_v0.bats — independent worker-output verifier.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  VERIFY="$REPO_ROOT/scripts/autospec-verify-worker-pr.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-verifier-v0-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_worker_artifacts() {
  local repo="$1"
  local classification="${2:-low-risk-code}"
  local validation_status="${3:-pass}"
  local forbidden="${4:-false}"
  local pr_allowed="${5:-true}"
  mkdir -p "$repo/.autospec/reports" "$repo/.autospec/state" "$repo/.autospec/backlog/issues"
  cat > "$repo/.autospec/reports/issue-plan.json" <<'JSON'
{
  "version": 1,
  "issues": [
    {
      "issue_id": "001-fix-report-formatting",
      "title": "fix: improve report formatting helper",
      "suggested_labels": ["autospec:managed", "autospec:discovered"],
      "source_reference": {"baseline":"web baseline","doctrine":"Testing Doctrine"},
      "source_gap": {"capability":"worker", "feature_family":"baseline"},
      "draft_path": ".autospec/backlog/issues/001-fix-report-formatting.md",
      "acceptance_criteria": ["Focused validation passes."]
    }
  ]
}
JSON
  cat > "$repo/.autospec/backlog/issues/001-fix-report-formatting.md" <<'MD'
# fix: improve report formatting helper

## Source
Baseline: web baseline
Doctrine: Testing Doctrine

## Acceptance criteria
- [ ] Focused validation passes.
MD
  cat > "$repo/.autospec/state/implementation-packet.md" <<'MD'
# Implementation packet: fix: improve report formatting helper

## Risk classification
low-risk-code

## Source
Baseline: web baseline
Doctrine: Testing Doctrine
Source gap: worker

## Test-first plan
- Update `tests/unit/test_example.bats`.
MD
  cat > "$repo/.autospec/reports/worker-risk-classification.json" <<JSON
{"version":1,"processed_issue_id":"001-fix-report-formatting","classification":"$classification","code_change_eligible":true,"classification_reasons":["fixture"]}
JSON
  cat > "$repo/.autospec/reports/worker-validation-plan.json" <<'JSON'
{"version":1,"focused_validation":["bats tests/unit/test_example.bats"],"full_validation":["bash scripts/validate.sh"],"skipped_validation":[],"validation_failures":[]}
JSON
  if [ "$validation_status" = "missing" ]; then
    rm -f "$repo/.autospec/reports/worker-validation.json"
  else
    cat > "$repo/.autospec/reports/worker-validation.json" <<JSON
{"version":1,"focused":[{"command":"bats tests/unit/test_example.bats","exit_code":0}],"full":[{"command":"bash scripts/validate.sh","exit_code":0}],"skipped":[],"status":"$validation_status"}
JSON
  fi
  cat > "$repo/.autospec/reports/worker-diff-review.json" <<JSON
{
  "version": 1,
  "files_changed": [
    {"path":"scripts/example.sh","added":2,"removed":1},
    {"path":"tests/unit/test_example.bats","added":2,"removed":1}
  ],
  "forbidden_path_check": {"passed": $([ "$forbidden" = "true" ] && printf false || printf true), "matches": $([ "$forbidden" = "true" ] && printf '[{"path":".env","patterns":[".env"]}]' || printf '[]')},
  "patch_budget": {"passed": $([ "$pr_allowed" = "true" ] && printf true || printf false), "failures": $([ "$pr_allowed" = "true" ] && printf '[]' || printf '["max_lines_changed patch budget exceeded"]')},
  "test_docs_metadata_change_check": {"test_files":["tests/unit/test_example.bats"],"code_files":["scripts/example.sh"],"expected_files":["scripts/example.sh","tests/unit/test_example.bats"]},
  "risk_change": {"planned":"$classification","actual":"$classification"},
  "pr_creation_allowed": $([ "$pr_allowed" = "true" ] && printf true || printf false)
}
JSON
  cat > "$repo/.autospec/reports/baseline-composition.json" <<'JSON'
{"version":1,"baselines":{"requested_profiles":["web"]}}
JSON
  cat > "$repo/.autospec/reports/baseline-gap-analysis.json" <<'JSON'
{"version":1,"matrix":[{"capability":"worker","feature_family":"baseline","status":"missing"}]}
JSON
  cat > "$repo/.autospec/reports/constitutional-gap-report.json" <<'JSON'
{"version":1,"sections":{"testing_gaps":{"status":"gap","summary":"Testing evidence needed."}}}
JSON
}

write_pr_body() {
  local repo="$1"
  cat > "$repo/pr-body.md" <<'MD'
## Summary
Improves report formatting helper.

## Source issue
001-fix-report-formatting

## Constitution/baseline references
Baseline: web baseline
Doctrine: Testing Doctrine
Source gap: worker

## Implementation mode
low-risk-code

## Risk classification
low-risk-code

## Patch budget
pass

## Test-first plan
bats tests/unit/test_example.bats

## Files changed
| File | Purpose |
| --- | --- |
| scripts/example.sh | helper |
| tests/unit/test_example.bats | focused test |

## Validation
bats tests/unit/test_example.bats

## Evidence artifacts
.autospec/reports/worker-diff-review.md

## Diff safety review
forbidden paths: none

## Safety notes
low-risk helper only

## Follow-up issues
none
MD
}

install_gh_stub() {
  local bin="$1"
  local log="$2"
  local body_file="$3"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "--repo" ]; then shift 2; fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  python3 - "$GH_PR_BODY" <<'PY'
import json, sys
body=open(sys.argv[1]).read()
print(json.dumps({
  "title":"fix: improve report formatting helper",
  "body":body,
  "headRefName":"feat/fix-report-formatting",
  "labels":[{"name":"autospec:managed"}],
  "files":[{"path":"scripts/example.sh"},{"path":"tests/unit/test_example.bats"}],
  "statusCheckRollup":[{"name":"focused","conclusion":"SUCCESS"}]
}))
PY
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "diff" ]; then
  printf 'diff --git a/scripts/example.sh b/scripts/example.sh\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "comment" ]; then
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  exit 0
fi
printf 'unexpected gh call: %s\n' "$*" >&2
exit 1
SH
  chmod +x "$bin/gh"
  : > "$log"
}

@test "verifier passes docs-only work item with required evidence" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_worker_artifacts "$TEST_TMPDIR/repo" "docs-only" "pass" "false" "true"
  write_pr_body "$TEST_TMPDIR/repo"

  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --work-item "$TEST_TMPDIR/repo/.autospec/state"

  [ "$status" -eq 0 ]
  run jq -r '.verdict' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "pass" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/verifications/work-item.json" ]
  grep -q '| Dimension | Status |' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.md"
}

@test "verifier passes low-risk code PR with tests and validation" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_worker_artifacts "$TEST_TMPDIR/repo" "low-risk-code" "pass" "false" "true"
  write_pr_body "$TEST_TMPDIR/repo"
  install_gh_stub "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log" "$TEST_TMPDIR/repo/pr-body.md"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" GH_PR_BODY="$TEST_TMPDIR/repo/pr-body.md" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --pr 7 --repo example/repo

  [ "$status" -eq 0 ]
  run jq -r '.verdict' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "pass" ]
  ! grep -q 'pr comment' "$TEST_TMPDIR/gh.log"
  ! grep -Eq 'pr (review|merge)' "$TEST_TMPDIR/gh.log"
}

@test "verifier fails missing validation for code changes" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_worker_artifacts "$TEST_TMPDIR/repo" "low-risk-code" "missing" "false" "true"
  write_pr_body "$TEST_TMPDIR/repo"

  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue 1

  [ "$status" -eq 1 ]
  run jq -r '.verdict' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "needs_changes" ]
  run jq -r '.dimensions[] | select(.dimension=="validation_evidence") | .status' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "fail" ]
}

@test "verifier blocks forbidden path and exceeded patch budget" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_worker_artifacts "$TEST_TMPDIR/repo" "low-risk-code" "pass" "true" "false"
  write_pr_body "$TEST_TMPDIR/repo"

  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue 1

  [ "$status" -eq 1 ]
  run jq -r '.verdict' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "blocked" ]
  grep -q 'forbidden' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.md"
}

@test "verifier flags missing PR body sections and traceability" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_worker_artifacts "$TEST_TMPDIR/repo" "low-risk-code" "pass" "false" "true"
  printf '## Summary\nNo traceability.\n' > "$TEST_TMPDIR/repo/pr-body.md"
  install_gh_stub "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log" "$TEST_TMPDIR/repo/pr-body.md"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" GH_PR_BODY="$TEST_TMPDIR/repo/pr-body.md" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --pr 7 --repo example/repo

  [ "$status" -eq 1 ]
  run jq -r '.verdict' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "needs_changes" ]
  run jq -r '.dimensions[] | select(.dimension=="pr_body_completeness") | .status' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "fail" ]
}

@test "verifier flags issue mismatch as needs guidance" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_worker_artifacts "$TEST_TMPDIR/repo" "low-risk-code" "pass" "false" "true"
  write_pr_body "$TEST_TMPDIR/repo"
  python3 - "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json" <<'PY'
import json, sys
p=sys.argv[1]
d=json.load(open(p))
d["processed_issue_id"]="999-other"
json.dump(d, open(p,"w"), indent=2)
PY

  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue 1

  [ "$status" -eq 1 ]
  run jq -r '.verdict' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "needs_guidance" ]
}

@test "verifier verdict logic covers warnings and unexpected risk paths" {
  mkdir -p "$TEST_TMPDIR/warn" "$TEST_TMPDIR/risk"
  write_worker_artifacts "$TEST_TMPDIR/warn" "docs-only" "missing" "false" "true"
  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/warn" --dry-run --work-item "$TEST_TMPDIR/warn/.autospec/state"
  [ "$status" -eq 0 ]
  run jq -r '.verdict' "$TEST_TMPDIR/warn/.autospec/reports/verifier-report.json"
  [ "$output" = "pass_with_warnings" ]

  write_worker_artifacts "$TEST_TMPDIR/risk" "low-risk-code" "pass" "false" "true"
  python3 - "$TEST_TMPDIR/risk/.autospec/reports/worker-diff-review.json" <<'PY'
import json, sys
p=sys.argv[1]
d=json.load(open(p))
d["files_changed"]=[{"path":"src/auth/login.py","added":1,"removed":0}]
json.dump(d, open(p,"w"), indent=2)
PY
  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/risk" --dry-run --issue 1
  [ "$status" -eq 1 ]
  run jq -r '.verdict' "$TEST_TMPDIR/risk/.autospec/reports/verifier-report.json"
  [ "$output" = "blocked" ]
}

@test "verifier confirm comments on PR but never approves or merges" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_worker_artifacts "$TEST_TMPDIR/repo" "low-risk-code" "pass" "false" "true"
  write_pr_body "$TEST_TMPDIR/repo"
  install_gh_stub "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log" "$TEST_TMPDIR/repo/pr-body.md"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" GH_PR_BODY="$TEST_TMPDIR/repo/pr-body.md" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --confirm --pr 7 --repo example/repo

  [ "$status" -eq 0 ]
  grep -q 'pr comment 7 --body-file' "$TEST_TMPDIR/gh.log"
  ! grep -Eq 'pr (review|merge)' "$TEST_TMPDIR/gh.log"
}

@test "verifier warns when architecture or high-risk work lacks ADR evidence" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_worker_artifacts "$TEST_TMPDIR/repo" "architecture-required" "pass" "false" "true"
  write_pr_body "$TEST_TMPDIR/repo"

  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue 1

  [ "$status" -eq 0 ]
  run jq -r '.dimensions[] | select(.dimension=="architecture_governance") | .status' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "warn" ]
  grep -q "Architecture Governance" "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.md"
}
