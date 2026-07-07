#!/usr/bin/env bats
# tests/ux-ui-workstream.bats — contract tests for issue #1538 autonomous UX/UI optimization tier.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/ux-ui-workstream.sh"
    WORK="$(mktemp -d -t ux-ui-workstream.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

@test "gate: enforces CWV, Lighthouse, token lint, visual regression, interaction health, and both themes" {
    LEDGER="$WORK/ux.jsonl"

    run bash "$SCRIPT" record-snapshot --ledger "$LEDGER" --commit base123 --theme light --lcp-ms 2200 --inp-ms 150 --cls 0.05 --lighthouse-performance 94 --token-violations 0 --visual-diff-pct 0.04 --console-errors 0 --failed-requests 0 --tap-target-violations 0 --horizontal-overflow 0 --timestamp 2026-07-07T00:00:00Z
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" record-snapshot --ledger "$LEDGER" --commit base123 --theme dark --lcp-ms 2300 --inp-ms 170 --cls 0.07 --lighthouse-performance 93 --token-violations 0 --visual-diff-pct 0.05 --console-errors 0 --failed-requests 0 --tap-target-violations 0 --horizontal-overflow 0 --timestamp 2026-07-07T00:01:00Z
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" gate --ledger "$LEDGER" --commit base123
    [ "$status" -eq 0 ]
    [[ "$output" == *"ux-ui gate passed"* ]]

    run bash "$SCRIPT" record-snapshot --ledger "$LEDGER" --commit cand456 --theme light --lcp-ms 2700 --inp-ms 230 --cls 0.12 --lighthouse-performance 84 --token-violations 2 --visual-diff-pct 0.35 --console-errors 1 --failed-requests 1 --tap-target-violations 3 --horizontal-overflow 1 --timestamp 2026-07-07T01:00:00Z
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" gate --ledger "$LEDGER" --commit cand456 --regressions-out "$WORK/regressions.jsonl"
    [ "$status" -eq 1 ]
    [[ "$output" == *"LCP_BUDGET_BREACH:light:2700ms>2500ms"* ]]
    [[ "$output" == *"INP_BUDGET_BREACH:light:230ms>200ms"* ]]
    [[ "$output" == *"CLS_BUDGET_BREACH:light:0.120>0.100"* ]]
    [[ "$output" == *"LIGHTHOUSE_PERFORMANCE_BELOW_FLOOR:light:84<90"* ]]
    [[ "$output" == *"TOKEN_LINT_VIOLATIONS:light:2"* ]]
    [[ "$output" == *"VISUAL_DIFF_ABOVE_BUDGET:light:0.350%>0.100%"* ]]
    [[ "$output" == *"CONSOLE_ERRORS:light:1"* ]]
    [[ "$output" == *"FAILED_REQUESTS:light:1"* ]]
    [[ "$output" == *"TAP_TARGET_VIOLATIONS:light:3"* ]]
    [[ "$output" == *"HORIZONTAL_OVERFLOW:light:1"* ]]
    [[ "$output" == *"THEME_MISSING:dark"* ]]
    [ -s "$WORK/regressions.jsonl" ]
}

@test "regression: proposes a prioritized auto-implement issue from CWV or Lighthouse failures" {
    cat > "$WORK/regressions.jsonl" <<'JSONL'
{"tag":"LCP_BUDGET_BREACH","theme":"light","metric":"lcp_ms","value":2700,"budget":2500,"commit":"cand456","test_cmd":"bash scripts/ux-ui-workstream.sh gate --ledger .autospec/ux-ui/snapshots.jsonl --commit cand456"}
{"tag":"LIGHTHOUSE_PERFORMANCE_BELOW_FLOOR","theme":"dark","metric":"lighthouse_performance","value":84,"budget":90,"commit":"cand456","test_cmd":"bash scripts/ux-ui-workstream.sh gate --ledger .autospec/ux-ui/snapshots.jsonl --commit cand456"}
JSONL

    run bash "$SCRIPT" propose-regression-issue --regressions "$WORK/regressions.jsonl" --out "$WORK/issues"
    [ "$status" -eq 0 ]
    [ -f "$WORK/issues/cwv-lighthouse-regression-cand456.md" ]
    body="$(cat "$WORK/issues/cwv-lighthouse-regression-cand456.md")"
    [[ "$body" == *"auto-implement"* ]]
    [[ "$body" == *"priority:high"* ]]
    [[ "$body" == *"LCP_BUDGET_BREACH"* ]]
    [[ "$body" == *"LIGHTHOUSE_PERFORMANCE_BELOW_FLOOR"* ]]
    [[ "$body" == *"bash scripts/ux-ui-workstream.sh gate"* ]]
    run bash "$REPO_ROOT/scripts/lint-issue.sh" "$WORK/issues/cwv-lighthouse-regression-cand456.md"
    [ "$status" -eq 0 ]
}

