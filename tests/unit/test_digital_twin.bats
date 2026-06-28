#!/usr/bin/env bats
# tests/unit/test_digital_twin.bats — Digital Twin v1 and Knowledge Graph foundations.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  BUILD="$REPO_ROOT/scripts/autospec-build-digital-twin.sh"
  IMPACT="$REPO_ROOT/scripts/autospec-impact-analysis.sh"
  DRIFT="$REPO_ROOT/scripts/autospec-metadata-drift.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-digital-twin-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_fixture_repo() {
  local repo="$1"
  mkdir -p "$repo/docs/tutorials" "$repo/src/api" "$repo/src/pages" "$repo/src/components" \
    "$repo/src/models" "$repo/src/ai" "$repo/tests/e2e" "$repo/tests/unit" "$repo/scripts" \
    "$repo/db/migrations" "$repo/.github/workflows" "$repo/.mcp"
  cat > "$repo/package.json" <<'JSON'
{
  "name": "twin-fixture",
  "scripts": {
    "build": "vite build",
    "test": "vitest run",
    "e2e": "playwright test",
    "guide": "node scripts/guide.js"
  },
  "dependencies": {
    "express": "^4.18.0",
    "react": "^18.0.0",
    "axios": "^1.0.0",
    "recharts": "^2.0.0",
    "chart.js": "^4.0.0",
    "openai": "^4.0.0",
    "@modelcontextprotocol/sdk": "^1.0.0",
    "zod": "^3.0.0"
  },
  "devDependencies": {
    "vitest": "^1.0.0",
    "jest": "^29.0.0",
    "playwright": "^1.0.0",
    "eslint": "^8.0.0",
    "prettier": "^3.0.0",
    "vite": "^5.0.0"
  }
}
JSON
  cat > "$repo/README.md" <<'MD'
# Twin Fixture

## Capabilities

### Create Project

Users can create projects and review analytics reports.

## Usage

Run `npm run guide` to inspect project guidance.
MD
  cat > "$repo/docs/tutorials/create-project.md" <<'MD'
# Create Project Tutorial

1. Open dashboard.
2. Create project.
3. Review analytics report.
MD
  printf 'app.get("/api/projects", handler)\napp.post("/api/projects", handler)\n' > "$repo/src/api/projects.ts"
  printf 'export default function DashboardPage() { return <ProjectList /> }\n' > "$repo/src/pages/dashboard.tsx"
  printf 'export function ProjectList() { return null }\n' > "$repo/src/components/ProjectList.tsx"
  printf 'export class Project { id = "" }\nexport class User { id = "" }\n' > "$repo/src/models/project.ts"
  printf 'OPENAI_API_KEY process.env.OPENAI_API_KEY\ncreateEmbedding("project")\n' > "$repo/src/ai/assistant.ts"
  printf 'DATABASE_URL process.env.DATABASE_URL\n' > "$repo/src/config.ts"
  printf 'CREATE TABLE projects (id text);\n' > "$repo/db/migrations/001_create_projects.sql"
  printf 'test("ProjectList renders", () => {})\n' > "$repo/tests/unit/project.test.ts"
  printf 'test("create project workflow", async ({ page }) => {})\n' > "$repo/tests/e2e/create-project.spec.ts"
  printf '#!/usr/bin/env bash\necho guide\n' > "$repo/scripts/guide.sh"
  printf 'name: ci\n' > "$repo/.github/workflows/ci.yml"
  printf '{"servers":{"local":{"command":"node server.js"}}}\n' > "$repo/.mcp/config.json"
  printf 'node_modules\n' > "$repo/.gitignore"
}

