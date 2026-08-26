#!/usr/bin/env bats
# tests/unit/test_runtime_evidence_quality.bats — runtime evidence and product quality automation.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-evidence-v1-XXXXXX)"
  LAUNCH="$REPO_ROOT/scripts/autospec-detect-app-launch.sh"
  HARNESS="$REPO_ROOT/scripts/autospec-app-harness.sh"
  PW="$REPO_ROOT/scripts/autospec-run-playwright-evidence.sh"
  CONTACT="$REPO_ROOT/scripts/autospec-generate-screenshot-contact-sheet.sh"
  VISUAL="$REPO_ROOT/scripts/autospec-visual-polish-audit.sh"
  A11Y="$REPO_ROOT/scripts/autospec-accessibility-evidence-audit.sh"
  TUTORIAL="$REPO_ROOT/scripts/autospec-generate-tutorial-artifacts.sh"
  PDF="$REPO_ROOT/scripts/autospec-pdf-artifact-plan.sh"
  REPORTS="$REPO_ROOT/scripts/autospec-generate-report-artifacts.sh"
  REPORT_VALIDATE="$REPO_ROOT/scripts/autospec-validate-report-artifact.sh"
  SIM="$REPO_ROOT/scripts/autospec-simulate-ai-nlai.sh"
  TOKENS="$REPO_ROOT/scripts/autospec-token-usage-evidence.sh"
  BUNDLE="$REPO_ROOT/scripts/autospec-build-evidence-bundle.sh"
  SCORECARD="$REPO_ROOT/scripts/autospec-product-quality-scorecard.sh"
  EVIDENCE_STATUS="$REPO_ROOT/scripts/autospec-runtime-evidence-status.sh"
  WORKER="$REPO_ROOT/scripts/autospec-worker-one.sh"
  VERIFY="$REPO_ROOT/scripts/autospec-verify-worker-pr.sh"
  SUPERVISOR="$REPO_ROOT/scripts/autospec-supervisor-cycle.sh"
  COVERAGE="$REPO_ROOT/scripts/autospec-spec-coverage.sh"
  MVP="$REPO_ROOT/scripts/autospec-mvp-status.sh"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_evidence_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state/runtime-generations" "$repo/.autospec/reports" "$repo/.autospec/artifacts/playwright/screenshots" "$repo/docs/specs" "$repo/src"
  cat > "$repo/package.json" <<'JSON'
{"scripts":{"dev":"vite --host 127.0.0.1","test:evidence":"playwright test"},"dependencies":{"@vitejs/plugin-react":"latest","react":"latest","typescript":"latest"},"devDependencies":{"@playwright/test":"latest","vite":"latest"}}
JSON
  echo 'export const App = () => null;' > "$repo/src/App.tsx"
  # tsconfig.json pairs the package.json marker with typescript, so the single .tsx source line carries 100% line share, confidence 0.95 >= 0.8 scaffold gate (language-selection-axis spec, confidence clamp).
  echo '{}' > "$repo/tsconfig.json"
  cat > "$repo/.autospec/state/runtime-generations/in-app-docs-center.json" <<'JSON'
{"schema":1,"feature_id":"in-app-docs-center","runtime_claim_level":"shell","generated_files":["src/pages/Docs.tsx"],"status":"generated"}
JSON
  cat > "$repo/.autospec/state/feature-slices.json" <<'JSON'
{"schema":1,"feature_slices":[{"id":"in-app-docs-center","title":"In-app documentation center","category":"product","runtime_claim_level":"shell"},{"id":"ai-provider-settings","title":"AI provider settings shell","category":"ai","runtime_claim_level":"shell"},{"id":"ai-usage-token-dashboard","title":"AI usage token dashboard shell","category":"ai","runtime_claim_level":"shell"}]}
JSON
  cat > "$repo/.autospec/state/ai-capabilities.json" <<'JSON'
{"schema":1,"provider_secret_policy":"references_only_no_values","features":[{"feature_id":"ai-provider-settings"}]}
JSON
  cat > "$repo/.autospec/state/capability-registry.json" <<'JSON'
{"schema":1,"capabilities":[{"id":"docs.search","title":"Docs search"}]}
JSON
  printf 'fake screenshot mobile' > "$repo/.autospec/artifacts/playwright/screenshots/in-app-docs-center-320x640.png"
  printf 'fake screenshot desktop' > "$repo/.autospec/artifacts/playwright/screenshots/in-app-docs-center-1440x900.png"
}

