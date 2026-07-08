#!/usr/bin/env bats
# tests/unit/test_release_candidate_closure.bats — local validation foundation release-candidate closure gates.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-rc-closure-XXXXXX)"
  RED="$REPO_ROOT/scripts/autospec-red-row-burndown.sh"
  XREPO="$REPO_ROOT/scripts/autospec-cross-repo-compatibility.sh"
  CHECKS="$REPO_ROOT/scripts/autospec-check-type-coverage.sh"
  TEMPLATES="$REPO_ROOT/scripts/autospec-template-coverage.sh"
  CONTRACT="$REPO_ROOT/scripts/autospec-command-contract-check.sh"
  QUALITY="$REPO_ROOT/scripts/autospec-report-quality.sh"
  RC="$REPO_ROOT/scripts/autospec-release-candidate-gate.sh"
  DOGFOOD="$REPO_ROOT/scripts/autospec-dogfood-rc.sh"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/reports" "$repo/.autospec/state" "$repo/.autospec/templates" "$repo/docs/specs" "$repo/docs/runbooks" "$repo/scripts"
  cp -R "$REPO_ROOT/.autospec/templates/architecture" "$repo/.autospec/templates/"
  cp -R "$REPO_ROOT/.autospec/templates/ai-platform" "$repo/.autospec/templates/"
  cp -R "$REPO_ROOT/.autospec/templates/product-baseline" "$repo/.autospec/templates/"
  cat > "$repo/.autospec/reports/spec-coverage.json" <<'JSON'
{"schema":1,"requirements":[
  {"id":"engine.core","title":"Core engine","category":"policy","priority":"critical","status":"implemented","requirement_type":"engine","risk":"low"},
  {"id":"digital_twin.surfaces","title":"Surfaces","category":"digital_twin","priority":"high","status":"partial","requirement_type":"engine","risk":"medium"},
  {"id":"ai.runtime","title":"AI runtime","category":"ai_platform","priority":"high","status":"implemented","requirement_type":"target_app_scaffold","risk":"high"},
  {"id":"automation.scheduler","title":"Scheduler","category":"autonomous_development","priority":"critical","status":"deferred","requirement_type":"documentation","risk":"high"}
]}
JSON
  cp "$repo/.autospec/reports/spec-coverage.json" "$repo/.autospec/state/master-requirements.json"
  cat > "$repo/.autospec/reports/doctrine-audit.json" <<'JSON'
{"schema":1,"scorecard":{"ai":{"findings":1}},"findings":[{"category":"ai","check":"token_usage","status":"fail","summary":"Token usage evidence missing","missing_evidence":["token usage"]}],"side_effects":{"github_writes":false}}
JSON
  cat > "$repo/.autospec/reports/mvp-status.json" <<'JSON'
{"schema":1,"readiness":"MVP_READY_WITH_WARNINGS","spec_coverage":{"critical_missing_requirements":0,"mvp_readiness_impact":"warnings_only"}}
JSON
  cat > "$repo/.autospec/reports/mvp-smoke.json" <<'JSON'
{"schema":1,"verdict":"pass_with_warnings"}
JSON
  cat > "$repo/.autospec/reports/autonomy-v2-status.json" <<'JSON'
{"schema":1,"summary":{"status":"pass"},"worker_capabilities":1}
JSON
  cat > "$repo/.autospec/reports/runtime-feature-status.json" <<'JSON'
{"schema":1,"supported_adapters":["react-vite"],"generated_runtime_features":[]}
JSON
  cat > "$repo/.autospec/reports/runtime-evidence-status.json" <<'JSON'
{"schema":1,"summary":{"status":"pass"}}
JSON
  cat > "$repo/.autospec/reports/autonomy-v3-status.json" <<'JSON'
{"schema":1,"summary":{"status":"pass"}}
JSON
  cat > "$repo/.autospec/reports/policy-compatibility.json" <<'JSON'
{"schema":1,"unsupported_check_types":[],"summary":{"unsupported_check_types":0}}
JSON
  cat > "$repo/.autospec/reports/constitution-audit.json" <<'JSON'
{"schema":1,"status":"pass","required_failures":[]}
JSON
  cat > "$repo/.autospec/reports/rule-check-results.json" <<'JSON'
{"schema":1,"results":[{"rule_id":"engine.core","check_type":"required_file","status":"pass","severity":"required","category":"policy"}]}
JSON
  cat > "$repo/.autospec/reports/issue-plan-v3.json" <<'JSON'
{"schema":1,"issues":[]}
JSON
  cat > "$repo/.autospec/reports/example.json" <<'JSON'
{
  "schema": 1,
  "status": "pass"
}
JSON
  cat > "$repo/.autospec/reports/example.md" <<'MD'
