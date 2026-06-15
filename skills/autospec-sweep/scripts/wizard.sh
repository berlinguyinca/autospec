#!/usr/bin/env bash
# wizard.sh — autospec-sweep first-run configuration wizard.

set -eu

usage() {
  cat <<'EOF'
Usage:
  wizard.sh init [--repo-root DIR] [--answers FILE] [--dry-run] [--force]

Writes .autospec/autospec.yml with safe defaults for all autospec phases.
EOF
}

fail() {
  printf 'autospec-sweep wizard: %s\n' "$*" >&2
  exit 1
}

refuse() {
  printf 'autospec-sweep wizard: %s\n' "$*" >&2
  exit 2
}

require_tool() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 not found on PATH"
}

yaml_string() {
  printf '%s' "$1" | sed "s/'/''/g; s/^/'/; s/$/'/"
}

answer() {
  key="$1"
  default="$2"
  if [ -n "$ANSWERS" ] && [ -f "$ANSWERS" ]; then
    value="$(yq -r ".$key // \"\"" "$ANSWERS")"
    if [ -n "$value" ] && [ "$value" != "null" ]; then
      printf '%s\n' "$value"
      return 0
    fi
  fi
  printf '%s\n' "$default"
}

prompt_default() {
  label="$1"
  default="$2"
  if [ -t 0 ] && [ -t 1 ]; then
    printf '%s [%s]: ' "$label" "$default" >/dev/tty
    read -r value </dev/tty || value=""
    printf '%s\n' "${value:-$default}"
  else
    printf '%s\n' "$default"
  fi
}

append_unique() {
  value="$1"
  case " $STACK " in
    *" $value "*) ;;
    *) STACK="${STACK}${STACK:+ }${value}" ;;
  esac
}

SUBCOMMAND="${1:-}"
if [ "$SUBCOMMAND" != "init" ]; then
  usage
  exit 2
fi
shift

REPO_ROOT="$PWD"
ANSWERS=""
DRY_RUN=0
FORCE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --repo-root)
      REPO_ROOT="${2:-}"
      shift 2
      ;;
    --answers)
      ANSWERS="${2:-}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --force)
      FORCE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      refuse "unknown flag: $1"
      ;;
  esac
done

[ -d "$REPO_ROOT" ] || fail "repo root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd)"

require_tool yq
require_tool jq

CONFIG_DIR="$REPO_ROOT/.autospec"
CONFIG_PATH="$CONFIG_DIR/autospec.yml"

if [ -e "$CONFIG_PATH" ] && [ "$FORCE" -ne 1 ]; then
  refuse "$CONFIG_PATH already exists; rerun with --force to replace it"
fi

PROFILE="$(answer profile full)"
SAFETY="$(answer safety strict_isolation)"
TEAM="$(answer team auto)"
ALLOW_COMPETITOR_RESEARCH="$(answer allow_competitor_research false)"

if [ -z "$ANSWERS" ]; then
  printf 'autospec-sweep first-run setup\n'
  printf 'Press return to accept defaults. You can edit .autospec/autospec.yml later.\n\n'
  PROFILE="$(prompt_default 'Sweep profile (full|docs-tests-code|docs-only)' "$PROFILE")"
  SAFETY="$(prompt_default 'Environment safety (strict_isolation|scoped_production)' "$SAFETY")"
  TEAM="$(prompt_default 'Team personality (auto or short team name)' "$TEAM")"
  ALLOW_COMPETITOR_RESEARCH="$(prompt_default 'Allow competitor research when useful (false|true)' "$ALLOW_COMPETITOR_RESEARCH")"
fi

case "$SAFETY" in
  strict_isolation|scoped_production) ;;
  *) refuse "safety must be strict_isolation or scoped_production" ;;
esac

case "$ALLOW_COMPETITOR_RESEARCH" in
  true|false) ;;
  *) refuse "allow_competitor_research must be true or false" ;;
esac

if git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  if git -C "$REPO_ROOT" check-ignore -q ".autospec/autospec.yml" 2>/dev/null; then
    refuse ".autospec/autospec.yml is ignored by git; unignore it before running autospec-sweep init"
  fi
fi

STACK=""
TEST_COMMAND=""
E2E_COMMAND=""
DEPLOY_COMMAND=""
QUESTIONS=""

if [ -f "$REPO_ROOT/package.json" ]; then
  append_unique node
  TEST_COMMAND="$(jq -r '.scripts.test // empty' "$REPO_ROOT/package.json" 2>/dev/null || true)"
  E2E_COMMAND="$(jq -r '.scripts.e2e // .scripts["test:e2e"] // empty' "$REPO_ROOT/package.json" 2>/dev/null || true)"
  DEPLOY_COMMAND="$(jq -r '.scripts.deploy // .scripts.start // .scripts.dev // empty' "$REPO_ROOT/package.json" 2>/dev/null || true)"
