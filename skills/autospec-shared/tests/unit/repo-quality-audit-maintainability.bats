#!/usr/bin/env bats
# repo-quality-audit-maintainability.bats — tests for the shared read-only repo quality audit.

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

@test "audit ranks newer denser test-backed maintainability hotspots first" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$REPO/src/app/ranking"
    for i in $(seq 1 100); do
        echo "export const oldValue$i: any = $i as any;"
    done > "$REPO/src/app/ranking/aa-old.ts"
    for i in $(seq 1 20); do
        echo "export const newValue$i: any = $i as any;"
    done > "$REPO/src/app/ranking/zz-new.ts"
    cat > "$REPO/src/app/ranking/zz-new.spec.ts" <<'EOF'
describe('zz-new', () => {
  it('locks behavior', () => {
    expect(recentTouch).toBe(true);
  });
});
EOF
    git -C "$REPO" init >/dev/null
    git -C "$REPO" config user.email test@example.com
    git -C "$REPO" config user.name Test
    git -C "$REPO" add .
    GIT_AUTHOR_DATE="2024-01-01T00:00:00Z" GIT_COMMITTER_DATE="2024-01-01T00:00:00Z" git -C "$REPO" commit -m old >/dev/null
    printf '\nexport const recentTouch = true;\n' >> "$REPO/src/app/ranking/zz-new.ts"
    git -C "$REPO" add .
    GIT_AUTHOR_DATE="2026-01-01T00:00:00Z" GIT_COMMITTER_DATE="2026-01-01T00:00:00Z" git -C "$REPO" commit -m new >/dev/null
    export AUTOSPEC_QUALITY_AUDIT_ANY_THRESHOLD=2

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.findings[] | select(.probe=="maintainability-hotspot" and .file=="src/app/ranking/zz-new.ts" and .rank==1 and .recent_touch=="2026-01-01" and .test_signal=="adjacent-spec")' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="maintainability-hotspot" and .file=="src/app/ranking/aa-old.ts" and .any_density > 0)' "$OUT_JSON"
}

@test "audit validates maintainability threshold config" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    cat > "$REPO/.autospec/quality-audit.json" <<'EOF'
{
  "maintainability": {
    "debug_threshold": "abc"
  }
}
EOF

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 1 ]
    [[ "$output" == *"debug_threshold must be a non-negative integer"* ]]
}

@test "audit rejects malformed quality-audit threshold config" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    printf '{not-json\n' > "$REPO/.autospec/quality-audit.json"

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 1 ]
    [[ "$output" == *".autospec/quality-audit.json must be valid JSON"* ]]
}

@test "audit discovers design/template guard scripts without running them by default" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20" },
  "scripts": {
    "test": "node --test",
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "lint:styles": "node scripts/lint-styles.js",
    "lint:templates": "node scripts/lint-templates.js"
  }
}
EOF

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.verification.lanes["lint:styles"].status == "not run"' "$OUT_JSON"
    jq -e '.verification.lanes["lint:templates"].status == "not run"' "$OUT_JSON"
    ! jq -e '.findings[] | select(.probe=="design-template-guard")' "$OUT_JSON"
}

@test "audit parses failing design/template guards and files one bounded issue per surface" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$REPO/src/app/shared-components/correction-report-panel" "$TEST_TMP/bin"
    cat > "$REPO/src/app/shared-components/correction-report-panel/correction-report-panel.component.html" <<'EOF'
