#!/usr/bin/env bats
# tests/unit/test_policy_sources_v2.bats — structured Constitution/Baseline policy sources.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  LOAD="$REPO_ROOT/scripts/autospec-load-policy-sources.sh"
  VALIDATE="$REPO_ROOT/scripts/autospec-validate-policy-sources.sh"
  LOCK="$REPO_ROOT/scripts/autospec-lock-policy-sources.sh"
  EXTRACT="$REPO_ROOT/scripts/autospec-extract-constitution-rules.sh"
  COMPOSE="$REPO_ROOT/scripts/autospec-baseline-compose.sh"
  CHECK="$REPO_ROOT/scripts/autospec-check-rules.sh"
  GAP="$REPO_ROOT/scripts/autospec-constitutional-gap-v1.sh"
  AUDIT="$REPO_ROOT/scripts/autospec-constitution-audit.sh"
  COMPAT="$REPO_ROOT/scripts/autospec-policy-compatibility.sh"
  STATUS="$REPO_ROOT/scripts/autospec-autonomy-status.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-policy-v2-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_structured_policy_repos() {
  local constitution="$1"
  local baselines="$2"
  mkdir -p "$constitution/manifests" "$constitution/rules" "$constitution/docs" "$constitution/schemas"
  cat > "$constitution/manifests/categories.yml" <<'YAML'
categories: [testing, metadata, ai, mcp, documentation]
YAML
  cat > "$constitution/manifests/maturity-levels.yml" <<'YAML'
levels:
  prototype: {}
  production: {}
  enterprise: {}
  autonomous: {}
YAML
  cat > "$constitution/manifests/constitution.yml" <<'YAML'
name: autospec-constitution
version: 0.1.0
schema: 1
doctrines:
  - id: testing
    document: docs/testing.md
    rules: rules/testing.yml
YAML
  cat > "$constitution/manifests/doctrines.yml" <<'YAML'
doctrines:
  - id: testing
    title: Testing
    document: docs/testing.md
    rules: rules/testing.yml
YAML
  printf '# Testing Doctrine\n' > "$constitution/docs/testing.md"
  printf '{"type":"object"}\n' > "$constitution/schemas/rule.schema.json"
  cat > "$constitution/rules/testing.yml" <<'YAML'
rules:
  - id: testing.playwright.required_for_web
    rule_id: testing.playwright.required_for_web
    title: Playwright required for web
    summary: Web apps need browser workflow coverage.
    source: {doctrine: testing, document: docs/testing.md, section: Rules}
    category: testing
    severity: required
    maturity: {level: production}
    applies_when:
      application_types: [web]
      profiles: [web]
      technologies: []
      repo_conditions: []
    check:
      type: required_tool
      expected: {name: playwright}
    evidence_required: [Playwright dependency or tests]
    acceptance_criteria: [Playwright is configured.]
    metadata_required: [test-coverage-map]
    quality_gates: [Viewport matrix exists]
    remediation:
      hint: Add Playwright viewport coverage.
      suggested_issue_title: "test: add Playwright coverage"
      suggested_labels: [autospec:testing, autospec:web]
    risk: {level: low, requires_human_review: false, requires_architecture_review: false}
YAML

  mkdir -p "$baselines/manifests" "$baselines/packs/application" "$baselines/packs/ai" "$baselines/schemas"
  cat > "$baselines/manifests/pack-categories.yml" <<'YAML'
categories: [application, ai]
YAML
  cat > "$baselines/manifests/baselines.yml" <<'YAML'
name: autospec-baselines
version: 0.1.0
schema: 1
packs:
  - id: application/web
    file: packs/application/web.yml
  - id: ai/mcp
    file: packs/ai/mcp.yml
YAML
  cat > "$baselines/manifests/profiles.yml" <<'YAML'
profiles:
  - id: web
    packs: [application/web]
  - id: ai-platform
    packs: [ai/mcp]
YAML
  printf '{"type":"object"}\n' > "$baselines/schemas/baseline-pack.schema.json"
  cat > "$baselines/packs/application/web.yml" <<'YAML'
id: application/web
title: Web Application
version: 0.1.0
type: application
summary: Browser app baseline.
applies_when: {application_types: [web], technologies: [], profiles: [web]}
inherits: []
requires: [ai/mcp]
conflicts_with: []
capabilities:
  required: [in-app documentation center, responsive UI]
  recommended: [design tokens]
