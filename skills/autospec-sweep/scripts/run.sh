#!/usr/bin/env bash
# run.sh — executable autospec-sweep runner.

set -eu

usage() {
  cat <<'EOF'
Usage:
  run.sh run [--repo-root DIR] [--dry-run] [--gaps FILE] [--since DATE] [--no-file]

Loads .autospec/autospec.yml, writes .autospec/sweep/latest.json, and, when
possible, hands emitted gaps to gap-remediation-loop.sh for issue filing.

Environment:
  AUTOSPEC_SWEEP_REVIEW_CMD  command to run with --emit-gaps <path>
  AUTOSPEC_SCRIPTS_DIR       installed helper script directory
EOF
}

fail() {
  printf 'autospec-sweep run: %s\n' "$*" >&2
  exit 1
}

refuse() {
  printf 'autospec-sweep run: %s\n' "$*" >&2
  exit 2
}

json_string() {
  jq -Rn --arg v "$1" '$v'
}

require_tool() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 not found on PATH"
}

is_missing_command() {
  case "$1" in
    ""|TODO:*) return 0 ;;
    *) return 1 ;;
  esac
}

run_sweep_command() {
  label="$1"
  command_text="$2"
  log_file="$3"

  printf 'autospec-sweep run: running %s: %s\n' "$label" "$command_text"
  {
    printf '\n[%s] %s\n' "$label" "$command_text"
    cd "$REPO_ROOT" && bash -lc "$command_text"
  } >> "$log_file" 2>&1
}

SUBCOMMAND="${1:-}"
if [ "$SUBCOMMAND" != "run" ]; then
  usage
  exit 2
fi
shift

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$PWD"
DRY_RUN=0
NO_FILE=0
SINCE=""
INPUT_GAPS=""

while [ $# -gt 0 ]; do
  case "$1" in
    --repo-root)
      REPO_ROOT="${2:-}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --no-file)
      NO_FILE=1
      shift
      ;;
    --since)
      SINCE="${2:-}"
      shift 2
      ;;
    --gaps)
      INPUT_GAPS="${2:-}"
      shift 2
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
CONFIG="$REPO_ROOT/.autospec/autospec.yml"
STATE_DIR="$REPO_ROOT/.autospec/sweep"
LATEST="$STATE_DIR/latest.json"
GAPS_OUT="$STATE_DIR/gaps.json"

require_tool jq
require_tool yq

[ -f "$CONFIG" ] || refuse "missing .autospec/autospec.yml; run /autospec-sweep init first"

if ! yq '.' "$CONFIG" >/dev/null 2>&1; then
  fail "failed to parse .autospec/autospec.yml"
fi

if command -v ajv >/dev/null 2>&1 && [ -f "$REPO_ROOT/schemas/autospec-config.schema.json" ]; then
  tmp_base="$(mktemp -t autospec-sweep-config.XXXXXX)"
  tmp_json="${tmp_base}.json"
  trap 'rm -f "$tmp_base" "$tmp_json"' EXIT
  yq -o=json '.' "$CONFIG" > "$tmp_json"
  ajv validate -s "$REPO_ROOT/schemas/autospec-config.schema.json" --spec=draft2020 -d "$tmp_json" >/dev/null \
    || fail ".autospec/autospec.yml does not match schemas/autospec-config.schema.json"
fi

review_enabled="$(yq -r '.steps.review.enabled != false' "$CONFIG")"
run_enabled="$(yq -r '.steps.run.enabled != false' "$CONFIG")"
spec_sync_enabled="$(yq -r '.sweep.spec_sync.enabled != false' "$CONFIG")"
file_issues="$(yq -r '.continuous_improvement.loop.file_issues != false' "$CONFIG")"
route_run="$(yq -r '.continuous_improvement.loop.route_fixes_via_autospec_run != false' "$CONFIG")"
max_issues="$(yq -r '.sweep.improvement_budget.max_issues_per_sweep // 5' "$CONFIG")"
tests_enabled="$(yq -r '.continuous_improvement.tests.enabled != false' "$CONFIG")"
run_all_tests="$(yq -r '.execution.tests.run_all_every_sweep != false' "$CONFIG")"
deploy_if_tests_require="$(yq -r '.execution.deployment.deploy_if_tests_require != false' "$CONFIG")"
test_cmd="$(yq -r '.project.findings.commands.test // ""' "$CONFIG")"
e2e_cmd="$(yq -r '.project.findings.commands.e2e // ""' "$CONFIG")"
deploy_cmd="$(yq -r '.project.findings.commands.deploy // ""' "$CONFIG")"

