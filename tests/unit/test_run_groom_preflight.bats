#!/usr/bin/env bats
# tests/unit/test_run_groom_preflight.bats — coverage for the autospec-run
# Phase 4 backlog-grooming preflight helper. The preflight is intentionally
# small: policy off skips the orchestrator; policy auto/on runs exactly one
# double-gated --apply cycle; failures warn and return success so the drain
# continues.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/skills/autospec-run/scripts/run-groom-preflight.sh"
    TMP="$(mktemp -d -t run-groom-preflight.XXXXXX)"
    export AUTOSPEC_SCRIPTS_DIR="$TMP/scripts"
    mkdir -p "$AUTOSPEC_SCRIPTS_DIR"
    REPORT="$TMP/run-report.md"
    CALLS="$TMP/promote.calls"
    export CALLS

    cat > "$AUTOSPEC_SCRIPTS_DIR/grooming-config.sh" <<'SH'
#!/usr/bin/env bash
case "$*" in
  *policy*) printf '%s\n' "${TEST_GROOM_POLICY:-auto}" ;;
  *budget.max_issues_per_cycle*) printf '3\n' ;;
  *) printf '\n' ;;
esac
SH
    chmod +x "$AUTOSPEC_SCRIPTS_DIR/grooming-config.sh"

    cat > "$AUTOSPEC_SCRIPTS_DIR/autonomous-promote-open-issues.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$CALLS"
if [ "${TEST_PROMOTE_FAIL:-0}" = "1" ]; then
  printf 'boom from groomer\n' >&2
  exit 42
fi
cat <<'JSON'
{"dry":false,"filed":1,"promoted":[101],"routed":[{"issue":202,"action":"groom-canary","reason":"needs-template"}],"held":[{"issue":303,"reason":"hold:needs-human"}],"skipped":[],"reason":"test-cycle"}
JSON
SH
    chmod +x "$AUTOSPEC_SCRIPTS_DIR/autonomous-promote-open-issues.sh"
}

teardown() {
    rm -rf "$TMP"
}

@test "policy off skips the grooming orchestrator" {
    TEST_GROOM_POLICY=off run bash "$SCRIPT" --repo owner/repo --report "$REPORT"
    [ "$status" -eq 0 ]
    [ ! -f "$CALLS" ]
    [ ! -s "$REPORT" ]
    echo "$output" | jq -e '.status == "skipped" and .policy == "off"' >/dev/null
}

@test "policy auto runs one double-gated apply cycle and appends run-report summary" {
    TEST_GROOM_POLICY=auto run bash "$SCRIPT" --repo owner/repo --report "$REPORT"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$CALLS" | tr -d ' ')" = "1" ]
    grep -q -- '--repo owner/repo --apply' "$CALLS"
    echo "$output" | jq -e '.status == "ok" and .promoted == [101] and .["groom:proposed"] == [202] and .held[0].issue == 303' >/dev/null
    grep -q 'Backlog grooming preflight' "$REPORT"
    grep -q '"promoted":\[101\]' "$REPORT"
    grep -q '"groom:proposed":\[202\]' "$REPORT"
    grep -q '"held":\[{"issue":303,"reason":"hold:needs-human"}\]' "$REPORT"
}

@test "grooming failure warns once and exits zero so the drain proceeds" {
    TEST_GROOM_POLICY=auto TEST_PROMOTE_FAIL=1 run bash "$SCRIPT" --repo owner/repo --report "$REPORT"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$CALLS" | tr -d ' ')" = "1" ]
    [ "$(printf '%s\n' "$output" | grep -c '^WARN: backlog grooming preflight failed')" = "1" ]
    echo "$output" | tail -n 1 | jq -e '.status == "warn" and .promoted == [] and .["groom:proposed"] == [] and .held == []' >/dev/null
    grep -q '"status":"warn"' "$REPORT"
}

@test "autospec-run prompt wires preflight before queue scan with double gate and no discovery" {
    skill="$REPO_ROOT/skills/autospec-run/SKILL.md"
    grep -q 'Backlog grooming preflight' "$skill"
    grep -q 'double gate' "$skill"
    grep -q 'no discovery' "$skill"
    python3 - "$skill" <<'PY'
import sys
text = open(sys.argv[1], encoding='utf-8').read()
preflight = text.index('Backlog grooming preflight')
queue = text.index('monitor-outer-loop.sh')
if preflight > queue:
    raise SystemExit('preflight must appear before queue scan')
PY
}