metadata_required: [ui-surface]
rules:
  - id: baseline.web.docs.required
    rule_id: baseline.web.docs.required
    title: In-app docs required
    summary: Web apps expose in-app documentation.
    source: {baseline: application/web, document: docs/application/web.md, section: Required Capabilities}
    category: documentation
    severity: required
    maturity: {level: production}
    applies_when: {application_types: [web], profiles: [web], technologies: [], repo_conditions: []}
    check: {type: required_capability, expected: {id: in-app-documentation}}
    evidence_required: [capability]
    acceptance_criteria: [In-app docs capability exists.]
    metadata_required: [capability-registry]
    quality_gates: [Docs route is navigable]
    remediation:
      hint: Add docs route.
      suggested_issue_title: "feat: add in-app documentation center"
      suggested_labels: [autospec:documentation]
    risk: {level: medium, requires_human_review: true, requires_architecture_review: false}
quality_gates: [Docs route is navigable]
issue_templates:
  - id: add-docs
    title: Add in-app docs
YAML
  cat > "$baselines/packs/ai/mcp.yml" <<'YAML'
id: ai/mcp
title: MCP
version: 0.1.0
type: ai
summary: MCP baseline.
applies_when: {application_types: [], technologies: [mcp], profiles: [ai-platform]}
inherits: []
requires: []
conflicts_with: []
capabilities:
  required: [MCP registry]
  recommended: [diagnostics]
metadata_required: [mcp-registry]
rules:
  - id: baseline.mcp.registry.required
    rule_id: baseline.mcp.registry.required
    title: MCP registry required
    summary: MCP capabilities are registered.
    source: {baseline: ai/mcp, document: docs/ai/mcp.md, section: Required Capabilities}
    category: mcp
    severity: required
    maturity: {level: autonomous}
    applies_when: {application_types: [], profiles: [ai-platform], technologies: [mcp], repo_conditions: []}
    check: {type: required_mcp_capability, expected: {id: registry}}
    evidence_required: [mcp registry]
    acceptance_criteria: [MCP registry exists.]
    metadata_required: [mcp-registry]
    quality_gates: [Tools default read-only]
    remediation:
      hint: Add MCP registry metadata.
      suggested_issue_title: "feat: add MCP registry"
      suggested_labels: [autospec:mcp]
    risk: {level: high, requires_human_review: true, requires_architecture_review: true}
quality_gates: [Tools default read-only]
issue_templates:
  - id: add-mcp-registry
    title: Add MCP registry
YAML
}

write_markdown_policy_repos() {
  local constitution="$1"
  local baselines="$2"
  mkdir -p "$constitution/docs" "$baselines/docs"
  cat > "$constitution/docs/testing.md" <<'MD'
# Testing Doctrine

## Playwright standard

Web apps require Playwright evidence.
MD
  cat > "$baselines/docs/web.md" <<'MD'
# Web Baseline

## Documentation required

Web applications require documentation.
MD
}

write_repo_config() {
  local repo="$1"
  local constitution="$2"
  local baselines="$3"
  mkdir -p "$repo/.autospec"
  cat > "$repo/.autospec/autospec.yml" <<YAML
constitution:
  source: local
  path: $constitution
  version: 0.1.0
baselines:
  source: local
  path: $baselines
  profiles:
    - web
    - ai-platform
application:
  type: web
  maturity_target: production
YAML
}

write_repo_metadata() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state" "$repo/.autospec/reports" "$repo/docs" "$repo/tests"
  printf '# App\n' > "$repo/README.md"
  printf '# Docs\n' > "$repo/docs/help.md"
  printf 'test\n' > "$repo/tests/app.test"
  cat > "$repo/.autospec/state/repository-inventory.json" <<'JSON'
{"files":[{"path":"README.md"},{"path":"docs/help.md"},{"path":"tests/app.test"},{"path":"package.json"}],"files_by_purpose":{"documentation":["README.md","docs/help.md"],"test":["tests/app.test"]}}
JSON
  cat > "$repo/.autospec/state/capability-registry.json" <<'JSON'
{"capabilities":[{"id":"in-app-documentation","title":"Docs"}]}
JSON
  cat > "$repo/.autospec/state/technology-registry.yml" <<'YAML'
technologies:
  - name: react
YAML
}

@test "policy source loader validation lock and structured extraction preserve provenance" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_structured_policy_repos "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"
  write_repo_config "$TEST_TMPDIR/repo" "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"
  write_repo_metadata "$TEST_TMPDIR/repo"

  run bash "$LOAD" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  run jq -r '.constitution.structured_available' "$TEST_TMPDIR/repo/.autospec/state/policy-sources.json"
  [ "$output" = "true" ]
  run bash "$VALIDATE" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  run bash "$LOCK" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/policy-sources.lock.json" ]

  run bash "$EXTRACT" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  run jq -r '.rules[] | select(.rule_id=="testing.playwright.required_for_web") | [.source_format,.source_repo,.source_file,.confidence] | @tsv' "$TEST_TMPDIR/repo/.autospec/state/constitution-rules.json"
  [ "$output" = $'structured_yaml\tautospec-constitution\trules/testing.yml\t1.0' ]
  run jq -r '.rules[] | select(.rule_id=="baseline.web.docs.required") | [.source_pack,.profile] | @tsv' "$TEST_TMPDIR/repo/.autospec/state/baseline-rules.json"
  [ "$output" = $'application/web\tweb' ]
}