helper_dir="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
if [ -n "${AUTOSPEC_SWEEP_REVIEW_CMD:-}" ]; then
  review_cmd="$AUTOSPEC_SWEEP_REVIEW_CMD"
elif [ -x "$SCRIPT_DIR/review.sh" ]; then
  review_cmd="$SCRIPT_DIR/review.sh"
elif [ -x "$helper_dir/autospec-sweep-review.sh" ]; then
  review_cmd="$helper_dir/autospec-sweep-review.sh"
else
  review_cmd="autospec-review"
fi
review_args="--repo-root $REPO_ROOT --emit-gaps $GAPS_OUT"
[ -z "$SINCE" ] || review_args="--repo-root $REPO_ROOT --since $SINCE --emit-gaps $GAPS_OUT"
remediation_cmd="\${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gap-remediation-loop.sh --gaps $GAPS_OUT --file"
run_cmd="autospec-run"

if [ "$DRY_RUN" -eq 1 ]; then
  jq -n \
    --arg config ".autospec/autospec.yml" \
    --arg review "$review_cmd $review_args" \
    --arg remediation "$remediation_cmd" \
    --arg run "$run_cmd" \
    --arg test "$test_cmd" \
    --arg e2e "$e2e_cmd" \
    --arg deploy "$deploy_cmd" \
    --argjson review_enabled "$review_enabled" \
    --argjson run_enabled "$run_enabled" \
    --argjson spec_sync_enabled "$spec_sync_enabled" \
    --argjson tests_enabled "$tests_enabled" \
    --argjson run_all_tests "$run_all_tests" \
    --argjson deploy_if_tests_require "$deploy_if_tests_require" \
    --argjson max_issues "$max_issues" \
    '{
      mode: "dry-run",
      config: $config,
      enabled: {
        review: $review_enabled,
        run: $run_enabled,
        spec_sync: $spec_sync_enabled,
        tests: $tests_enabled
      },
      execution: {
        run_all_tests_every_sweep: $run_all_tests,
        deploy_if_tests_require: $deploy_if_tests_require
      },
      improvement_budget: {max_issues_per_sweep: $max_issues},
      commands: {
        deploy: $deploy,
        test: $test,
        e2e: $e2e,
        review: $review,
        remediation: $remediation,
        run: $run
      }
    }'
  exit 0
fi

mkdir -p "$STATE_DIR"
COMMAND_LOG="$STATE_DIR/commands.log"
: > "$COMMAND_LOG"
printf '[]\n' > "$GAPS_OUT"

tests_status="skipped"
deployment_status="skipped"

if [ "$tests_enabled" = "true" ] && [ "$run_all_tests" = "true" ]; then
  tests_status="pass"

  if ! is_missing_command "$e2e_cmd" && [ "$deploy_if_tests_require" = "true" ]; then
    if is_missing_command "$deploy_cmd"; then
      refuse "E2E/integration tests are configured but project.findings.commands.deploy is missing"
    fi
    if run_sweep_command "deploy" "$deploy_cmd" "$COMMAND_LOG"; then
      deployment_status="pass"
    else
      deployment_status="fail"
      fail "deployment command failed; see .autospec/sweep/commands.log"
    fi
  fi

  if ! is_missing_command "$test_cmd"; then
    run_sweep_command "test" "$test_cmd" "$COMMAND_LOG" \
      || { tests_status="fail"; fail "test command failed; see .autospec/sweep/commands.log"; }
  fi

  if ! is_missing_command "$e2e_cmd"; then
    run_sweep_command "e2e" "$e2e_cmd" "$COMMAND_LOG" \
      || { tests_status="fail"; fail "e2e command failed; see .autospec/sweep/commands.log"; }
  fi