@test "app launch detection and harness are dry-run safe and confirm mockable" {
  mkdir -p "$TEST_TMPDIR/repo" "$TEST_TMPDIR/no-launch"
  write_evidence_repo "$TEST_TMPDIR/repo"
  echo "# no launch" > "$TEST_TMPDIR/no-launch/README.md"

  run bash "$LAUNCH" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  run jq -r '.profiles[0].command + ":" + .profiles[0].url' "$TEST_TMPDIR/repo/.autospec/reports/app-launch-detection.json"
  [ "$output" = "npm run dev:http://localhost:5173" ]

  run bash "$LAUNCH" --repo-root "$TEST_TMPDIR/no-launch" --dry-run
  [ "$status" -eq 0 ]
  run jq -r '.profiles[0].blocked_reason' "$TEST_TMPDIR/no-launch/.autospec/reports/app-launch-detection.json"
  [ "$output" = "no launch command detected" ]

  run bash "$HARNESS" --repo-root "$TEST_TMPDIR/repo" --dry-run --profile web-dev-server
  [ "$status" -eq 0 ]
  run jq -r '.side_effects.started_process|tostring' "$TEST_TMPDIR/repo/.autospec/reports/app-harness-plan.json"
  [ "$output" = "false" ]

  run bash "$HARNESS" --repo-root "$TEST_TMPDIR/repo" --confirm --command "mock:server" --url http://localhost:5173 --timeout 1
  [ "$status" -eq 0 ]
  grep -q "stopped cleanly" "$TEST_TMPDIR/repo/.autospec/reports/app-harness-result.md"
}

@test "Playwright evidence plans, blocks missing tooling, and records viewport matrix" {
  mkdir -p "$TEST_TMPDIR/repo" "$TEST_TMPDIR/missing"
  write_evidence_repo "$TEST_TMPDIR/repo"
  write_evidence_repo "$TEST_TMPDIR/missing"
  cat > "$TEST_TMPDIR/missing/package.json" <<'JSON'
{"scripts":{"dev":"vite"},"dependencies":{"react":"latest"},"devDependencies":{"vite":"latest"}}
JSON

  run bash "$PW" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "320x640" "$TEST_TMPDIR/repo/.autospec/reports/playwright-evidence-run.md"

  run bash "$PW" --repo-root "$TEST_TMPDIR/missing" --confirm --feature in-app-docs-center
  [ "$status" -eq 0 ]
  run jq -r '.status' "$TEST_TMPDIR/missing/.autospec/reports/playwright-evidence-run.json"
  [ "$output" = "blocked_missing_playwright" ]
  compgen -G "$TEST_TMPDIR/missing/.autospec/backlog/evidence/*.md" >/dev/null
}

@test "screenshots contact sheet, visual polish, and accessibility audits report evidence gaps" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_evidence_repo "$TEST_TMPDIR/repo"

  run bash "$CONTACT" --repo-root "$TEST_TMPDIR/repo" --confirm --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "Missing viewports" "$TEST_TMPDIR/repo/.autospec/reports/screenshot-contact-sheet.md"
  compgen -G "$TEST_TMPDIR/repo/.autospec/artifacts/contact-sheets/*" >/dev/null

  run bash "$VISUAL" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "heuristic" "$TEST_TMPDIR/repo/.autospec/reports/visual-polish-audit.md"

  run bash "$A11Y" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "keyboard" "$TEST_TMPDIR/repo/.autospec/reports/accessibility-evidence-audit.md"
  [ -f "$TEST_TMPDIR/repo/.autospec/templates/accessibility/accessibility-test-plan.md" ]
}

@test "tutorial PDF and report artifacts are generated or planned without installing tools" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_evidence_repo "$TEST_TMPDIR/repo"
  bash "$CONTACT" --repo-root "$TEST_TMPDIR/repo" --confirm --feature in-app-docs-center >/dev/null

  run bash "$TUTORIAL" --repo-root "$TEST_TMPDIR/repo" --confirm --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "# In-app documentation center Tutorial" "$TEST_TMPDIR/repo/docs/tutorials/autospec-generated/in-app-docs-center.md"
  grep -q "narration" "$TEST_TMPDIR/repo/docs/tutorials/autospec-generated/scripts/in-app-docs-center-narration.md"

  run bash "$PDF" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  grep -q "PDF quality checklist" "$TEST_TMPDIR/repo/.autospec/reports/pdf-artifact-plan.md"

  run bash "$REPORTS" --repo-root "$TEST_TMPDIR/repo" --confirm --report runtime-feature-evidence-report
  [ "$status" -eq 0 ]
  run bash "$REPORT_VALIDATE" --repo-root "$TEST_TMPDIR/repo" --dry-run --report runtime-feature-evidence-report
  [ "$status" -eq 0 ]
  grep -q "limitations" "$TEST_TMPDIR/repo/.autospec/reports/report-artifact-validation.md"
}

