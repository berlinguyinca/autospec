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
sessionStorage.setItem('user_groups', groups.join(','));
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
    jq -e '.findings[] | select(.probe=="security-sensitive-storage" and .classification=="client-credential-storage" and .storage_api=="localStorage" and .sensitive_term=="token")' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="security-sensitive-storage" and .storage_api=="sessionStorage" and .storage_key=="user_groups")' "$OUT_JSON"
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

@test "audit command probes detect failing verification and classified dependency advisories when enabled" {
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
    "mathjs": "11.12.0",
    "xlsx": "0.18.5"
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
{
  "metadata": {"vulnerabilities": {"low": 1, "moderate": 1, "high": 2, "critical": 0, "total": 4}},
  "vulnerabilities": {
    "mathjs": {
      "name": "mathjs",
      "severity": "high",
      "isDirect": true,
      "fixAvailable": {"name": "mathjs", "version": "15.2.0", "isSemVerMajor": true}
    },
    "xlsx": {
      "name": "xlsx",
      "severity": "high",
      "isDirect": true,
      "fixAvailable": false
    },
    "dompurify": {
      "name": "dompurify",
      "severity": "moderate",
      "isDirect": false,
      "fixAvailable": true
    }
  }
}
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
    jq -e '.artifacts.npm_audit | test("artifacts/npm-audit.json$")' "$OUT_JSON"
    [ -f "$(dirname "$OUT_JSON")/artifacts/npm-audit.json" ]
    jq -e '.findings[] | select(.probe=="dependency-audit-advisory" and .package_name=="mathjs" and .dependency_type=="direct" and .fix_available==true and .semver_major_fix==true)' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="dependency-audit-advisory" and .package_name=="xlsx" and .dependency_type=="direct" and .fix_available==false)' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="dependency-audit-advisory" and .package_name=="dompurify" and .dependency_type=="transitive" and .advisory_severity=="moderate")' "$OUT_JSON"
    grep -q 'npm_audit' "$OUT_MD"
}

@test "audit detects API key and authorization group browser storage patterns" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$REPO/src/app/services"
    cat > "$REPO/src/app/services/auth.service.ts" <<'EOF'
localStorage.setItem('x-api-key', apiKey);
const accessToken = localStorage.getItem('access_token');
sessionStorage.setItem('user_groups', JSON.stringify(groups));
document.cookie = `authorization=${token}`;
EOF

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.findings[] | select(.probe=="security-sensitive-storage" and .storage_api=="localStorage" and .storage_key=="x-api-key" and .sensitive_term=="x-api-key")' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="security-sensitive-storage" and .storage_api=="localStorage" and .storage_key=="access_token")' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="security-sensitive-storage" and .storage_api=="sessionStorage" and .storage_key=="user_groups")' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="security-sensitive-storage" and .storage_api=="document.cookie" and .sensitive_term=="authorization")' "$OUT_JSON"
}

@test "audit ranks maintainability hotspots and files bounded behavior-lock follow-ups" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$REPO/src/app/feature" "$TEST_TMP/bin"
    {
        echo "/* eslint-disable @typescript-eslint/no-explicit-any */"
        echo "// @ts-ignore legacy fixture"
        for i in $(seq 1 25); do
            echo "export const value$i: any = console.log('debug-$i') as any;"
        done
    } > "$REPO/src/app/feature/giant.service.ts"
    cat > "$REPO/src/app/feature/giant.service.spec.ts" <<'EOF'
