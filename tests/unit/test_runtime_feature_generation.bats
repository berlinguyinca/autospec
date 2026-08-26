#!/usr/bin/env bats
# tests/unit/test_runtime_feature_generation.bats — bounded target-app runtime feature slices.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-runtime-v1-XXXXXX)"
  STACK="$REPO_ROOT/scripts/autospec-detect-stack-profile.sh"
  ADAPTERS="$REPO_ROOT/scripts/autospec-runtime-adapter-index.sh"
  SLICES="$REPO_ROOT/scripts/autospec-feature-slice-index.sh"
  PLAN="$REPO_ROOT/scripts/autospec-runtime-implementation-plan.sh"
  GENERATE="$REPO_ROOT/scripts/autospec-generate-runtime-feature.sh"
  PLAYWRIGHT="$REPO_ROOT/scripts/autospec-generate-playwright-evidence.sh"
  SYNC="$REPO_ROOT/scripts/autospec-sync-runtime-metadata.sh"
  VERIFY_RUNTIME="$REPO_ROOT/scripts/autospec-verify-runtime-feature.sh"
  WORKER="$REPO_ROOT/scripts/autospec-worker-one.sh"
  VERIFY_PR="$REPO_ROOT/scripts/autospec-verify-worker-pr.sh"
  SUPERVISOR="$REPO_ROOT/scripts/autospec-supervisor-cycle.sh"
  STATUS="$REPO_ROOT/scripts/autospec-runtime-feature-status.sh"
  COVERAGE="$REPO_ROOT/scripts/autospec-spec-coverage.sh"
  MVP="$REPO_ROOT/scripts/autospec-mvp-status.sh"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_react_vite_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state" "$repo/.autospec/reports" "$repo/src" "$repo/tests/e2e" "$repo/docs/specs"
  cat > "$repo/package.json" <<'JSON'
{"dependencies":{"@vitejs/plugin-react":"latest","react":"latest","typescript":"latest"},"devDependencies":{"@playwright/test":"latest","vite":"latest"},"scripts":{"test":"vitest"}}
JSON
  # tsconfig.json pins the package.json marker to typescript; the .tsx source line then carries 100% line share, confidence 0.95 >= 0.8 scaffold gate (language-selection-axis spec, confidence clamp).
  echo '{}' > "$repo/tsconfig.json"
  echo 'export const App = () => null;' > "$repo/src/App.tsx"
  cat > "$repo/.autospec/state/rule-check-results.json" <<'JSON'
{"schema":1,"results":[{"rule_id":"product.docs.center","status":"fail","check_type":"required_in_app_documentation","severity":"required","category":"product_baseline"}]}
JSON
  cp "$repo/.autospec/state/rule-check-results.json" "$repo/.autospec/reports/rule-check-results.json"
  echo '{"schema":1,"capabilities":[]}' > "$repo/.autospec/state/capability-registry.json"
  echo '{"schema":1,"summary":{"application_type":"web"}}' > "$repo/.autospec/state/digital-twin.json"
}

write_next_app_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state" "$repo/.autospec/reports" "$repo/app"
  cat > "$repo/package.json" <<'JSON'
{"dependencies":{"next":"latest","react":"latest","typescript":"latest"}}
JSON
  # tsconfig.json pins the package.json marker to typescript; the .tsx source line then carries 100% line share, confidence 0.95 >= 0.8 scaffold gate (language-selection-axis spec, confidence clamp).
  echo '{}' > "$repo/tsconfig.json"
  echo 'export default function Home(){ return null; }' > "$repo/app/page.tsx"
}

write_next_pages_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state" "$repo/.autospec/reports" "$repo/pages"
  cat > "$repo/package.json" <<'JSON'
{"dependencies":{"next":"latest","react":"latest","typescript":"latest"}}
JSON
  # tsconfig.json pins the package.json marker to typescript; the .tsx source line then carries 100% line share, confidence 0.95 >= 0.8 scaffold gate (language-selection-axis spec, confidence clamp).
  echo '{}' > "$repo/tsconfig.json"
  echo 'export default function Home(){ return null; }' > "$repo/pages/index.tsx"
}

write_unknown_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state" "$repo/.autospec/reports"
  echo '# unknown' > "$repo/README.md"
}

