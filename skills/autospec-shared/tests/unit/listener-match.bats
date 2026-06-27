#!/usr/bin/env bats
# listener-match.bats — Bats tests for scripts/listener-match.sh in its
# verb→skill classifier (`--classify`) mode.
#
# Run: bats skills/autospec-shared/tests/unit/listener-match.bats
#
# Covers the keyword-routing classifier (issue #537):
#   * positive routes for each verb in the D3 map
#   * the D4 intent gate suppressing incidental/past/negated/question uses
#   * JSON schema {match,skill,trigger,intent,confidence}
#   * back-compat: default (no-flag) word mode + file-an-issue/write-a-spec

setup() {
    # The shared tests live under skills/autospec-shared/tests/unit; the
    # script lives at the repo root scripts/. Walk up to the repo root.
    REPO_ROOT="$(cd "$(dirname "${BATS_TEST_FILENAME}")/../../../.." && pwd)"
    MATCH="${REPO_ROOT}/scripts/listener-match.sh"
}

# Parse a JSON field from the output via node (available in this repo).
json_field() {
    local json="$1"
    local expr="$2"
    printf '%s' "$json" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
process.stdout.write(String($expr));
" 2>/dev/null || echo "PARSE_ERROR"
}

# ── exit code / schema ────────────────────────────────────────────────────────

@test "classify mode always exits 0" {
    run "$MATCH" --classify "implement the cache layer"
    [ "$status" -eq 0 ]
}

@test "classify mode emits valid JSON with required keys" {
    run "$MATCH" --classify "implement the cache layer"
    [ "$status" -eq 0 ]
    result="$(json_field "$output" "
typeof d.match==='boolean' &&
('skill' in d) && ('trigger' in d) &&
('intent' in d) && ('confidence' in d) ? 'ok' : 'fail'")"
    [ "$result" = "ok" ]
}

@test "non-match emits {match:false} (valid JSON, exit 0)" {
    run "$MATCH" --classify "what a lovely afternoon"
    [ "$status" -eq 0 ]
    m="$(json_field "$output" "d.match")"
    [ "$m" = "false" ]
}

# ── positive routes (one per verb) ─────────────────────────────────────────────

@test "positive: 'implement the cache layer' → autospec-run / imperative" {
    run "$MATCH" --classify "implement the cache layer"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-run" ]
    [ "$(json_field "$output" "d.intent")" = "imperative" ]
    [ "$(json_field "$output" "d.trigger")" = "implement" ]
}

@test "positive: 'build the auth service' → autospec-run" {
    run "$MATCH" --classify "build the auth service"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-run" ]
}

@test "positive: 'ship the new billing flow' → autospec-run" {
    run "$MATCH" --classify "ship the new billing flow"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-run" ]
}

@test "positive: 'design a webhook system' → autospec-define" {
    run "$MATCH" --classify "design a webhook system"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-define" ]
    [ "$(json_field "$output" "d.intent")" = "imperative" ]
}

@test "positive: 'spec out the import pipeline' → autospec-define" {
    run "$MATCH" --classify "spec out the import pipeline"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-define" ]
}

@test "positive: 'add a new feature for exports' → autospec-define" {
    run "$MATCH" --classify "add a new feature for exports"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-define" ]
}

@test "positive: 'review the PR' → autospec-review" {
    run "$MATCH" --classify "review the PR"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-review" ]
    [ "$(json_field "$output" "d.intent")" = "imperative" ]
}

@test "positive: 'autospec the billing module' → autospec" {
    run "$MATCH" --classify "autospec the billing module"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec" ]
}

@test "post-approval: approved autospec spec routes to autospec autonomous" {
    AUTOSPEC_LISTENER_APPROVED_SPEC_PATH="docs/specs/billing.md" \
        run "$MATCH" --classify "looks good"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec" ]
    [ "$(json_field "$output" "d.trigger")" = "post_approval_execution_ready" ]
    [ "$(json_field "$output" "d.autonomous")" = "true" ]
}

