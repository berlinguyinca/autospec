#!/usr/bin/env bats
# repo-quality-audit.bats — tests for the shared read-only repo quality audit.

AUDIT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/repo-quality-audit.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    REPO="$TEST_TMP/repo"
    mkdir -p "$REPO/src" "$REPO/tests" "$REPO/.autospec"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20" },
  "scripts": {
    "test": "node --test",
    "typecheck": "tsc --noEmit",
    "lint": "eslint ."
  },
  "dependencies": {
    "left-pad": "1.3.0"
  }
}
EOF
    cat > "$REPO/src/app.js" <<'EOF'
console.log('debug');
localStorage.setItem('token', token);
if (value == null) {}
EOF
    cat > "$REPO/src/routes.js" <<'EOF'
export const routes = ['/dashboard', '/settings'];
EOF
    cat > "$REPO/tests/app.spec.js" <<'EOF'
describe.only('dashboard', () => {
  it.skip('covers settings', () => {});
});
EOF
    printf '{"accepted_debt":["debug-logging-hotspots:src/app.js"]}\n' \
        > "$REPO/.autospec/quality-audit-accepted.json"
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "repo-quality-audit.sh is executable and prints help" {
    [ -x "$AUDIT" ]
    run bash "$AUDIT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"repo-quality-audit.sh"* ]]
}

@test "audit writes machine JSON and markdown with classified findings" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    [ -f "$OUT_JSON" ]
    [ -f "$OUT_MD" ]

    jq -e '.status == "fail"' "$OUT_JSON"
    jq -e '.summary.total_findings >= 4' "$OUT_JSON"
    jq -e '.summary.suppressed_findings == 1' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="security-sensitive-storage" and .classification=="current-branch-regression")' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="focused-skipped-tests" and .classification=="app-follow-up")' "$OUT_JSON"
    jq -e '.suppressed[] | select(.classification=="inherited-accepted-debt")' "$OUT_JSON"
    jq -e '.residual_risks[] | select(test("Unfiled"))' "$OUT_JSON"
    jq -e '.runtime.node.version | length > 0' "$OUT_JSON"
    jq -e '.runtime.package_managers.npm.version | length > 0' "$OUT_JSON"
    jq -e '.verification.lanes.lint.status == "not run"' "$OUT_JSON"
    jq -e '.verification.lanes.test.status == "not run"' "$OUT_JSON"
    jq -e '.verification.lanes.typecheck.status == "not run"' "$OUT_JSON"
    grep -q '^# autospec repo quality audit' "$OUT_MD"
    grep -q 'security-sensitive-storage' "$OUT_MD"
    grep -q '## Verification contract' "$OUT_MD"
    grep -q 'lint: not run' "$OUT_MD"
}

@test "audit files deduplicated follow-up issues only when policy permits" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$TEST_TMP/bin"
    export GH_LOG="$TEST_TMP/gh.log"
    export GH_OPEN="$TEST_TMP/open.json"
    cat > "$GH_OPEN" <<'EOF'
[{"number":5,"title":"autospec audit: focused test markers present","body":"dedupe_key: focused-skipped-tests:tests/app.spec.js","url":"https://github.com/example/repo/issues/5","labels":[{"name":"quality-audit"}]}]
EOF
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s|%s\n' "$PWD" "$*" >> "$GH_LOG"
case "$*" in
  *"issue list"*) cat "$GH_OPEN"; exit 0 ;;
  *"label create"*) exit 0 ;;
  *"issue create"*) echo "https://github.com/example/repo/issues/10"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD" --file-issues
    [ "$status" -eq 0 ]
    [ -f "$GH_LOG" ]
    ! grep -q 'focused test markers present' "$GH_LOG"
    grep -qF -- "$REPO|issue create" "$GH_LOG"
    grep -qF -- '--label quality-audit --label auto-implement --label autospec:v2-flow' "$GH_LOG"
    jq -e '.issue_links | length >= 2' "$OUT_JSON"
    jq -e '.issue_links[] | select(.url=="https://github.com/example/repo/issues/5" and .existing == true)' "$OUT_JSON"
    jq -e '.issue_links[] | select(.url=="https://github.com/example/repo/issues/10")' "$OUT_JSON"
    ! jq -e '.residual_risks[] | select(test("focused-skipped-tests:tests/app.spec.js"))' "$OUT_JSON"
}

