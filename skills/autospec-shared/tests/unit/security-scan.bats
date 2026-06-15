#!/usr/bin/env bats
# security-scan.bats — security-scan.sh maps findings to the gap contract.

setup() {
    SCRIPT_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")/../.." && pwd)"
    SCAN="${SCRIPT_DIR}/scripts/security-scan.sh"
    FIX="${SCRIPT_DIR}/tests/fixtures/secaudit"
    GAPLIB="${SCRIPT_DIR}/scripts/gap-json-lib.sh"
    TMP="$(mktemp -d /tmp/autospec-secscan-XXXXXX)"
    mkdir -p "$TMP/empty"
    BASH_BIN="$(command -v bash)"
    export AUTOSPEC_SECSCAN_FORCE_LLM=0
}

teardown() { rm -rf "$TMP"; }

# Stub gitleaks on PATH that emits one finding for our fixture.
stub_gitleaks() {
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/gitleaks" <<'EOF'
#!/usr/bin/env bash
# Minimal stub: emit gitleaks JSON report to the --report-path arg.
for a in "$@"; do case "$prev" in --report-path) out="$a";; esac; prev="$a"; done
cat > "$out" <<'JSON'
[{"RuleID":"aws-access-key","File":"config.py","StartLine":1,"Description":"AWS Access Key","Secret":"AKIAIOSFODNN7EXAMPLE"}]
JSON
exit 1
EOF
    chmod +x "$TMP/bin/gitleaks"
    export PATH="$TMP/bin:$PATH"
}

@test "gitleaks finding becomes a valid secrets gap object" {
    stub_gitleaks
    run bash "$SCAN" --tree --root "$FIX" --only secrets
    [ "$status" -eq 0 ]
    first="$(printf '%s' "$output" | head -1)"
    # Write to a real file: gap-json-lib's --validate-file uses `[ -f ]`, which
    # rejects /dev/fd process-substitution paths on bash 3.2 (macOS default).
    printf '%s' "$first" > "$TMP/first.json"
    run bash "$GAPLIB" --validate-file "$TMP/first.json"
    [ "$status" -eq 0 ]
    printf '%s' "$first" | jq -e '.dimension == "secrets" and .severity == "must-fix"'
}

stub_semgrep() {
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/semgrep" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"results":[
 {"check_id":"python.lang.security.audit.eval-detected","path":"eval.py","start":{"line":2},"extra":{"severity":"ERROR","message":"eval() on user input"}},
 {"check_id":"python.sqlalchemy.security.sqli","path":"sqli.py","start":{"line":2},"extra":{"severity":"ERROR","message":"SQL injection"}}
]}
JSON
exit 0
EOF
    chmod +x "$TMP/bin/semgrep"
    export PATH="$TMP/bin:$PATH"
}

@test "semgrep ERROR findings map to vuln must-fix gaps" {
    stub_semgrep
    run bash "$SCAN" --tree --root "$FIX" --only vuln
    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | grep -c '"dimension":"vuln"')" -eq 2 ]
    printf '%s' "$output" | grep -q '"severity":"must-fix"'
}

@test "missing scanner warns loudly and still exits 0 (LLM fallback)" {
    run env PATH="/usr/bin:/bin" bash "$SCAN" --tree --root "$FIX" --only secrets
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q "WARN scanner 'gitleaks' missing"
}

@test "engine fails closed (exit 2) when jq is unavailable" {
    run env PATH="$TMP/empty" "$BASH_BIN" "$SCAN" --tree --root "$FIX"
    [ "$status" -eq 2 ]
}