@test "post-approval: approved plan routes to autospec autonomous" {
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
        run "$MATCH" --classify "approved"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec" ]
    [ "$(json_field "$output" "d.autonomous")" = "true" ]
}

@test "post-approval: open auto-implement issues route to autospec-run" {
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
    AUTOSPEC_LISTENER_AUTO_IMPLEMENT_OPEN=2 \
        run "$MATCH" --classify "looks good"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-run" ]
    [ "$(json_field "$output" "d.trigger")" = "post_approval_execution_ready" ]
}

@test "plan-exit: completed saved implementation plan routes to autospec autonomous with plan metadata" {
    AUTOSPEC_LISTENER_PLAN_EXIT_READY=1 \
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
    AUTOSPEC_LISTENER_SOURCE_SPEC_PATH="docs/specs/billing.md" \
    AUTOSPEC_LISTENER_REPO="berlinguyinca/autospec" \
        run "$MATCH" --classify "Implementation plan saved. Subagent-Driven or Inline Execution?"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec" ]
    [ "$(json_field "$output" "d.trigger")" = "plan_exit_ready" ]
    [ "$(json_field "$output" "d.autonomous")" = "true" ]
    [ "$(json_field "$output" "d.plan_path")" = "docs/plans/billing.md" ]
    [ "$(json_field "$output" "d.source_spec_path")" = "docs/specs/billing.md" ]
    [ "$(json_field "$output" "d.repo")" = "berlinguyinca/autospec" ]
}

@test "plan-exit: open matching auto-implement issues route to autospec-run" {
    AUTOSPEC_LISTENER_PLAN_EXIT_READY=1 \
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
    AUTOSPEC_LISTENER_AUTO_IMPLEMENT_OPEN=3 \
        run "$MATCH" --classify "Implementation plan saved. Choose how to execute it."
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-run" ]
    [ "$(json_field "$output" "d.trigger")" = "plan_exit_ready" ]
}

@test "plan-exit: approval after completed plan routes with plan-approved trigger" {
    AUTOSPEC_LISTENER_PLAN_EXIT_READY=1 \
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
        run "$MATCH" --classify "continue"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec" ]
    [ "$(json_field "$output" "d.trigger")" = "plan_approved_ready" ]
}

@test "plan-exit: stop and cancel opt-outs prevent routing" {
    AUTOSPEC_LISTENER_PLAN_EXIT_READY=1 \
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
        run "$MATCH" --classify "stop"
    [ "$(json_field "$output" "d.match")" = "false" ]

    AUTOSPEC_LISTENER_PLAN_EXIT_READY=1 \
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
        run "$MATCH" --classify "cancel"
    [ "$(json_field "$output" "d.match")" = "false" ]
}

@test "plan-exit: blocked plan exit does not route" {
    AUTOSPEC_LISTENER_PLAN_EXIT_READY=1 \
    AUTOSPEC_LISTENER_PLAN_EXIT_BLOCKED=1 \
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
        run "$MATCH" --classify "Plan mode ended blocked on missing requirements."
    [ "$(json_field "$output" "d.match")" = "false" ]
}

@test "plan-exit: destructive action gate does not route" {
    AUTOSPEC_LISTENER_PLAN_EXIT_READY=1 \
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
    AUTOSPEC_LISTENER_PLAN_DESTRUCTIVE=1 \
        run "$MATCH" --classify "Plan complete but it requires production credentials."
    [ "$(json_field "$output" "d.match")" = "false" ]
}

# ── negatives (must NOT route) ──────────────────────────────────────────────────

@test "negative: 'I already reviewed it' → no route" {
    run "$MATCH" --classify "I already reviewed it"
    m="$(json_field "$output" "d.match")"
    intent="$(json_field "$output" "d.intent")"
    [ "$m" = "false" ] || [ "$intent" = "incidental" ]
}