@test "audit command probes detect failing verification and dependency advisories when enabled" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20" },
  "scripts": {
    "test": "node --test",
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "audit": "npm audit"
  },
  "dependencies": {
    "left-pad": "1.3.0"
  }
}
EOF
    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/npm" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "run -s lint") echo "lint failed"; exit 2 ;;
  "run -s test"|"run -s typecheck") exit 0 ;;
  "run -s audit -- --json")
    cat <<'JSON'
{"metadata":{"vulnerabilities":{"total":1}},"vulnerabilities":{"left-pad":{"severity":"high"}}}
JSON
    exit 1
    ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/npm"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_RUN_COMMANDS=1

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.verification.lanes.lint.status == "configured but failing"' "$OUT_JSON"
    jq -e '.verification.lanes.test.status == "passed"' "$OUT_JSON"
    jq -e '.verification.lanes.typecheck.status == "passed"' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="verification-command" and .classification=="verification-contract-drift" and .dedupe_key=="verification-command:lint")' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="dependency-audit-advisories" and .dedupe_key=="dependency-audit:advisories")' "$OUT_JSON"
}

@test "audit reports missing scripts as not configured verification-contract drift" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20" },
  "scripts": {
    "test": "node --test"
  }
}
EOF

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.verification.lanes.lint.status == "not configured"' "$OUT_JSON"
    jq -e '.verification.lanes.typecheck.status == "not configured"' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="package-manager-scripts" and .classification=="verification-contract-drift" and .dedupe_key=="package-script-missing:lint")' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="package-manager-scripts" and .classification=="verification-contract-drift" and .dedupe_key=="package-script-missing:typecheck")' "$OUT_JSON"
}

@test "audit validates caret-or engine ranges against recorded runtime versions" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": "^20.19.0 || ^22.12.0", "npm": ">=10.0.0" },
  "scripts": {
    "test": "node --test",
    "lint": "eslint .",
    "typecheck": "tsc --noEmit"
  }
}
EOF
    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/node" <<'EOF'
#!/usr/bin/env bash
echo "v26.3.0"
EOF
    cat > "$TEST_TMP/bin/npm" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "--version") echo "11.16.0"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/node" "$TEST_TMP/bin/npm"
    export PATH="$TEST_TMP/bin:$PATH"

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.runtime.node.version == "26.3.0"' "$OUT_JSON"
    jq -e '.runtime.node.engine == "^20.19.0 || ^22.12.0"' "$OUT_JSON"
    jq -e '.runtime.node.status == "configured but failing"' "$OUT_JSON"
    jq -e '.runtime.package_managers.npm.status == "passed"' "$OUT_JSON"
    jq -e '.findings[] | select(.dedupe_key=="runtime-engine:node-version" and .classification=="verification-contract-drift")' "$OUT_JSON"
}

@test "audit validates space-separated compound engine ranges without truncating upper bounds" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20 <23" },
  "scripts": {
    "test": "node --test",
    "lint": "eslint .",
    "typecheck": "tsc --noEmit"
  }
}
EOF
    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/node" <<'EOF'
#!/usr/bin/env bash
echo "v26.3.0"
EOF
    cat > "$TEST_TMP/bin/npm" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "--version") echo "11.16.0"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/node" "$TEST_TMP/bin/npm"
    export PATH="$TEST_TMP/bin:$PATH"

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.runtime.node.status == "configured but failing"' "$OUT_JSON"
    jq -e '.findings[] | select(.dedupe_key=="runtime-engine:node-version" and (.body | contains(">=20 <23")))' "$OUT_JSON"
}
