#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd -P)"
  TEST_TMPDIR="$(mktemp -d "${TMPDIR:-/tmp}/autospec-v3-XXXXXX")"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_ui_runtime_fixture() {
  local repo="$1"
  mkdir -p "$repo/.autospec/reports" "$repo/.autospec/state/runtime-generations" "$repo/.autospec/state" "$repo/src"
  cat > "$repo/.autospec/reports/runtime-generation-result.json" <<'JSON'
{"adapter":"react-vite","feature_id":"in-app-docs-center","generated_files":["src/Docs.tsx"],"runtime_claim_level":"shell","schema":1,"status":"planned"}
JSON
  cat > "$repo/.autospec/reports/evidence-bundle.json" <<'JSON'
{"feature":"in-app-docs-center","findings":[],"reports":[".autospec/reports/playwright-evidence-run.json"],"schema":1}
JSON
  cat > "$repo/.autospec/reports/verifier-report.json" <<'JSON'
{"verdict":"pass_with_warnings","dimensions":[{"dimension":"runtime_claim_honesty","status":"pass"}],"schema":1}
JSON
  cat > "$repo/.autospec/reports/worker-risk-classification.json" <<'JSON'
{"classification":"medium-risk-code","processed_issue_id":"42","version":1}
JSON
  cat > "$repo/.autospec/state/feature-slices.json" <<'JSON'
{"feature_slices":[{"category":"product","id":"in-app-docs-center","runtime_claim_level":"shell","title":"In-app docs center"}],"schema":1}
JSON
  cat > "$repo/.autospec/state/digital-twin.json" <<'JSON'
{"schema":1,"summary":{"application_type":"web"}}
JSON
}

write_ai_fixture() {
  local repo="$1"
  write_ui_runtime_fixture "$repo"
  cat > "$repo/.autospec/reports/runtime-generation-result.json" <<'JSON'
{"adapter":"react-vite","feature_id":"ai-provider-settings","generated_files":["src/AiSettings.tsx"],"runtime_claim_level":"shell","schema":1,"status":"planned"}
JSON
  cat > "$repo/.autospec/reports/evidence-bundle.json" <<'JSON'
{"feature":"ai-provider-settings","findings":[],"reports":[".autospec/reports/ai-nlai-simulation.json"],"schema":1}
JSON
  cat > "$repo/.autospec/state/feature-slices.json" <<'JSON'
{"feature_slices":[{"category":"ai","id":"ai-provider-settings","runtime_claim_level":"shell","title":"AI provider settings"}],"schema":1}
JSON
}

@test "specialist registry loads roles and maps ownership" {
  mkdir -p "$TEST_TMPDIR/repo"

  run bash "$REPO_ROOT/scripts/autospec-specialist-index.sh" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q "Autospec Specialist Agents" "$TEST_TMPDIR/repo/.autospec/reports/specialist-agents.md"
  grep -q "security-engineer" "$TEST_TMPDIR/repo/.autospec/reports/specialist-agents.json"
  grep -q "Rule/category ownership" "$TEST_TMPDIR/repo/.autospec/reports/specialist-agents.md"
}

@test "specialist assignment and review packets cover UI and AI runtime work" {
  mkdir -p "$TEST_TMPDIR/ui" "$TEST_TMPDIR/ai"
  write_ui_runtime_fixture "$TEST_TMPDIR/ui"
  write_ai_fixture "$TEST_TMPDIR/ai"

  bash "$REPO_ROOT/scripts/autospec-specialist-index.sh" --repo-root "$TEST_TMPDIR/ui" >/dev/null
  run bash "$REPO_ROOT/scripts/autospec-assign-specialists.sh" --repo-root "$TEST_TMPDIR/ui" --dry-run --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "ux-designer" "$TEST_TMPDIR/ui/.autospec/reports/specialist-assignment.json"
  grep -q "accessibility-specialist" "$TEST_TMPDIR/ui/.autospec/reports/specialist-assignment.json"
  grep -q "qa-engineer" "$TEST_TMPDIR/ui/.autospec/reports/specialist-assignment.json"

  run bash "$REPO_ROOT/scripts/autospec-specialist-review-packets.sh" --repo-root "$TEST_TMPDIR/ui" --confirm --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "Specialist Review Packet" "$TEST_TMPDIR/ui/.autospec/state/specialist-reviews/feature-in-app-docs-center/ux-designer.md"
  grep -q "Evidence bundle" "$TEST_TMPDIR/ui/.autospec/state/specialist-reviews/feature-in-app-docs-center/qa-engineer.md"

  bash "$REPO_ROOT/scripts/autospec-specialist-index.sh" --repo-root "$TEST_TMPDIR/ai" >/dev/null
  run bash "$REPO_ROOT/scripts/autospec-assign-specialists.sh" --repo-root "$TEST_TMPDIR/ai" --dry-run --feature ai-provider-settings
  [ "$status" -eq 0 ]
  grep -q "ai-engineer" "$TEST_TMPDIR/ai/.autospec/reports/specialist-assignment.json"
  grep -q "security-engineer" "$TEST_TMPDIR/ai/.autospec/reports/specialist-assignment.json"
  grep -q "privacy-engineer" "$TEST_TMPDIR/ai/.autospec/reports/specialist-assignment.json"
}