# Example Report

## Summary

Everything is readable.

## Next steps

- Continue.
MD
  cat > "$repo/docs/KNOWN_LIMITATIONS.md" <<'MD'
# Known Limitations

## Deferred beyond the local validation foundation

- Scheduler/GitHub Actions support is optional future work only.
MD
  cat > "$repo/docs/specs/AUTOSPEC_CONSTITUTION_MASTER_SPEC.md" <<'MD'
# Master Spec

Autospec is operator-invoked and local by default.
MD
  cat > "$repo/docs/runbooks/COMMANDS.md" <<'MD'
# Commands

scripts/autospec-red-row-burndown.sh
scripts/autospec-release-candidate-gate.sh
MD
  cp "$REPO_ROOT/scripts/autospec-red-row-burndown.sh" "$repo/scripts/autospec-red-row-burndown.sh" 2>/dev/null || true
}

write_policy_repos() {
  local constitution="$1"
  local baselines="$2"
  mkdir -p "$constitution/manifests" "$constitution/rules" "$baselines/manifests" "$baselines/packs/web"
  cat > "$constitution/manifests/categories.yml" <<'YAML'
categories:
  - id: testing
YAML
  cat > "$constitution/rules/testing.yml" <<'YAML'
rules:
  - rule_id: testing.playwright
    title: Playwright
    category: testing
    severity: required
    maturity:
      level: production
    check:
      type: required_playwright_config
    quality_gates:
      - id: testing.viewport
        title: Viewport
YAML
  cat > "$baselines/manifests/profiles.yml" <<'YAML'
profiles:
  - id: web
    packs:
      - web/core
YAML
  cat > "$baselines/packs/web/core.yml" <<'YAML'
pack_id: web/core
category: testing
rules:
  - rule_id: web.docs
    title: Docs
    category: documentation
    severity: recommended
    check:
      type: required_doc
YAML
}

@test "red row burndown classifies gaps and writes local backlog" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"

  run bash "$RED" --repo-root "$TEST_TMPDIR/repo" --dry-run --priority critical,high

  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/red-row-burndown-plan.md" ]
  compgen -G "$TEST_TMPDIR/repo/.autospec/backlog/red-row/*.md" >/dev/null
  run jq -r '.classifications.fix_now_engine >= 0 and .side_effects.github_writes == false' "$TEST_TMPDIR/repo/.autospec/reports/red-row-burndown-plan.json"
  [ "$output" = "true" ]
}

@test "cross repo compatibility validates policy repos and reports unsupported features" {
  mkdir -p "$TEST_TMPDIR/repo" "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"
  write_repo "$TEST_TMPDIR/repo"
  write_policy_repos "$TEST_TMPDIR/constitution" "$TEST_TMPDIR/baselines"

  run bash "$XREPO" --repo-root "$TEST_TMPDIR/repo" --dry-run --constitution "$TEST_TMPDIR/constitution" --baselines "$TEST_TMPDIR/baselines"

  [ "$status" -eq 0 ]
  grep -q "Cross-Repo Compatibility" "$TEST_TMPDIR/repo/.autospec/reports/cross-repo-compatibility.md"
  run jq -r '.verdict' "$TEST_TMPDIR/repo/.autospec/reports/cross-repo-compatibility.json"
  [[ "$output" =~ ^(pass|pass_with_warnings)$ ]]
}

@test "check type and template coverage matrices are generated" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"

  run bash "$CHECKS" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  run jq -r '.matrix[] | select(.check_type=="required_file") | .status' "$TEST_TMPDIR/repo/.autospec/reports/check-type-coverage.json"
  [ "$output" = "complete" ]

  run bash "$TEMPLATES" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/template-coverage.md" ]
  run jq -r '.categories[] | select(.category=="architecture") | .status' "$TEST_TMPDIR/repo/.autospec/reports/template-coverage.json"
  [[ "$output" =~ ^(complete|partial)$ ]]
}

