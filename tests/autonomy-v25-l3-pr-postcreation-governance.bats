#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs" 2>/dev/null || mkdir -p "$TEST_TMP/repo/docs"
  cp -R "$REPO_ROOT/tests" "$TEST_TMP/repo/tests"
  mkdir -p "$TEST_TMP/repo/examples" "$TEST_TMP/repo/.autospec/reports"
  printf '# Fixture AutoSpec\n\nA fixture README.\n' > "$TEST_TMP/repo/README.md"
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "v25 requested validation commands generate baseline artifacts" {
  run bash "$TEST_TMP/repo/scripts/autospec-spec-coverage.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  run bash "$TEST_TMP/repo/scripts/autospec-release-validation.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  run bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"V25_BASELINE_READY=true"* ]]

  [ -f "$TEST_TMP/repo/.autospec/reports/repository-audit.md" ]
  [ -f "$TEST_TMP/repo/.autospec/spec-index.json" ]
  [ -f "$TEST_TMP/repo/.autospec/spec-index.md" ]
  [ -f "$TEST_TMP/repo/.autospec/reports/dependency-validation.md" ]
  [ -f "$TEST_TMP/repo/.autospec/reports/documentation-coverage.md" ]
  [ -f "$TEST_TMP/repo/.autospec/reports/cli-audit.md" ]
  [ -f "$TEST_TMP/repo/.autospec/reports/test-matrix.md" ]
  [ -f "$TEST_TMP/repo/.autospec/baselines/performance.json" ]
  [ -f "$TEST_TMP/repo/.autospec/baselines/quality.json" ]
  [ -f "$TEST_TMP/repo/.autospec/baselines/v25-baseline.json" ]
  [ -f "$TEST_TMP/repo/.autospec/releases/v25.md" ]
  [ -f "$TEST_TMP/repo/.autospec/reports/autonomy-v25-status.json" ]
}

@test "v25 status is ready and reports clean safety proof" {
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  run bash "$TEST_TMP/repo/scripts/autospec-v25-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"V25_BASELINE_READY=true"* ]]

  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v25-status.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
assert data["status"] == "ready"
assert data["V25_BASELINE_READY"] is True
for key in [
    "network_attempted",
    "github_write_attempted",
    "git_push_attempted",
    "draft_pr_create_attempted",
    "issue_publishing_attempted",
    "merge_attempted",
    "approval_attempted",
    "self_approval_attempted",
    "default_branch_push_attempted",
    "raw_secret_values_exposed",
]:
    assert data[key] is False, key
assert data["scheduler"] == "absent"
assert data["daemon"] == "absent"
assert data["background_runner"] == "absent"
assert data["external_ai"] == "disabled_by_default"
PY
}

@test "v25 spec inventory assigns each spec to exactly one state" {
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/spec-index.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
states = {"implemented", "scaffolded", "validated", "deferred", "experimental", "superseded"}
assert data["summary"]["duplicate_assignments"] == 0
for item in data["specs"]:
    assert item["state"] in states
    assert isinstance(item["path"], str) and item["path"]
PY
}

@test "v25 spec inventory preserves state keyword precedence" {
  mkdir -p "$TEST_TMP/repo/docs/specs"
  cat > "$TEST_TMP/repo/docs/specs/state-precedence.md" <<'MD'
# State Precedence

This spec is superseded even though it also mentions experimental,
deferred, validated, scaffold, and template language.
MD
  cat > "$TEST_TMP/repo/docs/specs/validation-acceptance.md" <<'MD'
# Validation Acceptance

Acceptance evidence exists without any higher-priority state marker.
MD
  cat > "$TEST_TMP/repo/docs/specs/scaffold-template.md" <<'MD'
# Scaffold Template

Template and scaffold language remain classified as scaffolded.
MD

  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null

  python3 - "$TEST_TMP/repo/.autospec/spec-index.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
states = {item["path"]: item["state"] for item in data["specs"]}
assert states["docs/specs/state-precedence.md"] == "superseded"
assert states["docs/specs/validation-acceptance.md"] == "validated"
assert states["docs/specs/scaffold-template.md"] == "scaffolded"
PY
}

