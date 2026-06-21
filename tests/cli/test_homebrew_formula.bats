#!/usr/bin/env bats
# tests/cli/test_homebrew_formula.bats
#
# Smoke tests for the Homebrew formula and release workflow.
# brew-audit test skips when brew binary is absent (CI Linux runners).
#
# Run: bats tests/cli/test_homebrew_formula.bats

FORMULA="dist/homebrew/autospec.rb"
WORKFLOW=".github/workflows/release-cli.yml"

# Resolve repo root relative to this file
REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"

@test "formula file exists" {
  [ -f "$REPO_ROOT/$FORMULA" ]
}

@test "formula parses as valid Ruby" {
  command -v ruby >/dev/null 2>&1 || skip "ruby not found"
  ruby -c "$REPO_ROOT/$FORMULA"
}

@test "formula contains required fields: desc, homepage, url, sha256, license" {
  [ -f "$REPO_ROOT/$FORMULA" ]
  grep -q 'desc ' "$REPO_ROOT/$FORMULA"
  grep -q 'homepage ' "$REPO_ROOT/$FORMULA"
  grep -q 'url ' "$REPO_ROOT/$FORMULA"
  grep -q 'sha256 ' "$REPO_ROOT/$FORMULA"
  grep -q 'license ' "$REPO_ROOT/$FORMULA"
}

@test "formula depends_on node" {
  grep -q 'depends_on "node"' "$REPO_ROOT/$FORMULA"
}

@test "formula has install and test blocks" {
  grep -q 'def install' "$REPO_ROOT/$FORMULA"
  grep -q 'test do' "$REPO_ROOT/$FORMULA"
}

@test "formula install block references libexec" {
  grep -q 'libexec' "$REPO_ROOT/$FORMULA"
}

@test "release workflow file exists" {
  [ -f "$REPO_ROOT/$WORKFLOW" ]
}

@test "release workflow is valid YAML" {
  if command -v python3 >/dev/null 2>&1 && python3 -c "import yaml" 2>/dev/null; then
    python3 - "$REPO_ROOT/$WORKFLOW" << 'PYEOF'
import yaml, sys
yaml.safe_load(open(sys.argv[1]))
PYEOF
  elif command -v yamllint >/dev/null 2>&1; then
    yamllint "$REPO_ROOT/$WORKFLOW"
  else
    echo "no YAML validator (install python3-yaml or yamllint) — cannot validate workflow YAML" >&2
    return 1
  fi
}

@test "release workflow triggers on v* tags" {
  grep -q "tags:" "$REPO_ROOT/$WORKFLOW"
  grep -q "'v\*'" "$REPO_ROOT/$WORKFLOW"
}

@test "release workflow has npm publish step" {
  grep -q "npm publish" "$REPO_ROOT/$WORKFLOW"
}

@test "release workflow references NPM_TOKEN secret" {
  grep -q "NPM_TOKEN" "$REPO_ROOT/$WORKFLOW"
}

@test "release workflow references TAP_REPO_TOKEN secret" {
  grep -q "TAP_REPO_TOKEN" "$REPO_ROOT/$WORKFLOW"
}

@test "brew audit passes (skip if brew absent or formula not tapped)" {
  command -v brew >/dev/null 2>&1 || skip "brew not found — skipping audit"
  # brew audit only accepts formula names, not file paths.
  # We can only run it after the formula is installed via a tap.
  # At PR time, the tap does not yet exist, so skip gracefully.
  brew tap berlinguyinca/autospec 2>/dev/null || skip "tap berlinguyinca/autospec not available — skipping audit"
  run brew audit --strict berlinguyinca/autospec/autospec
  [ "$status" -eq 0 ]
}
