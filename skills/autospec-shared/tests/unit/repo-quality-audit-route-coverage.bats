#!/usr/bin/env bats
# repo-quality-audit-route-coverage.bats — tests for the shared read-only repo quality audit.

if [ -z "${BATS_TEST_FILENAME:-}" ]; then
    exec bats "$0" "$@"
fi

load './repo-quality-audit-helpers'

setup() {
    setup_repo_quality_audit_fixture
}

teardown() {
    teardown_repo_quality_audit_fixture
}

@test "audit parses route registry drift and compiler warnings into bounded family issues" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$TEST_TMP/bin"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20" },
  "scripts": {
    "test": "node --test",
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "test:route-coverage": "node scripts/check-route-registry.js"
  }
}
EOF
    cat > "$TEST_TMP/bin/npm" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "--version") echo "11.16.0"; exit 0 ;;
  "run -s test"|"run -s typecheck"|"run -s lint") exit 0 ;;
  "run -s test:route-coverage")
    cat <<'LOG'
Route coverage meta test failed
baseURL: http://127.0.0.1:4200
Live routes missing from ROUTE_REGISTRY:
• "admin/assistant-site-defaults"
• "stats/admin/state-over-time"
• "stats/database-content"
Missing smoke-test catalog entry: system
Warning: src/app/app.component.ts:15:10 - warning NG8113: RouterLink is not used within the template
LOG
    exit 1
    ;;
esac
exit 0
EOF
    export GH_LOG="$TEST_TMP/gh.log"
    export GH_OPEN="$TEST_TMP/open.json"
    printf '[]\n' > "$GH_OPEN"
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s|%s\n' "$PWD" "$*" >> "$GH_LOG"
case "$*" in
  *"issue list"*) cat "$GH_OPEN"; exit 0 ;;
  *"label create"*) exit 0 ;;
  *"issue create"*) echo "https://github.com/example/repo/issues/30"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/npm" "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_RUN_COMMANDS=1
    export AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD" --file-issues
    [ "$status" -eq 0 ]
    jq -e '.verification.lanes["test:route-coverage"].status == "configured but failing"' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="route-registry-drift" and .route_family=="admin" and (.missing_routes | index("admin/assistant-site-defaults")))' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="route-registry-drift" and .route_family=="stats" and (.missing_routes | index("stats/admin/state-over-time")) and (.missing_routes | index("stats/database-content")))' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="route-registry-drift" and .route_family=="system" and (.missing_routes | index("system")))' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="route-coverage-compiler-warning" and .warning_code=="NG8113" and (.body | contains("RouterLink")))' "$OUT_JSON"
    jq -e '.issue_links | map(select(.dedupe_key=="route-registry-drift:test:route-coverage:admin")) | length == 1' "$OUT_JSON"
    jq -e '.issue_links | map(select(.dedupe_key=="route-registry-drift:test:route-coverage:stats")) | length == 1' "$OUT_JSON"
    jq -e '.issue_links | map(select(.dedupe_key=="route-registry-drift:test:route-coverage:system")) | length == 1' "$OUT_JSON"
    grep -q 'route-registry-drift / app-follow-up' "$OUT_MD"
    grep -q 'NG8113' "$OUT_MD"
}

@test "audit classifies route coverage setup failures without route drift findings" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$TEST_TMP/bin"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20" },
  "scripts": {
    "test": "node --test",
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "test:route-coverage": "playwright test route-coverage.spec.ts"
  }
}
EOF
    cat > "$TEST_TMP/bin/npm" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "--version") echo "11.16.0"; exit 0 ;;
  "run -s test"|"run -s typecheck"|"run -s lint") exit 0 ;;
  "run -s test:route-coverage")
    echo "Error: connect ECONNREFUSED 127.0.0.1:4200"
    echo "Is the dev server running?"
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
    jq -e '.verification.lanes["test:route-coverage"].status == "setup failure"' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="route-coverage-setup" and .classification=="verification-contract-drift")' "$OUT_JSON"
    ! jq -e '.findings[] | select(.probe=="route-registry-drift")' "$OUT_JSON"
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

@test "audit large-file probe handles spaces and prunes generated state" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    rm -rf "$REPO/src" "$REPO/tests"
    mkdir -p "$REPO/src/assets/next steps" "$REPO/.autospec/refinements/next-steps" "$REPO/dist"
    python3 - <<PY
from pathlib import Path
Path('$REPO/src/assets/next steps/large artifact.txt').write_text('a' * 530000)
Path('$REPO/.autospec/refinements/next-steps/generated report.md').write_text('b' * 530000)
Path('$REPO/dist/bundle.js').write_text('c' * 530000)
PY

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    [[ "$output" != *"No such file"* ]]
    [[ "$output" != *"integer expression expected"* ]]
    jq -e '.findings[] | select(.probe=="large-files" and .file=="src/assets/next steps/large artifact.txt")' "$OUT_JSON"
    run jq -e '.findings[] | select(.probe=="large-files" and (.file | startswith(".autospec/") or startswith("dist/")))' "$OUT_JSON"
    [ "$status" -ne 0 ]
}
