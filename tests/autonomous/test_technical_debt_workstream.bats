#!/usr/bin/env bats
# tests/technical-debt-workstream.bats — contract tests for issue #1535 debt/dead-code/CVE workstream.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/technical-debt-workstream.sh"
    WORK="$(mktemp -d -t technical-debt-workstream.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

@test "hotspots: ranks by churn times complexity with duplicate tie-break evidence" {
    cat > "$WORK/churn.jsonl" <<'JSONL'
{"file":"src/cold.rs","commits":2}
{"file":"src/hot.rs","commits":12}
{"file":"src/medium.rs","commits":9}
JSONL
    cat > "$WORK/complexity.jsonl" <<'JSONL'
{"file":"src/cold.rs","complexity":40}
{"file":"src/hot.rs","complexity":11}
{"file":"src/medium.rs","complexity":13}
JSONL
    cat > "$WORK/duplicates.jsonl" <<'JSONL'
{"file":"src/medium.rs","duplicate_lines":30,"cluster":"validation"}
JSONL

    run bash "$SCRIPT" rank-hotspots --churn "$WORK/churn.jsonl" --complexity "$WORK/complexity.jsonl" --duplicates "$WORK/duplicates.jsonl" --out "$WORK/hotspots.jsonl" --limit 3
    [ "$status" -eq 0 ]
    [[ "$output" == *"ranked 3 hotspots"* ]]

    first="$(head -n 1 "$WORK/hotspots.jsonl")"
    python3 - <<PY
import json
row=json.loads('''$first''')
assert row['file'] == 'src/hot.rs', row
assert row['score'] == 132, row
assert row['churn'] == 12 and row['complexity'] == 11, row
assert 'churn×complexity' in row['evidence'], row
PY
}

@test "hotspots: top ranked item becomes a lint-clean verified refactor issue" {
    cat > "$WORK/hotspots.jsonl" <<'JSONL'
{"file":"src/hot.rs","score":132,"churn":12,"complexity":11,"duplicate_lines":0,"evidence":"churn×complexity=132"}
JSONL

    run bash "$SCRIPT" propose-refactor-issue --hotspots "$WORK/hotspots.jsonl" --out "$WORK/issues" --test-cmd "cargo test -p autospec-core"
    [ "$status" -eq 0 ]
    [ -f "$WORK/issues/src-hot-rs-refactor.md" ]
    body="$(cat "$WORK/issues/src-hot-rs-refactor.md")"
    [[ "$body" == *"churn×complexity=132"* ]]
    [[ "$body" == *"cargo test -p autospec-core"* ]]
    [[ "$body" == *"verified refactor PR"* ]]
    run bash "$REPO_ROOT/scripts/lint-issue.sh" "$WORK/issues/src-hot-rs-refactor.md"
    [ "$status" -eq 0 ]
}

@test "dead-code: test-only references produce a safe removal issue and production references are skipped" {
    cat > "$WORK/symbols.jsonl" <<'JSONL'
{"symbol":"legacy_test_helper","file":"src/legacy.rs","referenced_by":["tests/legacy_test.rs"],"kind":"fn"}
{"symbol":"live_api","file":"src/api.rs","referenced_by":["src/main.rs","tests/api_test.rs"],"kind":"fn"}
JSONL

    run bash "$SCRIPT" propose-dead-code-removal --symbols "$WORK/symbols.jsonl" --out "$WORK/issues" --test-cmd "cargo test --workspace"
    [ "$status" -eq 0 ]
    [[ "$output" == *"wrote 1 dead-code removal issues"* ]]
    [ -f "$WORK/issues/src-legacy-rs-legacy-test-helper-removal.md" ]
    [ ! -f "$WORK/issues/src-api-rs-live-api-removal.md" ]
    body="$(cat "$WORK/issues/src-legacy-rs-legacy-test-helper-removal.md")"
    [[ "$body" == *"referenced only from tests"* ]]
    [[ "$body" == *"Analysis proposes; removal requires this verified PR"* ]]
    run bash "$REPO_ROOT/scripts/lint-issue.sh" "$WORK/issues/src-legacy-rs-legacy-test-helper-removal.md"
    [ "$status" -eq 0 ]
}

@test "advisories: records cadence scan and prioritizes active fixable CVEs by CVSS" {
    cat > "$WORK/advisories.jsonl" <<'JSONL'
{"id":"CVE-2026-9999","package":"leftpad","current":"1.0.0","fixed":"1.0.3","cvss":9.8,"active":true,"manifest":"package-lock.json","title":"critical exploit"}
{"id":"CVE-2026-1111","package":"serde","current":"1.0.0","fixed":"1.0.1","cvss":5.0,"active":false,"manifest":"Cargo.lock","title":"moderate bug"}
JSONL

    run bash "$SCRIPT" scan-advisories --advisories "$WORK/advisories.jsonl" --ledger "$WORK/scans.jsonl" --out "$WORK/issues" --timestamp 2026-07-07T05:00:00Z
    [ "$status" -eq 0 ]
    [[ "$output" == *"recorded advisory scan findings=2"* ]]
    first_issue="$(ls "$WORK/issues" | head -n 1)"
    [ "$first_issue" = "p1-cve-2026-9999-leftpad.md" ]
    grep -q '"timestamp":"2026-07-07T05:00:00Z"' "$WORK/scans.jsonl"
    body="$(cat "$WORK/issues/p1-cve-2026-9999-leftpad.md")"
    [[ "$body" == *"CVSS 9.8"* ]]
    [[ "$body" == *"active exploit"* ]]
    [[ "$body" == *"package-lock.json"* ]]
    run bash "$REPO_ROOT/scripts/lint-issue.sh" "$WORK/issues/p1-cve-2026-9999-leftpad.md"
    [ "$status" -eq 0 ]
}
