#!/usr/bin/env bats
# Deterministic self-improvement candidate discovery for dry autonomous queues.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autonomous-self-improvement.sh"
    TMP="$(mktemp -d -t self-improve.XXXXXX)"
    mkdir -p "$TMP/repo/crates/autospec-cli/src/commands" "$TMP/repo/docs/reports" "$TMP/bin"
    cat > "$TMP/bin/autospec" <<'SH'
#!/usr/bin/env bash
exit 0
SH
    chmod +x "$TMP/bin/autospec"
    export PATH="$TMP/bin:$PATH"
}

teardown() {
    rm -rf "$TMP"
}

seed_repo_gaps() {
    cat > "$TMP/repo/crates/autospec-cli/src/commands/run.rs" <<'RS'
pub fn run(_args: &[String]) -> Result<(), String> {
    super::not_implemented("run")
}
RS
    cat > "$TMP/repo/docs/reports/spec-state-reconciliation-2026-07-08.md" <<'MD'
# Spec State Reconciliation Report

## Remaining Risks

- Issue backlog hygiene is now the main spec-to-execution bottleneck.
- RAG has solid local gates, but production-scale corpus and citation evidence still need ongoing proof.
MD
}

@test "candidates emits value-scored repo improvement work from local signals" {
    seed_repo_gaps

    run bash "$SCRIPT" candidates --repo-root "$TMP/repo"

    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | jq -s 'length')" -ge 2 ]
    printf '%s\n' "$output" | jq -e 'select(.id=="cli-stub-run")' >/dev/null
    printf '%s\n' "$output" | jq -e 'select(.workstream=="report-risk")' >/dev/null
}

@test "every emitted candidate carries the bounded falsifiable hypothesis contract" {
    seed_repo_gaps
    run bash "$SCRIPT" candidates --repo-root "$TMP/repo"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | jq -s -e 'all(.[ ];
      (.evidence | length > 0) and (.failed_invariant | length > 0) and
      (.named_consumer | length > 0) and (.falsifier.command | length > 0) and
      (.files | length > 0 and length <= 3) and (.dedupe_key | length > 0) and
      (.before_after | type == "object") and (.sample_floor >= 1) and
      (.max_cost_regression >= 0) and (.rollback | type == "object") and
      (.change_class == "strengthening" or .change_class == "neutral" or .change_class == "weakening") and
      (.questions | length == 8))' >/dev/null
}

@test "self-improvement evaluator is available to advance the lifecycle" {
    run python3 "$REPO_ROOT/scripts/autonomous-self-improvement-evaluate" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"evaluate"* ]]
    [[ "$output" == *"advance"* ]]
}

@test "apply is report-only unless both --apply and env opt-in are present" {
    seed_repo_gaps
    cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"repo view"*) printf 'berlinguyinca/autospec\n' ;;
esac
SH
    chmod +x "$TMP/bin/gh"
    export PATH="$TMP/bin:$PATH"
    export GH_LOG="$TMP/gh.log"

    run bash "$SCRIPT" apply --repo-root "$TMP/repo" --repo berlinguyinca/autospec --apply

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.dry')" = "true" ]
    [ ! -f "$GH_LOG" ] || ! grep -q 'issue create' "$GH_LOG"
}

@test "apply files needs-classify issues when explicitly enabled" {
    seed_repo_gaps
    cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"label create"*) exit 0 ;;
  *"issue create"*)
    while [ "$#" -gt 0 ]; do
      if [ "$1" = "--body-file" ]; then
        cp "$2" "$GH_BODY"
      fi
      shift
    done
    printf 'https://github.com/berlinguyinca/autospec/issues/999\n'
    exit 0
    ;;
  *"repo view"*) printf 'berlinguyinca/autospec\n'; exit 0 ;;
esac
exit 0
SH
    chmod +x "$TMP/bin/gh"
    export PATH="$TMP/bin:$PATH"
    export GH_LOG="$TMP/gh.log"
    export GH_BODY="$TMP/body.md"
    export AUTOSPEC_SELF_IMPROVEMENT_APPLY=1

    run bash "$SCRIPT" apply --repo-root "$TMP/repo" --repo berlinguyinca/autospec --apply --limit 1

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.dry')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.filed')" = "1" ]
    grep -q 'issue create' "$GH_LOG"
    grep -q 'needs-classify' "$GH_LOG"
    grep -q '^## Acceptance criteria$' "$GH_BODY"
    bash "$REPO_ROOT/scripts/lint-issue.sh" "$GH_BODY"
    [ "$(awk '
        /### Primary smoke test \(inner loop\)/ { in_smoke=1; next }
        in_smoke && /^```/ { fence++; next }
        in_smoke && fence == 1 && NF && $0 !~ /^[[:space:]]*#/ { count++ }
        in_smoke && fence == 2 { print count + 0; exit }
    ' "$GH_BODY")" = "1" ]
}

@test "apply emits issue bodies that pass issue-quality lint" {
    seed_repo_gaps
    cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"label create"*) exit 0 ;;
  *"issue create"*)
    while [ "$#" -gt 0 ]; do
      if [ "$1" = "--body-file" ]; then
        cp "$2" "$CAPTURED_BODY"
      fi
      shift
    done
    printf 'https://github.com/berlinguyinca/autospec/issues/999\n'
    exit 0
    ;;
  *"repo view"*) printf 'berlinguyinca/autospec\n'; exit 0 ;;
esac
exit 0
SH
    chmod +x "$TMP/bin/gh"
    export PATH="$TMP/bin:$PATH"
    export GH_LOG="$TMP/gh.log"
    export CAPTURED_BODY="$TMP/issue-body.md"
    export AUTOSPEC_SELF_IMPROVEMENT_APPLY=1

    run bash "$SCRIPT" apply --repo-root "$TMP/repo" --repo berlinguyinca/autospec --apply --limit 1

    [ "$status" -eq 0 ]
    [ -s "$CAPTURED_BODY" ]
    run bash "$REPO_ROOT/scripts/lint-issue.sh" "$CAPTURED_BODY"
    [ "$status" -eq 0 ]
}