@test "markdown-only policy sources fallback with lower confidence and clear report" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_markdown_policy_repos "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"
  write_repo_config "$TEST_TMPDIR/repo" "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"

  run bash "$LOAD" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  run jq -r '.constitution.fallback_used' "$TEST_TMPDIR/repo/.autospec/state/policy-sources.json"
  [ "$output" = "true" ]
  run bash "$EXTRACT" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  run jq -r '.rules[0].source_format' "$TEST_TMPDIR/repo/.autospec/state/constitution-rules.json"
  [ "$output" = "markdown_heuristic" ]
}

@test "structured baseline composition expands profiles requires capabilities rules and gates" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_structured_policy_repos "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"
  write_repo_config "$TEST_TMPDIR/repo" "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"

  run bash "$COMPOSE" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  run jq -r '.structured' "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json"
  [ "$output" = "true" ]
  run jq -r '.included_packs[].id' "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json"
  [[ "$output" == *"application/web"* ]]
  [[ "$output" == *"ai/mcp"* ]]
  run jq -r '.composed.rules[].rule_id' "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json"
  [[ "$output" == *"baseline.web.docs.required"* ]]
  grep -q 'Effective Rules' "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.md"
}

@test "rule checks v2 quality gates issue plan v3 audit and compatibility reports are generated" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_structured_policy_repos "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"
  write_repo_config "$TEST_TMPDIR/repo" "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"
  write_repo_metadata "$TEST_TMPDIR/repo"
  bash "$COMPOSE" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  bash "$EXTRACT" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  run bash "$CHECK" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 1 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/quality-gates.md" ]
  run jq -r '.results[] | select(.rule_id=="testing.playwright.required_for_web") | [.source_format,.suggested_labels[0]] | @tsv' "$TEST_TMPDIR/repo/.autospec/reports/rule-check-results.json"
  [ "$output" = $'structured_yaml\tautospec:testing' ]
  run bash "$GAP" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 1 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/issue-plan-v3.md" ]
  grep -q 'testing.playwright.required_for_web' "$TEST_TMPDIR/repo/.autospec/backlog/issues-v3/"*.md

  run bash "$COMPAT" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/policy-compatibility.md" ]
  run bash "$AUDIT" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 1 ]
  run jq -r '.side_effects.github_writes' "$TEST_TMPDIR/repo/.autospec/reports/constitution-audit.json"
  [ "$output" = "false" ]
  grep -q 'structured vs heuristic' "$TEST_TMPDIR/repo/.autospec/reports/constitution-audit.md"
}

@test "policy validation reports duplicate rule unsupported check and missing pack clearly" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_structured_policy_repos "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"
  write_repo_config "$TEST_TMPDIR/repo" "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"
  cat >> "$TEST_TMPDIR/constitution/rules/testing.yml" <<'YAML'
  - id: testing.playwright.required_for_web
    rule_id: testing.playwright.required_for_web
    title: Duplicate
    summary: Duplicate.
    source: {doctrine: testing, document: docs/testing.md, section: Rules}
    category: unknown_category
    severity: required
    maturity: {level: production}
    applies_when: {application_types: [], profiles: [], technologies: [], repo_conditions: []}
    check: {type: not_supported_yet, expected: {}}
    evidence_required: [evidence]
    acceptance_criteria: [criteria]
    metadata_required: []
    quality_gates: []
    remediation: {hint: Fix it., suggested_issue_title: "fix", suggested_labels: []}
    risk: {level: low, requires_human_review: false, requires_architecture_review: false}
YAML
  rm "$TEST_TMPDIR/baselines/packs/ai/mcp.yml"

  run bash "$VALIDATE" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 1 ]
  run jq -r '.findings[].code' "$TEST_TMPDIR/repo/.autospec/reports/policy-source-validation.json"
  [[ "$output" == *"DUPLICATE_RULE_ID"* ]]
  [[ "$output" == *"UNKNOWN_CATEGORY"* ]]
  [[ "$output" == *"UNSUPPORTED_CHECK_TYPE"* ]]
  [[ "$output" == *"BASELINE_PACK_MISSING"* ]]
}