@test "unified digital twin build writes deterministic inventory technology capability surface graph and twin reports" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_fixture_repo "$TEST_TMPDIR/repo"

  run bash "$BUILD" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"digital twin build: PASS"* ]]

  for file in \
    .autospec/state/repository-inventory.json \
    .autospec/state/technology-registry.yml \
    .autospec/state/capability-registry.json \
    .autospec/state/api-surface.json \
    .autospec/state/ui-surface.json \
    .autospec/state/data-surface.json \
    .autospec/state/settings-registry.json \
    .autospec/state/permission-model.json \
    .autospec/state/ai-capabilities.json \
    .autospec/state/mcp-registry.json \
    .autospec/state/domain-model.json \
    .autospec/state/workflow-map.json \
    .autospec/state/knowledge-graph.json \
    .autospec/state/digital-twin.json \
    .autospec/reports/repository-inventory.md \
    .autospec/reports/technology-registry.md \
    .autospec/reports/capability-registry.md \
    .autospec/reports/domain-workflow-map.md \
    .autospec/reports/knowledge-graph.md \
    .autospec/reports/digital-twin.md \
    .autospec/reports/digital-twin-build.json \
    .autospec/reports/digital-twin-build.md; do
    [ -f "$TEST_TMPDIR/repo/$file" ]
  done

  run jq -r '.files[] | select(.path=="src/api/projects.ts") | .hash' "$TEST_TMPDIR/repo/.autospec/state/repository-inventory.json"
  [[ "$output" =~ ^sha256: ]]
  run jq -r '.files[] | select(.path=="src/config.ts") | .evidence[0]' "$TEST_TMPDIR/repo/.autospec/state/repository-inventory.json"
  [[ "$output" == *"src/config.ts"* ]]
  run jq -r '.capabilities[] | select(.id=="create-project") | .type' "$TEST_TMPDIR/repo/.autospec/state/capability-registry.json"
  [ "$output" = "docs" ]
  run jq -r '.routes[0].path' "$TEST_TMPDIR/repo/.autospec/state/api-surface.json"
  [ "$output" = "/api/projects" ]
  run jq -r '.entities[] | select(.name=="Project") | .confidence' "$TEST_TMPDIR/repo/.autospec/state/domain-model.json"
  [ "$output" != "null" ]
  run jq -r '.nodes | length' "$TEST_TMPDIR/repo/.autospec/state/knowledge-graph.json"
  [ "$output" -gt 0 ]
  run jq -r '.sources.knowledge_graph' "$TEST_TMPDIR/repo/.autospec/state/digital-twin.json"
  [ "$output" = ".autospec/state/knowledge-graph.json" ]
  grep -q '## State Of The Repo' "$TEST_TMPDIR/repo/.autospec/reports/digital-twin.md"
  grep -q 'multiple charting libraries' "$TEST_TMPDIR/repo/.autospec/reports/technology-registry.md"
  ! grep -q 'sk-' "$TEST_TMPDIR/repo/.autospec/reports/settings-registry.md"

  cp "$TEST_TMPDIR/repo/.autospec/state/repository-inventory.json" "$TEST_TMPDIR/first.json"
  run bash "$BUILD" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  diff "$TEST_TMPDIR/first.json" "$TEST_TMPDIR/repo/.autospec/state/repository-inventory.json"
}

@test "impact analysis returns related files docs tests capabilities and warnings" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_fixture_repo "$TEST_TMPDIR/repo"
  bash "$BUILD" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  run bash "$IMPACT" --repo-root "$TEST_TMPDIR/repo" --dry-run --file src/api/projects.ts
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/impact-analysis.md" ]
  run jq -r '.query.type' "$TEST_TMPDIR/repo/.autospec/reports/impact-analysis.json"
  [ "$output" = "file" ]
  run jq -r '.directly_affected_files[0]' "$TEST_TMPDIR/repo/.autospec/reports/impact-analysis.json"
  [ "$output" = "src/api/projects.ts" ]
  run jq -r '.related_tests[]' "$TEST_TMPDIR/repo/.autospec/reports/impact-analysis.json"
  [[ "$output" == *"tests/unit/project.test.ts"* ]]

  run bash "$IMPACT" --repo-root "$TEST_TMPDIR/repo" --dry-run --capability create-project
  [ "$status" -eq 0 ]
  run jq -r '.related_capabilities[0]' "$TEST_TMPDIR/repo/.autospec/reports/impact-analysis.json"
  [ "$output" = "create-project" ]

  run bash "$IMPACT" --repo-root "$TEST_TMPDIR/repo" --dry-run --file missing.ts
  [ "$status" -eq 0 ]
  run jq -r '.warnings[0]' "$TEST_TMPDIR/repo/.autospec/reports/impact-analysis.json"
  [[ "$output" == *"not found"* ]]
}

@test "metadata drift reports missing references orphan docs tests and missing metadata" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_fixture_repo "$TEST_TMPDIR/repo"
  bash "$BUILD" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  rm "$TEST_TMPDIR/repo/src/components/ProjectList.tsx"
  rm "$TEST_TMPDIR/repo/.autospec/state/workflow-map.json"

  run bash "$DRIFT" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 1 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/metadata-drift.md" ]
  run jq -r '.findings[].code' "$TEST_TMPDIR/repo/.autospec/reports/metadata-drift.json"
  [[ "$output" == *"MISSING_REQUIRED_METADATA"* ]]
  [[ "$output" == *"MISSING_FILE_REFERENCE"* ]]
}

@test "metadata schema registry exists for digital twin state contracts" {
  for schema in repository-inventory technology-registry product-purpose domain-model workflow-map \
    api-surface ui-surface data-surface capability-registry settings-registry permission-model \
    ai-capabilities mcp-registry knowledge-graph digital-twin impact-analysis metadata-drift; do
    path="$REPO_ROOT/schemas/autospec-state/$schema.schema.json"
    [ -f "$path" ]
    run jq -r '.properties.schema.type' "$path"
    [ "$output" = "integer" ]
    run jq -r '.properties.generated_at.type' "$path"
    [ "$output" = "string" ]
  done
}

@test "digital twin batch adds no GitHub Actions cron or scheduler automation" {
  ! git -C "$REPO_ROOT" diff --name-only | grep -Eq '^\\.github/workflows/|cron|launchd|systemd'
}
