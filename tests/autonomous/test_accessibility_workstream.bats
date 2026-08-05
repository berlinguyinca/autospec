#!/usr/bin/env bats
# Contract tests for issue #1539 accessibility and web-standards compliance tier.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/accessibility-workstream.sh"
    WORK="$(mktemp -d -t accessibility-workstream.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

@test "gate: axe, pa11y, Lighthouse a11y, IBM Equal Access, and both themes block PRs" {
    LEDGER="$WORK/a11y.jsonl"

    run bash "$SCRIPT" record-scan --ledger "$LEDGER" --commit ok123 --theme light --axe-violations 0 --pa11y-errors 0 --lighthouse-a11y 100 --ibm-violations 0 --auto-fixable 0 --judgment-findings 0 --timestamp 2026-07-07T00:00:00Z
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" record-scan --ledger "$LEDGER" --commit ok123 --theme dark --axe-violations 0 --pa11y-errors 0 --lighthouse-a11y 99 --ibm-violations 0 --auto-fixable 0 --judgment-findings 0 --timestamp 2026-07-07T00:01:00Z
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" gate --ledger "$LEDGER" --commit ok123
    [ "$status" -eq 0 ]
    [[ "$output" == *"accessibility gate passed"* ]]

    run bash "$SCRIPT" record-scan --ledger "$LEDGER" --commit bad456 --theme light --axe-violations 2 --pa11y-errors 1 --lighthouse-a11y 89 --ibm-violations 3 --auto-fixable 6 --judgment-findings 0 --timestamp 2026-07-07T01:00:00Z
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" gate --ledger "$LEDGER" --commit bad456 --findings-out "$WORK/findings.jsonl"
    [ "$status" -eq 1 ]
    [[ "$output" == *"AXE_VIOLATIONS:light:2"* ]]
    [[ "$output" == *"PA11Y_ERRORS:light:1"* ]]
    [[ "$output" == *"LIGHTHOUSE_A11Y_BELOW_AA_FLOOR:light:89<90"* ]]
    [[ "$output" == *"IBM_EQUAL_ACCESS_VIOLATIONS:light:3"* ]]
    [[ "$output" == *"THEME_MISSING:dark"* ]]
    [ -s "$WORK/findings.jsonl" ]
}

@test "classification: machine-verifiable fixes are auto-fixable while judgment findings require human review" {
    run bash "$SCRIPT" classify-finding --rule missing-form-label --legal-exposure 5 --user-blocking 5 --traffic 4 --occurrences 3
    [ "$status" -eq 0 ]
    [[ "$output" == *'"remediation_class":"auto_fixable"'* ]]
    [[ "$output" == *'"priority":"P0"'* ]]
    [[ "$output" == *'"score":300'* ]]

    run bash "$SCRIPT" classify-finding --rule focus-order --legal-exposure 4 --user-blocking 5 --traffic 3 --occurrences 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"remediation_class":"judgment"'* ]]
    [[ "$output" == *'"human_review_required":true'* ]]

    cat > "$WORK/findings.jsonl" <<'JSONL'
{"rule":"duplicate-id","legal_exposure":3,"user_blocking":2,"traffic":4,"occurrences":5}
{"rule":"keyboard-trap","legal_exposure":5,"user_blocking":5,"traffic":5,"occurrences":2}
JSONL
    run bash "$SCRIPT" rank --findings "$WORK/findings.jsonl" --out "$WORK/ranked.jsonl"
    [ "$status" -eq 0 ]
    first_rule="$(head -n1 "$WORK/ranked.jsonl" | jq -r '.rule')"
    first_priority="$(head -n1 "$WORK/ranked.jsonl" | jq -r '.priority')"
    [ "$first_rule" = "keyboard-trap" ]
    [ "$first_priority" = "P0" ]
}

