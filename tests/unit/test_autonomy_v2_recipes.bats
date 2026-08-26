#!/usr/bin/env bats
# tests/unit/test_autonomy_v2_recipes.bats — bounded recipe-backed autonomy.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-autonomy-v2-XXXXXX)"
  STACK="$REPO_ROOT/scripts/autospec-detect-stack-profile.sh"
  RECIPES="$REPO_ROOT/scripts/autospec-recipe-index.sh"
  PLAN="$REPO_ROOT/scripts/autospec-rule-to-recipe-plan.sh"
  DECOMP="$REPO_ROOT/scripts/autospec-decompose-implementation.sh"
  PATCH="$REPO_ROOT/scripts/autospec-build-patch-plan.sh"
  APPLY="$REPO_ROOT/scripts/autospec-apply-template.sh"
  WORKER="$REPO_ROOT/scripts/autospec-worker-one.sh"
  RECHECK="$REPO_ROOT/scripts/autospec-rule-recheck.sh"
  EVIDENCE="$REPO_ROOT/scripts/autospec-generate-evidence-tests.sh"
  VERIFY="$REPO_ROOT/scripts/autospec-verify-worker-pr.sh"
  SUPERVISOR="$REPO_ROOT/scripts/autospec-supervisor-cycle.sh"
  STATUS="$REPO_ROOT/scripts/autospec-autonomy-v2-status.sh"
  COVERAGE="$REPO_ROOT/scripts/autospec-spec-coverage.sh"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_react_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state" "$repo/.autospec/reports" "$repo/src/components" "$repo/tests/e2e" "$repo/docs"
  cat > "$repo/package.json" <<'JSON'
{"scripts":{"test":"vitest"},"dependencies":{"@vitejs/plugin-react":"latest","react":"latest","typescript":"latest"},"devDependencies":{"@playwright/test":"latest","vite":"latest"}}
JSON
  echo 'export const App = () => null' > "$repo/src/App.tsx"
  cat > "$repo/.autospec/state/effective-rules.json" <<'JSON'
{"schema":1,"rules":[{"rule_id":"testing.playwright.viewport","title":"Viewport matrix","category":"testing","severity":"required","resolution":"active","check_type":"required_playwright_viewport_matrix"}]}
JSON
  cat > "$repo/.autospec/state/rule-check-results.json" <<'JSON'
{"schema":1,"results":[{"rule_id":"testing.playwright.viewport","title":"Viewport matrix","status":"fail","severity":"required","category":"testing","check_type":"required_playwright_viewport_matrix","missing_evidence":["viewport matrix"],"evidence":[]}]}
JSON
  cp "$repo/.autospec/state/rule-check-results.json" "$repo/.autospec/reports/rule-check-results.json"
  cat > "$repo/.autospec/state/quality-gates.json" <<'JSON'
{"schema":1,"quality_gates":[]}
JSON
  cat > "$repo/.autospec/state/digital-twin.json" <<'JSON'
{"schema":1,"summary":{"application_type":"web"}}
JSON
  cat > "$repo/.autospec/state/capability-registry.json" <<'JSON'
{"schema":1,"capabilities":[]}
JSON
}

@test "worker capability registry and recipe index load safe defaults" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_react_repo "$TEST_TMPDIR/repo"

  run bash "$RECIPES" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/worker-capabilities.yml" ]
  grep -q "playwright_scaffold" "$TEST_TMPDIR/repo/.autospec/reports/worker-capabilities.md"
  run jq -r '.recipes[] | select(.id=="playwright-viewport-matrix") | .status' "$TEST_TMPDIR/repo/.autospec/state/implementation-recipes.json"
  [ "$output" = "supported" ]
}

@test "stack profile detection identifies a javascript primary and a low-confidence unknown stack" {
  mkdir -p "$TEST_TMPDIR/react" "$TEST_TMPDIR/unknown"
  write_react_repo "$TEST_TMPDIR/react"
  mkdir -p "$TEST_TMPDIR/unknown/.autospec/state" "$TEST_TMPDIR/unknown/.autospec/reports"
  echo "# unknown" > "$TEST_TMPDIR/unknown/README.md"

  run bash "$STACK" --repo-root "$TEST_TMPDIR/react"
  [ "$status" -eq 0 ]
  run jq -r '.primary_profile.id' "$TEST_TMPDIR/react/.autospec/state/stack-profile.json"
  [ "$output" = "javascript" ]

  run bash "$STACK" --repo-root "$TEST_TMPDIR/unknown"
  [ "$status" -eq 0 ]
  run jq -r '.primary_profile.confidence < 0.5' "$TEST_TMPDIR/unknown/.autospec/state/stack-profile.json"
  [ "$output" = "true" ]
}

