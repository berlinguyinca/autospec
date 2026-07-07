#!/usr/bin/env bats
# Contract tests for issue #1537 proactive security/compliance workstream.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/security-workstream.sh"
    WORK="$(mktemp -d -t security-workstream.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

@test "scan: ranks SAST, secret, unsafe, and CVE findings by exploitability and exposure" {
    cat > "$WORK/raw.jsonl" <<'JSONL'
{"gap_id":"G1","dimension":"secrets","severity":"must-fix","file":"app/config.env","line":3,"title":"Hardcoded API token","body":"Remove and rotate credential","dedupe_key":"sec-secret"}
{"gap_id":"G2","dimension":"cve","severity":"nice-to-have","file":"Cargo.lock","line":0,"title":"CVE-2026-0001 in transitive package","body":"Upgrade when available","dedupe_key":"sec-cve"}
JSONL
    mkdir -p "$WORK/src"
    cat > "$WORK/src/main.rs" <<'RS'
fn main() { unsafe { std::ptr::read(0 as *const i32); } }
RS

    run bash "$SCRIPT" rank --findings "$WORK/raw.jsonl" --root "$WORK" --out "$WORK/ranked.jsonl"
    [ "$status" -eq 0 ]
    [ -f "$WORK/ranked.jsonl" ]
    first_priority="$(head -n1 "$WORK/ranked.jsonl" | jq -r '.priority')"
    [ "$first_priority" = "P0" ]
    grep -q '"dimension":"unsafe"' "$WORK/ranked.jsonl"
    grep -q '"severity_rank"' "$WORK/ranked.jsonl"
}

@test "issue filing: high-severity findings produce lint-clean remediation issues" {
    cat > "$WORK/ranked.jsonl" <<'JSONL'
{"gap_id":"G1","dimension":"secrets","severity":"must-fix","priority":"P0","severity_rank":100,"exploitability":5,"exposure":5,"file":"app/config.env","line":3,"title":"Hardcoded API token","body":"Remove the token and rotate the credential.","dedupe_key":"sec-secret","remediation":"Remove the committed token, rotate it, and add a regression scan."}
JSONL

    run bash "$SCRIPT" propose-issue --findings "$WORK/ranked.jsonl" --out "$WORK/issues"
    [ "$status" -eq 0 ]
    [ -f "$WORK/issues/p0-hardcoded-api-token.md" ]
    body="$(cat "$WORK/issues/p0-hardcoded-api-token.md")"
    [[ "$body" == *"auto-implement"* ]]
    [[ "$body" == *"priority:high"* ]]
    [[ "$body" == *"sec-secret"* ]]
    run bash "$REPO_ROOT/scripts/lint-issue.sh" "$WORK/issues/p0-hardcoded-api-token.md"
    [ "$status" -eq 0 ]
}

@test "headers: dashboard baseline gates missing CSP, HSTS, nosniff, referrer policy, and frame-ancestors" {
    cat > "$WORK/headers-ok.txt" <<'HEADERS'
Content-Security-Policy: default-src 'self'; frame-ancestors 'none'
Strict-Transport-Security: max-age=31536000; includeSubDomains
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
HEADERS
    run bash "$SCRIPT" check-headers --headers-file "$WORK/headers-ok.txt"
    [ "$status" -eq 0 ]
    [[ "$output" == *"security header baseline passed"* ]]

    cat > "$WORK/headers-bad.txt" <<'HEADERS'
Content-Security-Policy: default-src 'self'
X-Content-Type-Options: sniff
HEADERS
    run bash "$SCRIPT" check-headers --headers-file "$WORK/headers-bad.txt"
    [ "$status" -eq 1 ]
    [[ "$output" == *"missing Strict-Transport-Security"* ]]
    [[ "$output" == *"missing frame-ancestors"* ]]
    [[ "$output" == *"X-Content-Type-Options must be nosniff"* ]]
}

@test "verifier gate: security fixes cannot self-approve and sensitive domains require human review" {
    cat > "$WORK/fix-plan.json" <<'JSON'
{"author":"autospec-bot","verifier":"autospec-bot","touched_domains":["secrets"],"evidence":"bash scripts/security-workstream.sh check-headers --headers-file headers.txt"}
JSON
    run bash "$SCRIPT" verifier-gate --plan "$WORK/fix-plan.json"
    [ "$status" -eq 1 ]
    [[ "$output" == *"independent verifier required"* ]]
    [[ "$output" == *"human gate required: secrets"* ]]

    cat > "$WORK/fix-plan-ok.json" <<'JSON'
{"author":"autospec-bot","verifier":"security-reviewer","human_approved_by":"maintainer","touched_domains":["secrets"],"evidence":"bash scripts/security-workstream.sh rank --findings scan.jsonl --out ranked.jsonl"}
JSON
    run bash "$SCRIPT" verifier-gate --plan "$WORK/fix-plan-ok.json"
    [ "$status" -eq 0 ]
    [[ "$output" == *"security verifier gate passed"* ]]
}

@test "validate.sh wires the proactive security workstream and scheduled/per-PR workflow" {
    grep -q '^check_security_workstream_contract()' "$REPO_ROOT/scripts/validate.sh"
    grep -q 'security-workstream\.sh' "$REPO_ROOT/scripts/validate.sh"
    grep -q 'tests/autonomous/test_security_workstream\.bats' "$REPO_ROOT/scripts/validate.sh"
    grep -q 'schedule:' "$REPO_ROOT/.github/workflows/security-workstream.yml"
    grep -q 'pull_request:' "$REPO_ROOT/.github/workflows/security-workstream.yml"
}
