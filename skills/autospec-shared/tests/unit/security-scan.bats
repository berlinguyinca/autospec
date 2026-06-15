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
    # Under --only vuln, only the eval-detected finding stays in the vuln
    # dimension; the sqli finding is now classified as injection and filtered.
    [ "$(printf '%s\n' "$output" | grep -c '"dimension":"vuln"')" -eq 1 ]
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

@test "trivy CVE findings are advisory (nice-to-have), not blocking" {
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/trivy" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"Results":[{"Target":"package-lock.json","Vulnerabilities":[{"VulnerabilityID":"CVE-2021-1234","PkgName":"lodash","Severity":"HIGH","Title":"proto pollution","FixedVersion":""}]}]}
JSON
exit 0
EOF
    chmod +x "$TMP/bin/trivy"
    run env PATH="$TMP/bin:/usr/bin:/bin" bash "$SCAN" --tree --root "$FIX" --only cve
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e 'select(.dimension=="cve") | .severity == "nice-to-have"'
}

@test "--diff scopes findings to changed files only" {
    git -C "$TMP" init -q
    printf 'ok = 1\n' > "$TMP/clean.py"
    git -C "$TMP" add clean.py
    git -C "$TMP" -c user.email=t@t -c user.name=t commit -qm base
    cp "$FIX/sqli.py" "$TMP/sqli.py"
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/semgrep" <<'EOF'
#!/usr/bin/env bash
root="${@: -1}"
results=""
for f in $(find "$root" -name '*.py' 2>/dev/null); do
  rel="${f#$root/}"
  results="$results{\"check_id\":\"x\",\"path\":\"$rel\",\"start\":{\"line\":1},\"extra\":{\"severity\":\"ERROR\",\"message\":\"m\"}},"
done
results="${results%,}"
printf '{"results":[%s]}\n' "$results"
EOF
    chmod +x "$TMP/bin/semgrep"
    export PATH="$TMP/bin:$PATH"
    run bash "$SCAN" --diff HEAD --root "$TMP" --only vuln
    [ "$status" -eq 0 ]
    ! printf '%s' "$output" | grep -q 'clean.py'
}

@test "gitleaks is invoked in directory mode (not git-history mode)" {
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/gitleaks" <<EOF
#!/usr/bin/env bash
echo "\$@" >> "$TMP/gitleaks-argv.txt"
# honor --help probe so the script picks 'dir' mode
case "\$1" in dir) [ "\$2" = "--help" ] && exit 0 ;; esac
# write an empty report to the --report-path
prev=""; for a in "\$@"; do [ "\$prev" = "--report-path" ] && echo "[]" > "\$a"; prev="\$a"; done
exit 0
EOF
    chmod +x "$TMP/bin/gitleaks"
    export PATH="$TMP/bin:$PATH"
    run bash "$SCAN" --tree --root "$FIX" --only secrets
    [ "$status" -eq 0 ]
    # The real (non --help) invocation must be directory mode.
    grep -vE '^dir --help' "$TMP/gitleaks-argv.txt" | grep -qE '(^| )dir( |$)|--no-git'
}

@test "--diff cleans up its temp scan tree (no leak)" {
    git -C "$TMP" init -q
    printf 'ok=1\n' > "$TMP/a.py"
    git -C "$TMP" add a.py
    git -C "$TMP" -c user.email=t@t -c user.name=t commit -qm base
    cp "$FIX/sqli.py" "$TMP/sqli.py"
    mkdir -p "$TMP/tmphome"
    before="$(find "$TMP/tmphome" -mindepth 1 -maxdepth 1 -type d | wc -l)"
    run env TMPDIR="$TMP/tmphome" bash "$SCAN" --diff HEAD --root "$TMP" --only vuln
    [ "$status" -eq 0 ]
    after="$(find "$TMP/tmphome" -mindepth 1 -maxdepth 1 -type d | wc -l)"
    [ "$before" -eq "$after" ]
}

@test "semgrep sqli check maps to injection dimension under --only injection" {
    stub_semgrep   # existing helper: emits a check_id 'python.sqlalchemy.security.sqli' for sqli.py
    run bash "$SCAN" --tree --root "$FIX" --only injection
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q '"dimension":"injection"'
    # the eval finding (check_id ...eval-detected) is NOT injection, so under --only injection it is filtered out
    ! printf '%s' "$output" | grep -q 'eval-detected'
}

@test "scan_pii flags PII written to a log sink (nice-to-have)" {
    run bash "$SCAN" --tree --root "$FIX" --only pii
    [ "$status" -eq 0 ]
    # pii-log.js logs ssn/email as values → nice-to-have pii candidate
    # (FP-prone heuristic; LLM triage upgrades real findings)
    printf '%s' "$output" | grep -E 'pii-log.js' | grep -q '"dimension":"pii"'
    printf '%s' "$output" | grep -E 'pii-log.js' | grep -q '"severity":"nice-to-have"'
}

@test "scan_pii does not flag bare PII words or non-logged PII (low false positives)" {
    run bash "$SCAN" --tree --root "$FIX" --only pii
    [ "$status" -eq 0 ]
    # pii-ui.js ("email address" prose) and pii-nosink.py (assignment, no sink) must NOT appear
    ! printf '%s' "$output" | grep -q 'pii-ui.js'
    ! printf '%s' "$output" | grep -q 'pii-nosink.py'
}

@test "scan_promptinj flags user input concatenated into an LLM prompt (advisory)" {
    run bash "$SCAN" --tree --root "$FIX" --only injection
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q 'promptinj.py'
    # advisory candidate: nice-to-have (LLM triage upgrades confirmed cases)
    printf '%s' "$output" | grep -E 'promptinj.py' | grep -q '"dimension":"injection"'
    printf '%s' "$output" | grep -E 'promptinj.py' | grep -q '"severity":"nice-to-have"'
}

@test "scan_promptinj does not flag benign string concatenation (no LLM sink)" {
    run bash "$SCAN" --tree --root "$FIX" --only injection
    [ "$status" -eq 0 ]
    ! printf '%s' "$output" | grep -q 'concat-benign.py'
}