fi

if [ -n "$INPUT_GAPS" ]; then
  [ -f "$INPUT_GAPS" ] || refuse "gaps file not found: $INPUT_GAPS"
  jq -e 'type=="array"' "$INPUT_GAPS" >/dev/null 2>&1 || fail "gaps file must be a JSON array"
  cp "$INPUT_GAPS" "$GAPS_OUT"
elif [ "$review_enabled" = "true" ]; then
  if command -v "$review_cmd" >/dev/null 2>&1; then
    "$review_cmd" $review_args || printf 'autospec-sweep run: WARN review command failed; continuing with empty gaps\n' >&2
  elif [ -x "$review_cmd" ]; then
    "$review_cmd" $review_args || printf 'autospec-sweep run: WARN review command failed; continuing with empty gaps\n' >&2
  else
    printf 'autospec-sweep run: WARN review command not found: %s; wrote empty gaps\n' "$review_cmd" >&2
  fi
fi

if ! jq -e 'type=="array"' "$GAPS_OUT" >/dev/null 2>&1; then
  printf 'autospec-sweep run: WARN review output was not a JSON array; replacing with empty gaps\n' >&2
  printf '[]\n' > "$GAPS_OUT"
fi

gap_count="$(jq 'length' "$GAPS_OUT")"
file_status="skipped"
loop_helper="$helper_dir/gap-remediation-loop.sh"

if [ "$gap_count" -gt 0 ] && [ "$NO_FILE" -eq 0 ] && [ "$file_issues" = "true" ]; then
  if [ -x "$loop_helper" ]; then
    AUTOSPEC_GAP_MAX_ROUNDS="${AUTOSPEC_GAP_MAX_ROUNDS:-$max_issues}" "$loop_helper" --gaps "$GAPS_OUT" --file
    file_status="filed"
  elif [ -x "$SCRIPT_DIR/../../autospec-shared/scripts/gap-remediation-loop.sh" ]; then
    AUTOSPEC_GAP_MAX_ROUNDS="${AUTOSPEC_GAP_MAX_ROUNDS:-$max_issues}" "$SCRIPT_DIR/../../autospec-shared/scripts/gap-remediation-loop.sh" --gaps "$GAPS_OUT" --file
    file_status="filed"
  else
    printf 'autospec-sweep run: WARN gap-remediation-loop.sh not found; gaps written but not filed\n' >&2
    file_status="helper_missing"
  fi
fi

generated_at="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
jq -n \
  --arg generated_at "$generated_at" \
  --arg repo_root "$REPO_ROOT" \
  --arg config ".autospec/autospec.yml" \
  --arg gaps ".autospec/sweep/gaps.json" \
  --arg file_status "$file_status" \
  --arg tests_status "$tests_status" \
  --arg deployment_status "$deployment_status" \
  --argjson gap_count "$gap_count" \
  --argjson review_enabled "$review_enabled" \
  --argjson run_enabled "$run_enabled" \
  --argjson route_run "$route_run" \
  '{
    generated_at: $generated_at,
    repo_root: $repo_root,
    config: $config,
    gaps: {
      path: $gaps,
      count: $gap_count,
      file_status: $file_status
    },
    enabled: {
      review: $review_enabled,
      run: $run_enabled,
      route_fixes_via_autospec_run: $route_run
    },
    tests: {
      status: $tests_status,
      log: ".autospec/sweep/commands.log"
    },
    deployment: {
      status: $deployment_status,
      log: ".autospec/sweep/commands.log"
    }
  }' > "$LATEST"

printf 'autospec-sweep run: wrote %s\n' "$LATEST"
printf 'autospec-sweep run: gaps=%s file_status=%s\n' "$gap_count" "$file_status"
if [ "$gap_count" -gt 0 ] && [ "$route_run" = "true" ] && [ "$run_enabled" = "true" ]; then
  printf 'autospec-sweep run: next step: /autospec-run\n'
fi