fi

if ls "$REPO_ROOT"/playwright.config.* >/dev/null 2>&1; then
  append_unique playwright
  QUESTIONS="${QUESTIONS}${QUESTIONS:+
}What is the canonical local or staging base URL for browser E2E sweeps?"
  if [ -z "$DEPLOY_COMMAND" ]; then
    QUESTIONS="${QUESTIONS}${QUESTIONS:+
}Which deploy/start command should autospec run before E2E tests?"
  fi
fi

if [ -f "$REPO_ROOT/pyproject.toml" ] || [ -f "$REPO_ROOT/requirements.txt" ]; then
  append_unique python
  [ -n "$TEST_COMMAND" ] || TEST_COMMAND="pytest"
fi

if [ -f "$REPO_ROOT/go.mod" ]; then
  append_unique go
  [ -n "$TEST_COMMAND" ] || TEST_COMMAND="go test ./..."
fi

if [ -f "$REPO_ROOT/Cargo.toml" ]; then
  append_unique rust
  [ -n "$TEST_COMMAND" ] || TEST_COMMAND="cargo test"
fi

if [ -f "$REPO_ROOT/pom.xml" ]; then
  append_unique jvm
  [ -n "$TEST_COMMAND" ] || TEST_COMMAND="mvn test"
fi

if [ -z "$TEST_COMMAND" ]; then
  QUESTIONS="${QUESTIONS}${QUESTIONS:+
}Which fast test command should autospec use for the inner-loop smoke test?"
fi

[ -n "$STACK" ] || STACK="unknown"
[ -n "$TEST_COMMAND" ] || TEST_COMMAND="TODO: set project test command"
[ -n "$E2E_COMMAND" ] || E2E_COMMAND="TODO: set project E2E command"
[ -n "$DEPLOY_COMMAND" ] || DEPLOY_COMMAND="TODO: set deploy/start command for E2E tests"

# Loud warning: any TODO: stub means the sweep runs in DEGRADED mode (the
# corresponding command is silently skipped by review.sh). Never let this pass
# unnoticed — the operator must fill these in before autospec-sweep can act.
_sweep_stub_warns=""
case "$TEST_COMMAND"   in TODO:*) _sweep_stub_warns="${_sweep_stub_warns} test";;   esac
case "$E2E_COMMAND"    in TODO:*) _sweep_stub_warns="${_sweep_stub_warns} e2e";;    esac
case "$DEPLOY_COMMAND" in TODO:*) _sweep_stub_warns="${_sweep_stub_warns} deploy";; esac
if [ -n "$_sweep_stub_warns" ]; then
  printf 'autospec-sweep: WARN — could not auto-detect command(s):%s. Written as TODO: stubs in .autospec/autospec.yml and SKIPPED (degraded mode) until you set them.\n' "$_sweep_stub_warns" >&2
fi

TMP_CONFIG="$(mktemp -t autospec-sweep-config.XXXXXX)"
trap 'rm -f "$TMP_CONFIG"' EXIT