@test "optimization: before/after report requires a measured UX/UI improvement and rejects collateral regressions" {
    cat > "$WORK/before.jsonl" <<'JSONL'
{"commit":"before","theme":"light","lcp_ms":2500,"inp_ms":190,"cls":0.08,"lighthouse_performance":91,"token_violations":0,"visual_diff_pct":0.05,"console_errors":0,"failed_requests":0,"tap_target_violations":0,"horizontal_overflow":0}
{"commit":"before","theme":"dark","lcp_ms":2400,"inp_ms":180,"cls":0.07,"lighthouse_performance":92,"token_violations":0,"visual_diff_pct":0.05,"console_errors":0,"failed_requests":0,"tap_target_violations":0,"horizontal_overflow":0}
JSONL
    cat > "$WORK/after-bad.jsonl" <<'JSONL'
{"commit":"after","theme":"light","lcp_ms":2300,"inp_ms":170,"cls":0.06,"lighthouse_performance":94,"token_violations":0,"visual_diff_pct":0.04,"console_errors":0,"failed_requests":0,"tap_target_violations":0,"horizontal_overflow":0}
{"commit":"after","theme":"dark","lcp_ms":2800,"inp_ms":180,"cls":0.07,"lighthouse_performance":92,"token_violations":0,"visual_diff_pct":0.05,"console_errors":0,"failed_requests":0,"tap_target_violations":0,"horizontal_overflow":0}
JSONL

    run bash "$SCRIPT" improvement-report --before "$WORK/before.jsonl" --after "$WORK/after-bad.jsonl" --out "$WORK/report.md"
    [ "$status" -eq 1 ]
    [[ "$output" == *"COLLATERAL_REGRESSION:dark:lcp_ms:2400->2800"* ]]
    [ ! -f "$WORK/report.md" ]

    cat > "$WORK/after-good.jsonl" <<'JSONL'
{"commit":"after","theme":"light","lcp_ms":2300,"inp_ms":170,"cls":0.06,"lighthouse_performance":94,"token_violations":0,"visual_diff_pct":0.04,"console_errors":0,"failed_requests":0,"tap_target_violations":0,"horizontal_overflow":0}
{"commit":"after","theme":"dark","lcp_ms":2350,"inp_ms":170,"cls":0.06,"lighthouse_performance":93,"token_violations":0,"visual_diff_pct":0.04,"console_errors":0,"failed_requests":0,"tap_target_violations":0,"horizontal_overflow":0}
JSONL
    run bash "$SCRIPT" improvement-report --before "$WORK/before.jsonl" --after "$WORK/after-good.jsonl" --out "$WORK/report.md"
    [ "$status" -eq 0 ]
    grep -q 'Measured UX/UI before/after' "$WORK/report.md"
    grep -q 'light lcp_ms: 2500 -> 2300' "$WORK/report.md"
    grep -q 'dark lighthouse_performance: 92 -> 93' "$WORK/report.md"
    grep -q 'No collateral UX/UI regressions' "$WORK/report.md"
}

@test "design doc: cites source canon and validates both light and dark themes" {
    run bash "$SCRIPT" validate-design-doc --doc "$REPO_ROOT/docs/runbooks/ux-ui-workstream.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"design doc validated"* ]]
}

@test "validate.sh wires the UX/UI workstream helper, runbook, CI gate, and bats suite" {
    grep -q '^check_ux_ui_workstream_contract()' "$REPO_ROOT/scripts/validate.sh"
    grep -q 'ux-ui-workstream\.sh' "$REPO_ROOT/scripts/validate.sh"
    grep -q 'tests/autonomous/test_ux_ui_workstream\.bats' "$REPO_ROOT/scripts/validate.sh"
    grep -q 'ux-ui-workstream\.yml' "$REPO_ROOT/scripts/validate.sh"
    [ -f "$REPO_ROOT/docs/runbooks/ux-ui-workstream.md" ]
}