@test "state_for_spec is table-driven and no longer dogfood-allowlisted" {
  python3 - "$REPO_ROOT/scripts/autospec-baseline-v25.py" "$REPO_ROOT/tests/dogfood/allowlist/qa-brute-force-sweep.json" <<'PY'
import ast
import json
import sys
from pathlib import Path

source_path = Path(sys.argv[1])
allowlist_path = Path(sys.argv[2])
module = ast.parse(source_path.read_text(encoding="utf-8"))
state_for_spec = next(
    node for node in module.body
    if isinstance(node, ast.FunctionDef) and node.name == "state_for_spec"
)
branch_count = sum(isinstance(node, ast.If) for node in ast.walk(state_for_spec))
assert branch_count <= 1, f"state_for_spec still has {branch_count} explicit if branches"

entries = json.loads(allowlist_path.read_text(encoding="utf-8"))
assert not any(
    entry.get("file") == "scripts/autospec-baseline-v25.py"
    and entry.get("function") == "state_for_spec"
    and entry.get("rule_id") == "REPEATED_STRUCTURE_AS_CODE"
    for entry in entries
), "state_for_spec REPEATED_STRUCTURE_AS_CODE dogfood allowlist entry remains"
PY
}

@test "generic_gate is table-driven and no longer dogfood-allowlisted" {
  python3 - "$REPO_ROOT/scripts/autospec-baseline-v25.py" "$REPO_ROOT/tests/dogfood/allowlist/qa-brute-force-sweep.json" <<'PY'
import ast
import json
import sys
from pathlib import Path

source_path = Path(sys.argv[1])
allowlist_path = Path(sys.argv[2])
module = ast.parse(source_path.read_text(encoding="utf-8"))
generic_gate = next(
    node for node in module.body
    if isinstance(node, ast.FunctionDef) and node.name == "generic_gate"
)
branch_count = sum(isinstance(node, ast.If) for node in ast.walk(generic_gate))
assert branch_count <= 4, f"generic_gate still has {branch_count} explicit if branches"

entries = json.loads(allowlist_path.read_text(encoding="utf-8"))
assert not any(
    entry.get("file") == "scripts/autospec-baseline-v25.py"
    and entry.get("function") == "generic_gate"
    and entry.get("rule_id") == "REPEATED_STRUCTURE_AS_CODE"
    for entry in entries
), "generic_gate REPEATED_STRUCTURE_AS_CODE dogfood allowlist entry remains"
PY
}

@test "run_repo_quality_audit is table-driven and no longer dogfood-allowlisted" {
  python3 - "$REPO_ROOT/crates/autospec-core/src/validation/external.rs" "$REPO_ROOT/tests/dogfood/allowlist/qa-brute-force-sweep.json" <<'PY'
import json
import sys
from pathlib import Path

source_path = Path(sys.argv[1])
allowlist_path = Path(sys.argv[2])
source = source_path.read_text(encoding="utf-8")
start = source.index("fn run_repo_quality_audit(")
end = source.index("\nfn run_autospec_autonomous_contract(", start)
function_source = source[start:end]

assert "REPO_QUALITY_CONTRACTS" in function_source, "repo-quality checks should be table-dispatched"
assert function_source.count("return aggregate(") <= 1, "repo-quality audit still repeats failure branches"

entries = json.loads(allowlist_path.read_text(encoding="utf-8"))
assert not any(
    entry.get("file") == "crates/autospec-core/src/validation/external.rs"
    and entry.get("function") == "run_repo_quality_audit"
    and entry.get("rule_id") == "REPEATED_STRUCTURE_AS_CODE"
    for entry in entries
), "run_repo_quality_audit REPEATED_STRUCTURE_AS_CODE dogfood allowlist entry remains"
PY
}

@test "v25 dependency graph is acyclic and release validation has no blockers" {
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/reports/dependency-validation.json" "$TEST_TMP/repo/.autospec/reports/release-validation.json" <<'PY'
import json, sys
dep = json.load(open(sys.argv[1]))
rel = json.load(open(sys.argv[2]))
assert dep["acyclic"] is True
assert dep["blockers"] == []
assert rel["status"] == "pass"
assert rel["blockers"] == []
PY
}