@test "specialist review findings and quorum block missing security evidence" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_ai_fixture "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/reports/evidence-bundle.json" <<'JSON'
{"feature":"ai-provider-settings","findings":["secret evidence missing"],"reports":[],"schema":1}
JSON

  bash "$REPO_ROOT/scripts/autospec-specialist-index.sh" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  bash "$REPO_ROOT/scripts/autospec-assign-specialists.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature ai-provider-settings >/dev/null
  bash "$REPO_ROOT/scripts/autospec-specialist-review-packets.sh" --repo-root "$TEST_TMPDIR/repo" --confirm --feature ai-provider-settings >/dev/null

  run bash "$REPO_ROOT/scripts/autospec-run-specialist-review.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature ai-provider-settings
  [ "$status" -eq 0 ]
  grep -q "security-engineer" "$TEST_TMPDIR/repo/.autospec/reports/specialist-review-result.json"
  grep -q "secret_handling" "$TEST_TMPDIR/repo/.autospec/reports/specialist-review-result.json"

  run bash "$REPO_ROOT/scripts/autospec-review-quorum.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature ai-provider-settings
  [ "$status" -ne 0 ]
  grep -q "blocked" "$TEST_TMPDIR/repo/.autospec/reports/review-quorum.md"
}

@test "promotion gate consumes review quorum and never merges or approves" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_ai_fixture "$TEST_TMPDIR/repo"
  mkdir -p "$TEST_TMPDIR/repo/.autospec/reports"
  cat > "$TEST_TMPDIR/repo/.autospec/reports/review-quorum.json" <<'JSON'
{"schema":1,"verdict":"blocked","blocking_findings":[{"specialist":"security-engineer"}]}
JSON

  run bash "$REPO_ROOT/scripts/autospec-promote-pr.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --pr 9
  [ "$status" -ne 0 ]
  grep -q "Review Quorum" "$TEST_TMPDIR/repo/.autospec/reports/promotion-plan.md"
  grep -q "merged\": false" "$TEST_TMPDIR/repo/.autospec/reports/promotion-plan.json"
  grep -q "approved\": false" "$TEST_TMPDIR/repo/.autospec/reports/promotion-plan.json"
}

@test "medium-risk planning and guidance create plans but no implementation" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_ui_runtime_fixture "$TEST_TMPDIR/repo"

  run bash "$REPO_ROOT/scripts/autospec-medium-risk-plan.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue 42
  [ "$status" -eq 0 ]
  grep -q "Medium-Risk Implementation Plan" "$TEST_TMPDIR/repo/.autospec/reports/medium-risk-plan.md"
  grep -q "Human decisions needed" "$TEST_TMPDIR/repo/.autospec/reports/medium-risk-plan.md"
  [ ! -f "$TEST_TMPDIR/repo/src/api.ts" ]

  run bash "$REPO_ROOT/scripts/autospec-build-guidance-request.sh" --repo-root "$TEST_TMPDIR/repo" --confirm --issue 42
  [ "$status" -eq 0 ]
  grep -q "Autospec Guidance Request" "$TEST_TMPDIR/repo/.autospec/state/guidance-requests/issue-42.md"
  grep -q "Resume criteria" "$TEST_TMPDIR/repo/.autospec/reports/guidance-request.md"
}