@test "remediation report: requires re-scan improvement and blocks auto-merged judgment findings" {
    cat > "$WORK/before.jsonl" <<'JSONL'
{"commit":"before","theme":"light","axe_violations":2,"pa11y_errors":1,"lighthouse_a11y":89,"ibm_violations":1,"auto_fixable":4,"judgment_findings":0}
{"commit":"before","theme":"dark","axe_violations":1,"pa11y_errors":0,"lighthouse_a11y":95,"ibm_violations":0,"auto_fixable":1,"judgment_findings":1,"judgment_status":"human_review"}
JSONL
    cat > "$WORK/after-bad.jsonl" <<'JSONL'
{"commit":"after","theme":"light","axe_violations":0,"pa11y_errors":0,"lighthouse_a11y":100,"ibm_violations":0,"auto_fixable":0,"judgment_findings":0}
{"commit":"after","theme":"dark","axe_violations":0,"pa11y_errors":0,"lighthouse_a11y":100,"ibm_violations":0,"auto_fixable":0,"judgment_findings":1,"judgment_status":"auto_merged"}
JSONL
    run bash "$SCRIPT" remediation-report --before "$WORK/before.jsonl" --after "$WORK/after-bad.jsonl" --out "$WORK/report.md"
    [ "$status" -eq 1 ]
    [[ "$output" == *"JUDGMENT_FINDING_AUTO_MERGED:dark"* ]]
    [ ! -f "$WORK/report.md" ]

    cat > "$WORK/after-good.jsonl" <<'JSONL'
{"commit":"after","theme":"light","axe_violations":0,"pa11y_errors":0,"lighthouse_a11y":100,"ibm_violations":0,"auto_fixable":0,"judgment_findings":0}
{"commit":"after","theme":"dark","axe_violations":0,"pa11y_errors":0,"lighthouse_a11y":100,"ibm_violations":0,"auto_fixable":0,"judgment_findings":1,"judgment_status":"human_review"}
JSONL
    run bash "$SCRIPT" remediation-report --before "$WORK/before.jsonl" --after "$WORK/after-good.jsonl" --out "$WORK/report.md"
    [ "$status" -eq 0 ]
    grep -q 'Accessibility before/after' "$WORK/report.md"
    grep -q 'light axe_violations: 2 -> 0' "$WORK/report.md"
    grep -q 'dark judgment findings routed to human review' "$WORK/report.md"
}

@test "validate-design-doc rejects a doc that drops the machine-verifiable scope caveat" {
    DOC="$WORK/runbook.md"
    grep -v 'machine-verifiable subset' "$REPO_ROOT/docs/runbooks/accessibility-workstream.md" > "$DOC"
    run bash "$SCRIPT" validate-design-doc --doc "$DOC"
    [ "$status" -eq 1 ]
    [[ "$output" == *"DOC_MISSING:machine-verifiable subset"* ]]
}

@test "design doc and validation glob cite WCAG 2.2, WAI-ARIA APG, Section508.gov, Deque axe-core, and adjacent standards" {
    run bash "$SCRIPT" validate-design-doc --doc "$REPO_ROOT/docs/runbooks/accessibility-workstream.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"accessibility design doc validated"* ]]

    catalog="$REPO_ROOT/crates/autospec-core/src/validation/catalog.rs"
    grep -q '"check_autonomous_phase2_suite"' "$catalog"
    grep -q 'BatsDirectory("tests/autonomous")' "$catalog"
    [ -f "$REPO_ROOT/tests/autonomous/test_accessibility_workstream.bats" ]
    [ -f "$REPO_ROOT/.github/workflows/accessibility-workstream.yml" ]
    grep -q 'privacy/cookie UX' "$REPO_ROOT/docs/runbooks/accessibility-workstream.md"
    grep -q 'schema.org JSON-LD' "$REPO_ROOT/docs/runbooks/accessibility-workstream.md"
    grep -q 'ISO 9241' "$REPO_ROOT/docs/runbooks/accessibility-workstream.md"
}
