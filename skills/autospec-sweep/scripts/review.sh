#!/usr/bin/env bash
# review.sh — deterministic autospec-sweep review wrapper.

set -eu

usage() {
  cat <<'EOF'
Usage:
  review.sh --repo-root DIR --emit-gaps FILE [--since DATE]

Emits gap JSON for autospec-sweep without requiring a slash-command harness.
EOF
}

fail() {
  printf 'autospec-sweep review: %s\n' "$*" >&2
  exit 1
}

json_escape() {
  jq -Rn --arg v "$1" '$v'
}

add_gap() {
  dimension="$1"
  severity="$2"
  file="$3"
  line="$4"
  title="$5"
  body="$6"
  dedupe_key="$7"

  tmp_gap="$(mktemp -t autospec-sweep-gap.XXXXXX)"
  jq -n \
    --arg dimension "$dimension" \
    --arg severity "$severity" \
    --arg file "$file" \
    --argjson line "$line" \
    --arg title "$title" \
    --arg body "$body" \
    --arg dedupe_key "$dedupe_key" \
    '{
      dimension: $dimension,
      severity: $severity,
      file: $file,
      line: $line,
      title: $title,
      body: $body,
      dedupe_key: $dedupe_key
    }' > "$tmp_gap"
  jq -s '.[0] + [.[1]]' "$GAPS_TMP" "$tmp_gap" > "$GAPS_TMP.next"
  mv "$GAPS_TMP.next" "$GAPS_TMP"
  rm -f "$tmp_gap"
}

REPO_ROOT="$PWD"
OUT=""
SINCE=""

while [ $# -gt 0 ]; do
  case "$1" in
    --repo-root)
      REPO_ROOT="${2:-}"
      shift 2
      ;;
    --emit-gaps)
      OUT="${2:-}"
      shift 2
      ;;
    --since)
      SINCE="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      shift
      ;;
  esac
done

[ -n "$OUT" ] || fail "--emit-gaps is required"
[ -d "$REPO_ROOT" ] || fail "repo root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd)"
CONFIG="$REPO_ROOT/.autospec/autospec.yml"
[ -f "$CONFIG" ] || fail "missing .autospec/autospec.yml"

command -v jq >/dev/null 2>&1 || fail "jq not found on PATH"
command -v yq >/dev/null 2>&1 || fail "yq not found on PATH"

mkdir -p "$(dirname "$OUT")"
GAPS_TMP="$(mktemp -t autospec-sweep-review.XXXXXX)"
trap 'rm -f "$GAPS_TMP"' EXIT
printf '[]\n' > "$GAPS_TMP"

test_cmd="$(yq -r '.project.findings.commands.test // ""' "$CONFIG")"
e2e_cmd="$(yq -r '.project.findings.commands.e2e // ""' "$CONFIG")"
deploy_cmd="$(yq -r '.project.findings.commands.deploy // ""' "$CONFIG")"
spec_sync_enabled="$(yq -r '.sweep.spec_sync.enabled // true' "$CONFIG")"
docs_enabled="$(yq -r '.continuous_improvement.docs.enabled // true' "$CONFIG")"
tests_enabled="$(yq -r '.continuous_improvement.tests.enabled // true' "$CONFIG")"
code_enabled="$(yq -r '.continuous_improvement.code.enabled // true' "$CONFIG")"
deploy_if_tests_require="$(yq -r '.execution.deployment.deploy_if_tests_require // true' "$CONFIG")"

if [ "$tests_enabled" = "true" ]; then
  case "$test_cmd" in
    ""|TODO:*)
      add_gap \
        "tests" \
        "high" \
        ".autospec/autospec.yml" \
        1 \
        "Configure autospec inner-loop test command" \
        "Set project.findings.commands.test to the fast command autospec should run before filing or merging fixes." \
        "autospec-config-test-command"
      ;;
  esac

  case "$e2e_cmd" in
    ""|TODO:*)
      add_gap \
        "tests" \
        "medium" \
        ".autospec/autospec.yml" \
        1 \
        "Configure autospec E2E command" \
        "Set project.findings.commands.e2e or disable the E2E sweep for projects without browser or workflow coverage." \
        "autospec-config-e2e-command"
      ;;
  esac

  case "$e2e_cmd" in
    ""|TODO:*) ;;
    *)
      if [ "$deploy_if_tests_require" = "true" ]; then
        case "$deploy_cmd" in
          ""|TODO:*)
            add_gap \
              "tests" \
              "high" \
              ".autospec/autospec.yml" \
              1 \
              "Configure deploy command before E2E tests" \
              "Set project.findings.commands.deploy so autospec can deploy or start the software before E2E/integration tests run." \
              "autospec-config-deploy-command"
            ;;
        esac
      fi
      ;;
  esac
fi

if [ "$spec_sync_enabled" = "true" ] && [ ! -d "$REPO_ROOT/docs/specs" ]; then
  add_gap \
    "spec_sync" \
    "medium" \
    "docs/specs" \
    0 \
    "Create tracked specs directory" \
    "Add docs/specs/ with at least one current design spec so sweep results can reconcile implementation reality against source artifacts." \
    "autospec-specs-directory"
fi

if [ "$docs_enabled" = "true" ] && [ ! -f "$REPO_ROOT/README.md" ]; then
  add_gap \
    "docs" \
    "medium" \
    "README.md" \
    0 \
    "Add project README" \
    "Create README.md so autospec-sweep has a user-facing documentation surface to keep synchronized with implemented behavior." \
    "autospec-readme-missing"
fi

if [ "$code_enabled" = "true" ] && command -v rg >/dev/null 2>&1; then
  todo_count="$(rg -n '\b(TODO|FIXME|XXX)\b' "$REPO_ROOT" -g '!artifacts/**' -g '!node_modules/**' -g '!.git/**' 2>/dev/null | wc -l | tr -d ' ')"
  if [ "${todo_count:-0}" -gt 0 ]; then
    add_gap \
      "code" \
      "low" \
      "." \
      0 \
      "Resolve lingering TODO markers" \
      "Found ${todo_count} TODO/FIXME/XXX markers; convert each real deferred behavior into tracked specs or issues and remove stale markers." \
      "autospec-code-todo-markers"
  fi
fi

jq -c '[to_entries[] | .value + {gap_id: ("G" + ((.key + 1) | tostring))}]' "$GAPS_TMP" > "$OUT"
printf 'autospec-sweep review: wrote %s\n' "$OUT" >&2