@test "failed rule maps to recipe and disabled capability is reported" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_react_repo "$TEST_TMPDIR/repo"
  bash "$RECIPES" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  run bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" --dry-run --rule testing.playwright.viewport
  [ "$status" -eq 0 ]
  run jq -r '.plans[] | select(.rule_id=="testing.playwright.viewport") | .status' "$TEST_TMPDIR/repo/.autospec/reports/rule-to-recipe-plan.json"
  [ "$output" = "recipe_available" ]

  perl -0pi -e 's/id: playwright_scaffold\n  title: Playwright scaffolds\n  status: enabled/id: playwright_scaffold\n  title: Playwright scaffolds\n  status: disabled/' "$TEST_TMPDIR/repo/.autospec/state/worker-capabilities.yml"
  run bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" --dry-run --rule testing.playwright.viewport
  [ "$status" -eq 0 ]
  run jq -r '.plans[] | select(.rule_id=="testing.playwright.viewport") | .status' "$TEST_TMPDIR/repo/.autospec/reports/rule-to-recipe-plan.json"
  [ "$output" = "recipe_available_but_disabled" ]
}

@test "decomposition and patch plan produce bounded issue contracts" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_react_repo "$TEST_TMPDIR/repo"
  bash "$RECIPES" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" --dry-run >/dev/null

  run bash "$DECOMP" --repo-root "$TEST_TMPDIR/repo" --dry-run --recipe ai-provider-config-scaffold
  [ "$status" -eq 0 ]
  compgen -G "$TEST_TMPDIR/repo/.autospec/backlog/decomposed/*.md" >/dev/null
  grep -q "token usage" "$TEST_TMPDIR/repo/.autospec/reports/decomposition-plan.md"

  run bash "$PATCH" --repo-root "$TEST_TMPDIR/repo" --dry-run --recipe playwright-viewport-matrix
  [ "$status" -eq 0 ]
  run jq -r '.files_forbidden[]' "$TEST_TMPDIR/repo/.autospec/reports/patch-plan.json"
  [[ "$output" == *".github/workflows"* ]]
  grep -q "Rollback plan" "$TEST_TMPDIR/repo/.autospec/reports/patch-plan.md"
}

@test "template application dry-run is safe, confirm writes allowed path, unresolved vars fail" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/templates" "$TEST_TMPDIR/repo/docs/specs"
  cat > "$TEST_TMPDIR/repo/.autospec/templates/example.md" <<'MD'
# {{title}}
MD

  run bash "$APPLY" --repo-root "$TEST_TMPDIR/repo" --dry-run --template .autospec/templates/example.md --output docs/specs/generated.md --var title=Hello
  [ "$status" -eq 0 ]
  [ ! -f "$TEST_TMPDIR/repo/docs/specs/generated.md" ]

  run bash "$APPLY" --repo-root "$TEST_TMPDIR/repo" --confirm --template .autospec/templates/example.md --output docs/specs/generated.md --var title=Hello
  [ "$status" -eq 0 ]
  grep -q "Generated by Autospec" "$TEST_TMPDIR/repo/docs/specs/generated.md"

  run bash "$APPLY" --repo-root "$TEST_TMPDIR/repo" --confirm --template .autospec/templates/example.md --output /tmp/unsafe.md --var title=Hello
  [ "$status" -ne 0 ]
  run bash "$APPLY" --repo-root "$TEST_TMPDIR/repo" --dry-run --template .autospec/templates/example.md --output docs/specs/missing.md
  [ "$status" -ne 0 ]
}

@test "stack profile reports UI capability gaps so implementers adopt primitives instead of hand-rolling" {
  mkdir -p "$TEST_TMPDIR/bare" "$TEST_TMPDIR/equipped" "$TEST_TMPDIR/cli"
  write_react_repo "$TEST_TMPDIR/bare"
  write_react_repo "$TEST_TMPDIR/equipped"
  cat > "$TEST_TMPDIR/equipped/package.json" <<'JSON'
{"dependencies":{"react":"latest","@radix-ui/react-dialog":"latest","framer-motion":"latest","typescript":"latest"},"devDependencies":{"vite":"latest"}}
JSON
  printf '@media (prefers-reduced-motion: reduce) { * { animation: none; } }\n' > "$TEST_TMPDIR/equipped/src/reset.css"

  # A repo with a web UI but no accessible primitives, no motion library, no reset.
  run bash "$STACK" --repo-root "$TEST_TMPDIR/bare"
  [ "$status" -eq 0 ]
  run jq -r '.ui_capabilities.is_web_ui' "$TEST_TMPDIR/bare/.autospec/state/stack-profile.json"
  [ "$output" = "true" ]
  run jq -r '.ui_capabilities.gaps | sort | join(",")' "$TEST_TMPDIR/bare/.autospec/state/stack-profile.json"
  [ "$output" = "accessible_primitives,motion_library,reduced_motion_reset" ]

  # A repo that already has all three reports no gaps.
  run bash "$STACK" --repo-root "$TEST_TMPDIR/equipped"
  [ "$status" -eq 0 ]
  run jq -r '.ui_capabilities.gaps | length' "$TEST_TMPDIR/equipped/.autospec/state/stack-profile.json"
  [ "$output" = "0" ]
  run jq -r '.ui_capabilities.accessible_primitives.evidence | length > 0' "$TEST_TMPDIR/equipped/.autospec/state/stack-profile.json"
  [ "$output" = "true" ]

  # A non-UI repo is never nagged about UI primitives.
  mkdir -p "$TEST_TMPDIR/cli/.autospec/state" "$TEST_TMPDIR/cli/.autospec/reports"
  echo 'print("hi")' > "$TEST_TMPDIR/cli/main.py"
  run bash "$STACK" --repo-root "$TEST_TMPDIR/cli"
  [ "$status" -eq 0 ]
  run jq -r '.ui_capabilities.is_web_ui' "$TEST_TMPDIR/cli/.autospec/state/stack-profile.json"
  [ "$output" = "false" ]
  run jq -r '.ui_capabilities.gaps | length' "$TEST_TMPDIR/cli/.autospec/state/stack-profile.json"
  [ "$output" = "0" ]
}