<div class="my-2 alert alert-danger">Error</div>
EOF
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20" },
  "scripts": {
    "test": "node --test",
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "lint:styles": "node scripts/lint-styles.js",
    "lint:templates": "node scripts/lint-templates.js"
  }
}
EOF
    cat > "$TEST_TMP/bin/npm" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "--version") echo "11.16.0"; exit 0 ;;
  "run -s test"|"run -s typecheck"|"run -s lint") exit 0 ;;
  "run -s lint:styles")
    echo "src/app/shared-components/correction-report-panel/correction-report-panel.component.html:1: disallowed Bootstrap class my-2"
    echo "src/app/shared-components/correction-report-panel/correction-report-panel.component.html:1: disallowed Bootstrap class alert-danger"
    exit 1
    ;;
  "run -s lint:templates")
    echo "src/app/shared-components/correction-report-panel/correction-report-panel.component.html:1: template guard [alert-primary]"
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
  *"issue create"*) echo "https://github.com/example/repo/issues/20"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/npm" "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_RUN_COMMANDS=1
    export AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD" --file-issues
    [ "$status" -eq 0 ]
    jq -e '.verification.lanes["lint:styles"].status == "configured but failing"' "$OUT_JSON"
    jq -e '.verification.lanes["lint:templates"].status == "configured but failing"' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="design-template-guard" and .classification=="design-template-contract" and .guard_script=="lint:styles" and .file=="src/app/shared-components/correction-report-panel/correction-report-panel.component.html" and .line==1 and .["class"]=="my-2")' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="design-template-guard" and .guard_script=="lint:templates" and .rule=="alert-primary" and .["class"]=="alert-primary")' "$OUT_JSON"
    jq -e '.issue_links | map(select(.dedupe_key=="design-template-guard:lint:styles:src/app/shared-components/correction-report-panel/correction-report-panel.component.html")) | length == 1' "$OUT_JSON"
    jq -e '.issue_links | map(select(.dedupe_key=="design-template-guard:lint:templates:src/app/shared-components/correction-report-panel/correction-report-panel.component.html")) | length == 1' "$OUT_JSON"
    grep -q 'design-template-guard / design-template-contract' "$OUT_MD"
}

@test "audit discovers route coverage meta scripts without running them by default" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
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

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.verification.lanes["test:route-coverage"].status == "not run"' "$OUT_JSON"
    jq -e '.verification.lanes["test:route-coverage"].command == "npm run test:route-coverage"' "$OUT_JSON"
    ! jq -e '.findings[] | select(.probe=="route-registry-drift")' "$OUT_JSON"
}

@test "audit discovers direct Playwright route coverage meta specs without route-specific npm scripts" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$REPO/e2e-playwright"
    touch "$REPO/e2e-playwright/route-coverage.meta.spec.ts"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20" },
  "scripts": {
    "test": "node --test",
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "e2e": "npx playwright test"
  }
}
EOF

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.verification.lanes["route-coverage:e2e-playwright/route-coverage.meta.spec.ts"].status == "not run"' "$OUT_JSON"
    jq -e '.verification.lanes["route-coverage:e2e-playwright/route-coverage.meta.spec.ts"].command == "CI=1 npx playwright test e2e-playwright/route-coverage.meta.spec.ts --project=chrome"' "$OUT_JSON"
}

@test "audit runs discovered direct Playwright route coverage meta specs when command probes are enabled" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$REPO/e2e-playwright" "$TEST_TMP/bin"
    touch "$REPO/e2e-playwright/route-coverage.meta.spec.ts"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20" },
  "scripts": {
    "test": "node --test",
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "e2e": "npx playwright test"
  }
}
EOF
    cat > "$TEST_TMP/bin/npm" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "--version") echo "11.16.0"; exit 0 ;;
  "run -s test"|"run -s typecheck"|"run -s lint") exit 0 ;;
esac
exit 0
EOF
    cat > "$TEST_TMP/bin/npx" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "playwright test e2e-playwright/route-coverage.meta.spec.ts --project=chrome")
    cat <<'LOG'
Route coverage meta test failed
Live routes missing from ROUTE_REGISTRY:
• "database/samples"
LOG
    exit 1
    ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/npm" "$TEST_TMP/bin/npx"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_RUN_COMMANDS=1

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.verification.lanes["route-coverage:e2e-playwright/route-coverage.meta.spec.ts"].status == "configured but failing"' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="route-registry-drift" and .route_family=="database" and (.missing_routes | index("database/samples")))' "$OUT_JSON"
}