@test "runtime adapter and feature slice indexes load and map recognized stacks" {
  mkdir -p "$TEST_TMPDIR/react" "$TEST_TMPDIR/next" "$TEST_TMPDIR/unknown"
  write_react_vite_repo "$TEST_TMPDIR/react"
  write_next_app_repo "$TEST_TMPDIR/next"
  write_unknown_repo "$TEST_TMPDIR/unknown"

  bash "$STACK" --repo-root "$TEST_TMPDIR/react" >/dev/null
  run bash "$ADAPTERS" --repo-root "$TEST_TMPDIR/react"
  [ "$status" -eq 0 ]
  run jq -r '.adapters[] | select(.id=="react-vite") | .match_status' "$TEST_TMPDIR/react/.autospec/reports/runtime-adapters.json"
  [ "$output" = "available" ]

  bash "$STACK" --repo-root "$TEST_TMPDIR/next" >/dev/null
  run bash "$ADAPTERS" --repo-root "$TEST_TMPDIR/next"
  [ "$status" -eq 0 ]
  run jq -r '.adapters[] | select(.id=="nextjs-app-router") | .match_status' "$TEST_TMPDIR/next/.autospec/reports/runtime-adapters.json"
  [ "$output" = "available" ]

  bash "$STACK" --repo-root "$TEST_TMPDIR/unknown" >/dev/null
  run bash "$ADAPTERS" --repo-root "$TEST_TMPDIR/unknown"
  [ "$status" -eq 0 ]
  run jq -r '[.adapters[] | select(.match_status=="available")] | length' "$TEST_TMPDIR/unknown/.autospec/reports/runtime-adapters.json"
  [ "$output" = "0" ]

  run bash "$SLICES" --repo-root "$TEST_TMPDIR/react"
  [ "$status" -eq 0 ]
  run jq -r '.feature_slices[] | select(.id=="in-app-docs-center") | .runtime_claim_level' "$TEST_TMPDIR/react/.autospec/reports/feature-slices.json"
  [ "$output" = "shell" ]
}

@test "runtime planner classifies safe recognized features and refuses low confidence" {
  mkdir -p "$TEST_TMPDIR/react" "$TEST_TMPDIR/unknown"
  write_react_vite_repo "$TEST_TMPDIR/react"
  write_unknown_repo "$TEST_TMPDIR/unknown"
  bash "$STACK" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$ADAPTERS" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$SLICES" --repo-root "$TEST_TMPDIR/react" >/dev/null

  run bash "$PLAN" --repo-root "$TEST_TMPDIR/react" --dry-run --feature in-app-docs-center
  [ "$status" -eq 0 ]
  run jq -r '.plans[0].status + ":" + .plans[0].adapter + ":" + .plans[0].runtime_claim_level' "$TEST_TMPDIR/react/.autospec/reports/runtime-implementation-plan.json"
  [ "$output" = "safe_to_generate:react-vite:shell" ]

  bash "$STACK" --repo-root "$TEST_TMPDIR/unknown" >/dev/null
  bash "$ADAPTERS" --repo-root "$TEST_TMPDIR/unknown" >/dev/null
  bash "$SLICES" --repo-root "$TEST_TMPDIR/unknown" >/dev/null
  run bash "$PLAN" --repo-root "$TEST_TMPDIR/unknown" --dry-run --feature in-app-docs-center
  [ "$status" -eq 0 ]
  run jq -r '.plans[0].status' "$TEST_TMPDIR/unknown/.autospec/reports/runtime-implementation-plan.json"
  [ "$output" = "low_stack_confidence" ]
}

@test "runtime generator dry-run writes nothing and confirm writes bounded files with headers" {
  mkdir -p "$TEST_TMPDIR/react"
  write_react_vite_repo "$TEST_TMPDIR/react"
  bash "$STACK" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$ADAPTERS" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$SLICES" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$PLAN" --repo-root "$TEST_TMPDIR/react" --dry-run --feature in-app-docs-center >/dev/null

  run bash "$GENERATE" --repo-root "$TEST_TMPDIR/react" --dry-run --feature in-app-docs-center
  [ "$status" -eq 0 ]
  [ ! -f "$TEST_TMPDIR/react/src/components/autospec/DocsCenter.tsx" ]

  run bash "$GENERATE" --repo-root "$TEST_TMPDIR/react" --confirm --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "Generated by Autospec runtime feature generator" "$TEST_TMPDIR/react/src/components/autospec/DocsCenter.tsx"
  grep -q "Runtime claim level: shell" "$TEST_TMPDIR/react/src/pages/Docs.tsx"
  run jq -r '.status + ":" + (.side_effects.wrote_files|tostring)' "$TEST_TMPDIR/react/.autospec/reports/runtime-generation-result.json"
  [ "$output" = "generated:true" ]
}