@test "AI/NLAI simulation and token usage evidence are mock-only and backlog gaps" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_evidence_repo "$TEST_TMPDIR/repo"

  run bash "$SIM" --repo-root "$TEST_TMPDIR/repo" --confirm --mock-only --scenario rag-docs
  [ "$status" -eq 0 ]
  run jq -r '.external_api_calls' "$TEST_TMPDIR/repo/.autospec/reports/ai-nlai-simulation.json"
  [ "$output" = "false" ]
  grep -q "simulated" "$TEST_TMPDIR/repo/.autospec/reports/ai-nlai-simulation.md"

  run bash "$TOKENS" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  grep -q "per-user" "$TEST_TMPDIR/repo/.autospec/reports/token-usage-evidence.md"
  compgen -G "$TEST_TMPDIR/repo/.autospec/backlog/ai-token-usage/*.md" >/dev/null
}

@test "evidence bundle gathers artifacts, blocks secrets, and verifier references bundle" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_evidence_repo "$TEST_TMPDIR/repo"
  bash "$PW" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature in-app-docs-center >/dev/null
  bash "$CONTACT" --repo-root "$TEST_TMPDIR/repo" --confirm --feature in-app-docs-center >/dev/null
  bash "$VISUAL" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature in-app-docs-center >/dev/null
  bash "$A11Y" --repo-root "$TEST_TMPDIR/repo" --dry-run --feature in-app-docs-center >/dev/null
  bash "$TUTORIAL" --repo-root "$TEST_TMPDIR/repo" --confirm --feature in-app-docs-center >/dev/null
  bash "$SIM" --repo-root "$TEST_TMPDIR/repo" --confirm --mock-only --scenario rag-docs >/dev/null
  bash "$TOKENS" --repo-root "$TEST_TMPDIR/repo" --dry-run >/dev/null

  run bash "$BUNDLE" --repo-root "$TEST_TMPDIR/repo" --confirm --issue 7 --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "Autospec Evidence Bundle" "$TEST_TMPDIR/repo/.autospec/reports/evidence-bundle.md"

  cat > "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json" <<'JSON'
{"version":1,"processed_issue_id":"7","classification":"docs-only"}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/reports/worker-diff-review.json" <<'JSON'
{"patch_budget":{"passed":true},"forbidden_path_check":{"passed":true,"matches":[]},"files_changed":[],"test_docs_metadata_change_check":{}}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/reports/worker-pr-body.md" <<'MD'
# Worker

## Source issue
`7`

## Constitution/baseline references
Evidence bundle: `.autospec/reports/evidence-bundle.md`
MD
  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue 7
  [ "$status" -eq 0 ]
  grep -q "Evidence Bundle Review" "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.md"
}

@test "worker supervisor scorecard runtime evidence status and RC integrations report evidence readiness" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_evidence_repo "$TEST_TMPDIR/repo"

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue 3 --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "Evidence Bundle" "$TEST_TMPDIR/repo/.autospec/reports/worker-runtime-feature-result.md"

  run bash "$SUPERVISOR" --repo-root "$TEST_TMPDIR/repo" --dry-run --next
  [ "$status" -eq 0 ]
  grep -q "Evidence Readiness" "$TEST_TMPDIR/repo/.autospec/reports/supervisor-cycle-plan.md"

  run bash "$SCORECARD" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  grep -q "heuristic scorecard" "$TEST_TMPDIR/repo/.autospec/reports/product-quality-scorecard.md"

  run bash "$EVIDENCE_STATUS" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q "Runtime Evidence Status" "$TEST_TMPDIR/repo/.autospec/reports/runtime-evidence-status.md"

  run bash "$COVERAGE" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  grep -q "runtime evidence" "$TEST_TMPDIR/repo/.autospec/reports/spec-coverage.md"

  run bash "$MVP" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q "Runtime evidence status" "$TEST_TMPDIR/repo/.autospec/reports/mvp-status.md"
}