@test "worker v3 dry-run executes safe recipe and refuses unsupported stack scaffold" {
  mkdir -p "$TEST_TMPDIR/react" "$TEST_TMPDIR/unknown"
  write_react_repo "$TEST_TMPDIR/react"
  write_react_repo "$TEST_TMPDIR/unknown"
  rm -f "$TEST_TMPDIR/unknown/package.json" "$TEST_TMPDIR/unknown/src/App.tsx"
  bash "$RECIPES" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$STACK" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$RECIPES" --repo-root "$TEST_TMPDIR/unknown" >/dev/null
  bash "$STACK" --repo-root "$TEST_TMPDIR/unknown" >/dev/null

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/react" --dry-run --issue 1 --recipe playwright-viewport-matrix
  [ "$status" -eq 0 ]
  run jq -r '.recipe_id' "$TEST_TMPDIR/react/.autospec/reports/worker-recipe-execution.json"
  [ "$output" = "playwright-viewport-matrix" ]

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/unknown" --dry-run --issue 1 --recipe settings-page-scaffold
  [ "$status" -ne 0 ]
  grep -q "stack confidence" "$TEST_TMPDIR/unknown/.autospec/reports/worker-recipe-execution.md"
}

@test "evidence generation and rule recheck report honest before-after state" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_react_repo "$TEST_TMPDIR/repo"
  bash "$RECIPES" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  run bash "$EVIDENCE" --repo-root "$TEST_TMPDIR/repo" --dry-run --capability playwright-viewport
  [ "$status" -eq 0 ]
  [ ! -f "$TEST_TMPDIR/repo/tests/e2e/autospec-viewport.spec.ts" ]
  grep -q "Playwright" "$TEST_TMPDIR/repo/.autospec/reports/evidence-test-generation-plan.md"

  run bash "$RECHECK" --repo-root "$TEST_TMPDIR/repo" --dry-run --rule testing.playwright.viewport
  [ "$status" -eq 0 ]
  run jq -r '.results[0].before_status + ":" + .results[0].after_status' "$TEST_TMPDIR/repo/.autospec/reports/rule-recheck.json"
  [ "$output" = "fail:fail" ]
}

@test "verifier and supervisor expose recipe context without claiming scaffold runtime complete" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_react_repo "$TEST_TMPDIR/repo"
  bash "$RECIPES" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  bash "$STACK" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue 1 --recipe playwright-viewport-matrix >/dev/null
  cat > "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json" <<'JSON'
{"version":1,"processed_issue_id":"1","classification":"docs-only"}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/reports/worker-diff-review.json" <<'JSON'
{"patch_budget":{"passed":true},"forbidden_path_check":{"passed":true,"matches":[]},"files_changed":[],"test_docs_metadata_change_check":{}}
JSON

  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue 1
  [ "$status" -eq 0 ]
  grep -q "Recipe Review" "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.md"

  run bash "$SUPERVISOR" --repo-root "$TEST_TMPDIR/repo" --dry-run --next
  [ "$status" -eq 0 ]
  grep -q "Recipe Availability" "$TEST_TMPDIR/repo/.autospec/reports/supervisor-cycle-plan.md"
}

@test "autonomy v2 status and spec coverage recognize recipe-backed support" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_react_repo "$TEST_TMPDIR/repo"
  bash "$RECIPES" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  bash "$STACK" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" --dry-run >/dev/null

  run bash "$STATUS" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q "Safe implementable issues" "$TEST_TMPDIR/repo/.autospec/reports/autonomy-v2-status.md"

  run bash "$COVERAGE" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  grep -q "recipe" "$TEST_TMPDIR/repo/.autospec/reports/spec-coverage.md"
}