@test "command contract and report quality gates flag unsafe or unreadable artifacts" {
  mkdir -p "$TEST_TMPDIR/repo/scripts"
  write_repo "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/scripts/autospec-bad.sh" <<'SH'
#!/usr/bin/env bash
gh issue create --title bad
SH
  chmod +x "$TEST_TMPDIR/repo/scripts/autospec-bad.sh"
  fake_token="$(printf 'gh%s_%s%s' "p" "abcdefghijklmnopqrstuvwxyz" "123456")"
  printf '{"token":"%s"}\n' "$fake_token" > "$TEST_TMPDIR/repo/.autospec/reports/leak.json"

  run bash "$CONTRACT" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 1 ]
  grep -q "GitHub writes without confirm" "$TEST_TMPDIR/repo/.autospec/reports/command-contract-check.md"

  run bash "$QUALITY" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 1 ]
  grep -q "sensitive" "$TEST_TMPDIR/repo/.autospec/reports/report-quality.md"
}

@test "release candidate gate reports ready and blocks critical failures" {
  mkdir -p "$TEST_TMPDIR/ready" "$TEST_TMPDIR/blocked"
  write_repo "$TEST_TMPDIR/ready"
  write_repo "$TEST_TMPDIR/blocked"
  python3 - "$TEST_TMPDIR/blocked/.autospec/reports/spec-coverage.json" <<'PY'
import json, sys
p=sys.argv[1]
d=json.load(open(p))
d["requirements"].append({"id":"engine.missing","title":"Missing core","category":"policy","priority":"critical","status":"missing","requirement_type":"engine","risk":"high"})
json.dump(d, open(p,"w"), indent=2)
PY

  run bash "$RC" --repo-root "$TEST_TMPDIR/ready" --dry-run
  [ "$status" -eq 0 ]
  run jq -r '.verdict' "$TEST_TMPDIR/ready/.autospec/reports/release-candidate-gate.json"
  [ "$output" = "RC_READY" ]

  run bash "$RC" --repo-root "$TEST_TMPDIR/blocked" --dry-run
  [ "$status" -eq 1 ]
  run jq -r '.verdict' "$TEST_TMPDIR/blocked/.autospec/reports/release-candidate-gate.json"
  [ "$output" = "RC_NOT_READY" ]
}

@test "dogfood rc dry-run handles missing sibling repos with setup guidance" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"

  run bash "$DOGFOOD" --repo-root "$TEST_TMPDIR/repo" --dry-run --constitution "$TEST_TMPDIR/missing-constitution" --baselines "$TEST_TMPDIR/missing-baselines"

  [ "$status" -eq 0 ]
  grep -q "configure" "$TEST_TMPDIR/repo/.autospec/reports/dogfood-rc.md"
  run jq -r '.side_effects.github_writes' "$TEST_TMPDIR/repo/.autospec/reports/dogfood-rc.json"
  [ "$output" = "false" ]
}

@test "release notes manifest roadmap and runbook quickstart exist" {
  [ -f "$REPO_ROOT/docs/RELEASE_NOTES.md" ]
  [ -f "$REPO_ROOT/docs/ROADMAP_AFTER_MVP.md" ]
  grep -q "Autospec Local Validation Foundation" "$REPO_ROOT/docs/RELEASE_NOTES.md"
  grep -q "bash scripts/autospec-release-candidate-gate.sh --dry-run" "$REPO_ROOT/docs/runbooks/MVP_WALKTHROUGH.md"
  grep -q "Dry-run first" "$REPO_ROOT/docs/runbooks/MVP_WALKTHROUGH.md"
  grep -q "Confirm required for writes" "$REPO_ROOT/docs/runbooks/MVP_WALKTHROUGH.md"
  grep -q "auto-merge" "$REPO_ROOT/docs/runbooks/MVP_WALKTHROUGH.md"
  grep -q "optional future work" "$REPO_ROOT/docs/ROADMAP_AFTER_MVP.md"
}