@test "runtime generator writes valid relative imports for Next adapters" {
  mkdir -p "$TEST_TMPDIR/next-app" "$TEST_TMPDIR/next-pages"
  write_next_app_repo "$TEST_TMPDIR/next-app"
  write_next_pages_repo "$TEST_TMPDIR/next-pages"

  for repo in "$TEST_TMPDIR/next-app" "$TEST_TMPDIR/next-pages"; do
    bash "$STACK" --repo-root "$repo" >/dev/null
    bash "$ADAPTERS" --repo-root "$repo" >/dev/null
    bash "$SLICES" --repo-root "$repo" >/dev/null
    run bash "$GENERATE" --repo-root "$repo" --confirm --feature in-app-docs-center
    [ "$status" -eq 0 ]
  done

  grep -q "from '../../src/components/autospec/DocsCenter'" "$TEST_TMPDIR/next-app/app/docs/page.tsx"
  grep -q "from '../src/components/autospec/DocsCenter'" "$TEST_TMPDIR/next-pages/pages/docs.tsx"
  grep -q "Generated by Autospec runtime feature generator" "$TEST_TMPDIR/next-app/src/components/autospec/DocsCenter.tsx"
  grep -q "Generated by Autospec runtime feature generator" "$TEST_TMPDIR/next-pages/src/components/autospec/DocsCenter.tsx"
}

@test "AI token dashboard shell avoids migrations and creates dependency decision backlog" {
  mkdir -p "$TEST_TMPDIR/react"
  write_react_vite_repo "$TEST_TMPDIR/react"
  bash "$STACK" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$ADAPTERS" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$SLICES" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$PLAN" --repo-root "$TEST_TMPDIR/react" --dry-run --feature ai-usage-token-dashboard >/dev/null

  run bash "$GENERATE" --repo-root "$TEST_TMPDIR/react" --confirm --feature ai-usage-token-dashboard
  [ "$status" -eq 0 ]
  grep -q "No database migration" "$TEST_TMPDIR/react/docs/specs/ai-usage-token-dashboard.md"
  [ ! -d "$TEST_TMPDIR/react/migrations" ]
  compgen -G "$TEST_TMPDIR/react/.autospec/backlog/runtime-dependencies/*.md" >/dev/null
}

@test "Playwright evidence generation creates tests only when Playwright exists" {
  mkdir -p "$TEST_TMPDIR/react" "$TEST_TMPDIR/no-playwright"
  write_react_vite_repo "$TEST_TMPDIR/react"
  write_react_vite_repo "$TEST_TMPDIR/no-playwright"
  cat > "$TEST_TMPDIR/no-playwright/package.json" <<'JSON'
{"dependencies":{"@vitejs/plugin-react":"latest","react":"latest","typescript":"latest"},"devDependencies":{"vite":"latest"},"scripts":{"test":"vitest"}}
JSON
  bash "$STACK" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$ADAPTERS" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$SLICES" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$PLAN" --repo-root "$TEST_TMPDIR/react" --dry-run --feature in-app-docs-center >/dev/null
  bash "$GENERATE" --repo-root "$TEST_TMPDIR/react" --confirm --feature in-app-docs-center >/dev/null

  run bash "$PLAYWRIGHT" --repo-root "$TEST_TMPDIR/react" --confirm --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "viewport" "$TEST_TMPDIR/react/tests/e2e/autospec-in-app-docs-center.spec.ts"

  run bash "$PLAYWRIGHT" --repo-root "$TEST_TMPDIR/no-playwright" --dry-run --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "adoption spec" "$TEST_TMPDIR/no-playwright/.autospec/reports/playwright-generation-plan.md"
}