@test "IDR learning proposals retrospective memory and repeated miss reports are deterministic" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/reports"
  cat > "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json" <<'JSON'
{"verdict":"needs_changes","dimensions":[{"dimension":"test_evidence","status":"fail","summary":"Missing focused test"}],"schema":1}
JSON

  run bash "$REPO_ROOT/scripts/autospec-record-implementation-decision.sh" --repo-root "$TEST_TMPDIR/repo" --confirm --issue 7
  [ "$status" -eq 0 ]
  grep -q "IDR:" "$TEST_TMPDIR/repo/docs/idrs/"*.md

  run bash "$REPO_ROOT/scripts/autospec-update-learning-ledger.sh" --repo-root "$TEST_TMPDIR/repo" --confirm --from-issue 7
  [ "$status" -eq 0 ]
  grep -q "Missing focused test" "$TEST_TMPDIR/repo/.autospec/reports/learning-ledger.md"

  run bash "$REPO_ROOT/scripts/autospec-policy-improvement-proposals.sh" --repo-root "$TEST_TMPDIR/repo" --confirm
  [ "$status" -eq 0 ]
  grep -q "Policy Improvement Proposal" "$TEST_TMPDIR/repo/.autospec/proposals/policy/"*.md

  run bash "$REPO_ROOT/scripts/autospec-retrospective.sh" --repo-root "$TEST_TMPDIR/repo" --confirm
  [ "$status" -eq 0 ]
  grep -q "Autospec Retrospective" "$TEST_TMPDIR/repo/.autospec/reports/retrospective.md"

  run bash "$REPO_ROOT/scripts/autospec-build-memory-index.sh" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q "policy_gap" "$TEST_TMPDIR/repo/.autospec/reports/memory-index.md"

  run bash "$REPO_ROOT/scripts/autospec-plan-repeated-miss-issues.sh" --repo-root "$TEST_TMPDIR/repo" --confirm
  [ "$status" -eq 0 ]
  grep -q "repeated miss" "$TEST_TMPDIR/repo/.autospec/reports/repeated-miss-issue-plan.md"
}

@test "council supervisor statuses spec coverage and guide expose v3 governance" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_ui_runtime_fixture "$TEST_TMPDIR/repo"
  bash "$REPO_ROOT/scripts/autospec-specialist-index.sh" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  bash "$REPO_ROOT/scripts/autospec-assign-specialists.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature in-app-docs-center >/dev/null
  bash "$REPO_ROOT/scripts/autospec-specialist-review-packets.sh" --repo-root "$TEST_TMPDIR/repo" --confirm --feature in-app-docs-center >/dev/null
  bash "$REPO_ROOT/scripts/autospec-run-specialist-review.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature in-app-docs-center >/dev/null
  bash "$REPO_ROOT/scripts/autospec-review-quorum.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature in-app-docs-center >/dev/null

  run bash "$REPO_ROOT/scripts/autospec-council-report.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "Autospec Council Report" "$TEST_TMPDIR/repo/.autospec/reports/council-report.md"

  run bash "$REPO_ROOT/scripts/autospec-supervisor-cycle.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --next
  [ "$status" -eq 0 ]
  grep -q "Specialist Plan" "$TEST_TMPDIR/repo/.autospec/reports/supervisor-cycle-plan.md"
  grep -q "Review Quorum Requirements" "$TEST_TMPDIR/repo/.autospec/reports/supervisor-cycle-plan.md"

  run bash "$REPO_ROOT/scripts/autospec-autonomy-v3-status.sh" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q "Autospec Autonomy v3 Status" "$TEST_TMPDIR/repo/.autospec/reports/autonomy-v3-status.md"

  run bash "$REPO_ROOT/scripts/autospec-spec-coverage.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  grep -q "specialist" "$TEST_TMPDIR/repo/.autospec/reports/spec-coverage.md"

  grep -q "review quorum" "$REPO_ROOT/skills/autospec-guide/SKILL.md"
  grep -q "never bypass quorum" "$REPO_ROOT/skills/autospec-guide/SKILL.md"
}