@test "negative: 'the design looks nice' → no route" {
    run "$MATCH" --classify "the design looks nice"
    m="$(json_field "$output" "d.match")"
    intent="$(json_field "$output" "d.intent")"
    [ "$m" = "false" ] || [ "$intent" = "incidental" ]
}

@test "negative: \"don't implement that yet\" → no route" {
    run "$MATCH" --classify "don't implement that yet"
    m="$(json_field "$output" "d.match")"
    intent="$(json_field "$output" "d.intent")"
    [ "$m" = "false" ] || [ "$intent" = "incidental" ]
}

@test "negative: 'should we redesign this?' → no route" {
    run "$MATCH" --classify "should we redesign this?"
    m="$(json_field "$output" "d.match")"
    intent="$(json_field "$output" "d.intent")"
    [ "$m" = "false" ] || [ "$intent" = "incidental" ]
}

@test "negative: 'the review went well' → no route" {
    run "$MATCH" --classify "the review went well"
    m="$(json_field "$output" "d.match")"
    intent="$(json_field "$output" "d.intent")"
    [ "$m" = "false" ] || [ "$intent" = "incidental" ]
}

@test "negative: 'we already shipped that' → no route" {
    run "$MATCH" --classify "we already shipped that"
    m="$(json_field "$output" "d.match")"
    intent="$(json_field "$output" "d.intent")"
    [ "$m" = "false" ] || [ "$intent" = "incidental" ]
}

@test "negative: 'do not build it' → no route" {
    run "$MATCH" --classify "do not build it"
    m="$(json_field "$output" "d.match")"
    intent="$(json_field "$output" "d.intent")"
    [ "$m" = "false" ] || [ "$intent" = "incidental" ]
}

@test "negative: 'how should I implement this?' → no route" {
    run "$MATCH" --classify "how should I implement this?"
    m="$(json_field "$output" "d.match")"
    intent="$(json_field "$output" "d.intent")"
    [ "$m" = "false" ] || [ "$intent" = "incidental" ]
}

@test "negative: post-approval plain opt-out → no route" {
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
        run "$MATCH" --classify "plain"
    [ "$(json_field "$output" "d.match")" = "false" ]
}

@test "negative: descriptive autospec mention does not post-approval route" {
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
        run "$MATCH" --classify "the autospec plan looks detailed"
    [ "$(json_field "$output" "d.match")" = "false" ]
}

@test "negative: do not implement suppresses post-approval route" {
    AUTOSPEC_LISTENER_PLAN_PATH="docs/plans/billing.md" \
        run "$MATCH" --classify "do not implement"
    [ "$(json_field "$output" "d.match")" = "false" ]
}

# ── back-compat: classify mode still surfaces file-an-issue / write-a-spec ──────

@test "back-compat classify: 'file an issue' → autospec-define (trigger=issue)" {
    run "$MATCH" --classify "file an issue for that"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-define" ]
}

@test "back-compat classify: 'write a spec' → autospec-define" {
    run "$MATCH" --classify "could you write a spec for this"
    [ "$(json_field "$output" "d.match")" = "true" ]
    [ "$(json_field "$output" "d.skill")" = "autospec-define" ]
}

# ── back-compat: default word mode unchanged ────────────────────────────────────

@test "back-compat word mode: 'file an issue' → issue" {
    run "$MATCH" "file an issue"
    [ "$status" -eq 0 ]
    [ "$output" = "issue" ]
}

@test "back-compat word mode: 'write a spec' → spec" {
    run "$MATCH" "write a spec"
    [ "$output" = "spec" ]
}

@test "back-compat word mode: bare 'issue' → none" {
    run "$MATCH" "issue"
    [ "$output" = "none" ]
}

# ── syntax ──────────────────────────────────────────────────────────────────────

@test "bash -n passes on the script" {
    run bash -n "$MATCH"
    [ "$status" -eq 0 ]
}