@test "metadata sync and runtime verification validate generated shell honestly" {
  mkdir -p "$TEST_TMPDIR/react"
  write_react_vite_repo "$TEST_TMPDIR/react"
  bash "$STACK" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$ADAPTERS" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$SLICES" --repo-root "$TEST_TMPDIR/react" >/dev/null
  bash "$PLAN" --repo-root "$TEST_TMPDIR/react" --dry-run --feature ai-provider-settings >/dev/null
  bash "$GENERATE" --repo-root "$TEST_TMPDIR/react" --confirm --feature ai-provider-settings >/dev/null

  run bash "$SYNC" --repo-root "$TEST_TMPDIR/react" --confirm
  [ "$status" -eq 0 ]
  run jq -r '.features[0].feature_id' "$TEST_TMPDIR/react/.autospec/state/ui-surface.json"
  [ "$output" = "ai-provider-settings" ]
  run jq -r '.features[0].provider_secret_policy' "$TEST_TMPDIR/react/.autospec/state/ai-capabilities.json"
  [ "$output" = "references_only_no_values" ]

  run bash "$VERIFY_RUNTIME" --repo-root "$TEST_TMPDIR/react" --dry-run --feature ai-provider-settings
  [ "$status" -eq 0 ]
  grep -q "Runtime Claim Honesty" "$TEST_TMPDIR/react/.autospec/reports/runtime-feature-verification.md"
}

@test "worker v4 runtime feature flow is explicit and refuses low-confidence stacks" {
  mkdir -p "$TEST_TMPDIR/react" "$TEST_TMPDIR/unknown"
  write_react_vite_repo "$TEST_TMPDIR/react"
  write_unknown_repo "$TEST_TMPDIR/unknown"

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/react" --dry-run --issue 1 --feature in-app-docs-center
  [ "$status" -eq 0 ]
  grep -q "Runtime Feature Generation" "$TEST_TMPDIR/react/.autospec/reports/worker-runtime-feature-result.md"

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/unknown" --dry-run --issue 1 --feature in-app-docs-center
  [ "$status" -ne 0 ]
  grep -q "low_stack_confidence" "$TEST_TMPDIR/unknown/.autospec/reports/runtime-generation-result.md"
}

@test "verifier and supervisor expose runtime review without enabling generation by default" {
  mkdir -p "$TEST_TMPDIR/react"
  write_react_vite_repo "$TEST_TMPDIR/react"
  bash "$WORKER" --repo-root "$TEST_TMPDIR/react" --dry-run --issue 1 --feature in-app-docs-center >/dev/null
  cat > "$TEST_TMPDIR/react/.autospec/reports/worker-risk-classification.json" <<'JSON'
{"version":1,"processed_issue_id":"1","classification":"docs-only"}
JSON
  cat > "$TEST_TMPDIR/react/.autospec/reports/worker-diff-review.json" <<'JSON'
{"patch_budget":{"passed":true},"forbidden_path_check":{"passed":true,"matches":[]},"files_changed":[],"test_docs_metadata_change_check":{}}
JSON

  run bash "$VERIFY_PR" --repo-root "$TEST_TMPDIR/react" --dry-run --issue 1
  [ "$status" -eq 0 ]
  grep -q "Runtime Feature Review" "$TEST_TMPDIR/react/.autospec/reports/verifier-report.md"

  run bash "$SUPERVISOR" --repo-root "$TEST_TMPDIR/react" --dry-run --next
  [ "$status" -eq 0 ]
  grep -q "Runtime Feature Candidate" "$TEST_TMPDIR/react/.autospec/reports/supervisor-cycle-plan.md"
  grep -q "allow_runtime_features: false" "$TEST_TMPDIR/react/.autospec/reports/supervisor-cycle-plan.md"
}

@test "runtime status and spec coverage recognize generated runtime shells" {
  mkdir -p "$TEST_TMPDIR/react"
  write_react_vite_repo "$TEST_TMPDIR/react"
  bash "$WORKER" --repo-root "$TEST_TMPDIR/react" --dry-run --issue 1 --feature in-app-docs-center >/dev/null

  run bash "$STATUS" --repo-root "$TEST_TMPDIR/react"
  [ "$status" -eq 0 ]
  grep -q "Runtime shells" "$TEST_TMPDIR/react/.autospec/reports/runtime-feature-status.md"

  run bash "$COVERAGE" --repo-root "$TEST_TMPDIR/react" --dry-run
  [ "$status" -eq 0 ]
  grep -q "runtime feature" "$TEST_TMPDIR/react/.autospec/reports/spec-coverage.md"

  run bash "$MVP" --repo-root "$TEST_TMPDIR/react"
  [ "$status" -eq 0 ]
  grep -q "Runtime feature status" "$TEST_TMPDIR/react/.autospec/reports/mvp-status.md"
}