{
  printf 'version: 1\n'
  printf 'generated_by: autospec-sweep\n'
  printf 'config_path: .autospec/autospec.yml\n'
  printf 'git:\n'
  printf '  tracked: true\n'
  printf '  commit_required: true\n'
  printf 'project:\n'
  printf '  profile: %s\n' "$(yaml_string "$PROFILE")"
  printf '  team_personality: %s\n' "$(yaml_string "$TEAM")"
  printf '  findings:\n'
  printf '    stack:\n'
  for item in $STACK; do
    printf '      - %s\n' "$(yaml_string "$item")"
  done
  printf '    commands:\n'
  printf '      test: %s\n' "$(yaml_string "$TEST_COMMAND")"
  printf '      e2e: %s\n' "$(yaml_string "$E2E_COMMAND")"
  printf '      deploy: %s\n' "$(yaml_string "$DEPLOY_COMMAND")"
  printf '  questions:\n'
  if [ -n "$QUESTIONS" ]; then
    printf '%s\n' "$QUESTIONS" | while IFS= read -r question; do
      [ -n "$question" ] || continue
      printf '    - %s\n' "$(yaml_string "$question")"
    done
  else
    printf '    - %s\n' "$(yaml_string "Which product surfaces are most important for the first sweep?")"
  fi
  printf 'steps:\n'
  for step in define classify run review test clone sweep; do
    printf '  %s:\n' "$step"
    printf '    enabled: true\n'
  done
  printf 'sweep:\n'
  printf '  cadence: end_of_run\n'
  printf '  profile: %s\n' "$(yaml_string "$PROFILE")"
  printf '  spec_sync:\n'
  printf '    enabled: true\n'
  printf '    mode: reality_check\n'
  printf '    update_specs_before_code: true\n'
  printf '    create_gap_issues: true\n'
  printf '  competitor_research:\n'
  printf '    enabled: %s\n' "$ALLOW_COMPETITOR_RESEARCH"
  printf '    sources: []\n'
  printf '  improvement_budget:\n'
  printf '    max_issues_per_sweep: 5\n'
  printf '    prefer_small_reviewable_changes: true\n'
  printf 'continuous_improvement:\n'
  printf '  docs:\n'
  printf '    enabled: true\n'
  printf '    checks: [doc_drift, user_manual, api_reference, runbooks]\n'
  printf '  tests:\n'
  printf '    enabled: true\n'
  printf '    checks: [coverage_gaps, e2e_surface_gaps, regression_gaps]\n'
  printf '  code:\n'
  printf '    enabled: true\n'
  printf '    checks: [complexity, duplication, dead_code, security_footguns]\n'
  printf '  loop:\n'
  printf '    create_or_update_specs: true\n'
  printf '    file_issues: true\n'
  printf '    route_fixes_via_autospec_run: true\n'
  printf 'documentation:\n'
  printf '  enabled: true\n'
  printf '  audiences:\n'
  printf '    - id: users\n'
  printf '      label: End users\n'
  printf '      path: docs/USER_MANUAL.md\n'
  printf '      focus: Installation, daily workflows, troubleshooting, and success criteria.\n'
  printf '      require_scope: true\n'
  printf '    - id: developers\n'
  printf '      label: Developers\n'
  printf '      path: docs/API_REFERENCE.md\n'
  printf '      focus: CLI, helper scripts, config schema, and extension points.\n'
  printf '      require_scope: true\n'
  printf '    - id: operators\n'
  printf '      label: Operators\n'
  printf '      path: docs/runbooks/OPERATIONS.md\n'
  printf '      focus: Deployment, monitoring, recovery, scheduling, and safe rollback.\n'
  printf '      require_scope: true\n'
  printf '    - id: security-reviewers\n'
  printf '      label: Security reviewers\n'
  printf '      path: docs/SECURITY.md\n'
  printf '      focus: Trust boundaries, secret handling, production access, and abuse cases.\n'
  printf '      require_scope: true\n'
  printf '  scopes:\n'
  printf '    - id: repository-overview\n'
  printf '      label: Repository overview\n'
  printf '      path: README.md\n'
  printf '      focus: What autospec does, which skill to use, and the supported workflow.\n'
  printf '      require_scope: false\n'
  printf '    - id: user-workflows\n'
  printf '      label: User workflows\n'
  printf '      path: docs/USER_MANUAL.md\n'
  printf '      focus: Invocation flows, setup, stop/resume, sweep, review, and run behavior.\n'
  printf '      require_scope: true\n'
  printf '    - id: api-reference\n'
  printf '      label: API and script reference\n'
  printf '      path: docs/API_REFERENCE.md\n'
  printf '      focus: Installed helper scripts, config fields, command flags, and exit codes.\n'
  printf '      require_scope: true\n'
  printf '    - id: operations-runbooks\n'
  printf '      label: Operations runbooks\n'
  printf '      path: docs/runbooks/OPERATIONS.md\n'
  printf '      focus: Daemon operation, concurrency, CI waiting, quota recovery, and incident response.\n'
  printf '      require_scope: true\n'
  printf '    - id: troubleshooting\n'
  printf '      label: Troubleshooting\n'
  printf '      path: docs/TROUBLESHOOTING.md\n'
  printf '      focus: Common failures, diagnostics, recovery commands, and escalation paths.\n'
  printf '      require_scope: true\n'
  printf 'execution:\n'
  printf '  tests:\n'
  printf '    run_all_every_sweep: true\n'
  printf '    fail_sweep_on_test_failure: true\n'
  printf '  deployment:\n'
  printf '    deploy_if_tests_require: true\n'
  printf '    required_for: [e2e]\n'
  printf 'safety:\n'
  printf '  production_access: %s\n' "$(yaml_string "$SAFETY")"
  printf '  secrets_policy: references_only\n'
  printf '  never_clone_raw_secrets: true\n'
  printf 'compatibility:\n'
  printf '  legacy_skill_contracts: read_if_present\n'
  printf '  source_of_truth: .autospec/autospec.yml\n'
} > "$TMP_CONFIG"

if ! yq '.' "$TMP_CONFIG" >/dev/null; then
  fail "generated config is not valid YAML"
fi

if [ "$DRY_RUN" -eq 1 ]; then
  cat "$TMP_CONFIG"
  exit 0
fi

mkdir -p "$CONFIG_DIR"
cp "$TMP_CONFIG" "$CONFIG_PATH"
printf 'autospec-sweep wizard: wrote %s\n' "$CONFIG_PATH"
printf 'autospec-sweep wizard: commit .autospec/autospec.yml so every agent uses the same workflow defaults.\n'