describe.skip('giant service', () => {
  it.skip('has a behavior lock', () => {});
});
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
  *"issue create"*) echo "https://github.com/example/repo/issues/40"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1
    export AUTOSPEC_QUALITY_AUDIT_LARGE_FILE_LINES=10
    export AUTOSPEC_QUALITY_AUDIT_ANY_THRESHOLD=2
    export AUTOSPEC_QUALITY_AUDIT_DEBUG_THRESHOLD=2

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD" --file-issues
    [ "$status" -eq 0 ]
    jq -e '.findings[] | select(.probe=="maintainability-hotspot" and .file=="src/app/feature/giant.service.ts" and .rank==1 and .hotspot_kind=="source" and .any_count >= 25 and .debug_logging_count >= 25 and .eslint_disable_count==1 and .ts_ignore_count==1 and (.remediation | contains("behavior locks")))' "$OUT_JSON"
    jq -e '.findings[] | select(.probe=="maintainability-hotspot" and .file=="src/app/feature/giant.service.spec.ts" and .disabled_test_count==2)' "$OUT_JSON"
    jq -e '.issue_links | map(select(.dedupe_key=="maintainability-hotspot:src/app/feature/giant.service.ts")) | length == 1' "$OUT_JSON"
    jq -e '.issue_links | map(select(.dedupe_key=="any-usage:src/app/feature/giant.service.ts" or .dedupe_key=="debug-logging-hotspots:src/app/feature/giant.service.ts" or .dedupe_key=="eslint-disable:src/app/feature/giant.service.ts" or .dedupe_key=="ts-ignore:src/app/feature/giant.service.ts")) | length == 0' "$OUT_JSON"
    run jq -e '.residual_risks[] | select(test("any-usage:src/app/feature/giant.service.ts|debug-logging-hotspots:src/app/feature/giant.service.ts|eslint-disable:src/app/feature/giant.service.ts|ts-ignore:src/app/feature/giant.service.ts"))' "$OUT_JSON"
    [ "$status" -ne 0 ]
    grep -q 'bounded refactor issue' "$GH_LOG"
    grep -q 'behavior locks or regression tests' "$GH_LOG"
    grep -q 'maintainability-hotspot / maintainability-hotspot' "$OUT_MD"
}

@test "audit does not emit blank hotspots when only generated files exist" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    rm -rf "$REPO/src" "$REPO/tests"
    mkdir -p "$REPO/dist"
    for i in $(seq 1 20); do
        echo "export const generated$i: any = console.log('generated-$i') as any;"
    done > "$REPO/dist/bundle.js"
    export AUTOSPEC_QUALITY_AUDIT_ANY_THRESHOLD=2
    export AUTOSPEC_QUALITY_AUDIT_DEBUG_THRESHOLD=2

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    [[ "$output" != *"No such file"* ]]
    run jq -e '.findings[] | select(.probe=="maintainability-hotspot")' "$OUT_JSON"
    [ "$status" -ne 0 ]
}

@test "audit ignores generated maintainability hotspots" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$REPO/dist" "$REPO/src/app/maintained"
    for i in $(seq 1 80); do
        echo "export const generated$i: any = console.log('generated-$i') as any;"
    done > "$REPO/dist/bundle.js"
    for i in $(seq 1 5); do
        echo "export const maintained$i: any = console.log('maintained-$i') as any;"
    done > "$REPO/src/app/maintained/source.ts"
    export AUTOSPEC_QUALITY_AUDIT_ANY_THRESHOLD=2
    export AUTOSPEC_QUALITY_AUDIT_DEBUG_THRESHOLD=2

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    run jq -e '.findings[] | select(.probe=="maintainability-hotspot" and .file=="dist/bundle.js")' "$OUT_JSON"
    [ "$status" -ne 0 ]
    jq -e '.findings[] | select(.probe=="maintainability-hotspot" and .file=="src/app/maintained/source.ts")' "$OUT_JSON"
}


@test "audit skips git history lookups for files below maintainability thresholds" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$REPO/src/cold" "$TEST_TMP/bin"
    for i in $(seq 1 25); do
        echo "export const cold$i = $i;" > "$REPO/src/cold/file$i.ts"
    done
    cat > "$TEST_TMP/bin/git" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "rev-parse --is-inside-work-tree") exit 0 ;;
  "status --porcelain") exit 0 ;;
  log*src/cold/*) echo "unexpected git log for cold file: $*" >> "$GIT_LOG_MARKER"; exit 0 ;;
  log*) exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/git"
    export GIT_LOG_MARKER="$TEST_TMP/git-log-marker"
    export PATH="$TEST_TMP/bin:$PATH"

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    [ ! -e "$GIT_LOG_MARKER" ]
    run jq -e '.findings[] | select(.probe=="maintainability-hotspot" and (.file | startswith("src/cold/")))' "$OUT_JSON"
    [ "$status" -ne 0 ]
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
  it('locks behavior', () => {});
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
