#!/usr/bin/env bats
# repo-quality-audit.bats — tests for the shared read-only repo quality audit.

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

@test "mock policy classifies forbidden integration broker mock" {
    printf '%s\n' 'Real-service testing is required; tests must not use broker or DB mocks.' > "$REPO/AGENTS.md"
    mkdir -p "$REPO/tests/integration"
    cat > "$REPO/tests/integration/broker_test.js" <<'EOF'
import { wiremockBroker } from 'wiremock';
EOF
    run bash "$AUDIT" --repo "$REPO" --json "$TEST_TMP/audit.json" --markdown "$TEST_TMP/audit.md"
    [ "$status" -eq 0 ]
    jq -e '.findings[] | select(.probe=="mock-policy" and .classification=="mock-policy-integration" and (.body|contains("wiremock")) and (.body|contains("broker/DB")))' "$TEST_TMP/audit.json"
}

@test "f64 invariant probe classifies monetary and statistical fields" {
    printf '%s\n' 'Numeric invariants: money and statistical metrics must use Decimal.' > "$REPO/AGENTS.md"
    mkdir -p "$REPO/crates/pricing/src" "$REPO/tests"
    cat > "$REPO/crates/pricing/Cargo.toml" <<'EOF'
[package]
name = "pricing"
EOF
    printf 'pub struct Quote { pub price: f64, pub sharpe: f64 }\n' > "$REPO/crates/pricing/src/lib.rs"
    printf 'let price: f64 = 1.0;\n' > "$REPO/tests/fixture.rs"
    run bash "$AUDIT" --repo "$REPO" --json "$TEST_TMP/audit.json" --markdown "$TEST_TMP/audit.md"
    [ "$status" -eq 0 ]
    run jq -e '[.findings[] | select(.probe=="f64-numeric-invariant" and (.classification=="money/price" or .classification=="statistical metric"))] | length >= 1' "$TEST_TMP/audit.json"
    [ "$status" -eq 0 ]
}

@test "f64 bridge annotation suppresses invariant finding" {
    printf '%s\n' 'Numeric invariant: prices use Decimal.' > "$REPO/AGENTS.md"
    mkdir -p "$REPO/crates/bridge/src"
    cat > "$REPO/crates/bridge/Cargo.toml" <<'EOF'
[package]
name = "bridge"
EOF
    printf 'pub fn price_bridge() { let price: f64 = 1.0; } // quality-audit: f64-bridge external API\n' > "$REPO/crates/bridge/src/lib.rs"
    run bash "$AUDIT" --repo "$REPO" --json "$TEST_TMP/audit.json" --markdown "$TEST_TMP/audit.md"
    [ "$status" -eq 0 ]
    ! jq -e '.findings[] | select(.probe=="f64-numeric-invariant")' "$TEST_TMP/audit.json"
}

@test "mock policy exception annotation suppresses finding" {
    printf '%s\n' 'Real-service testing is required; tests must not use broker or DB mocks.' > "$REPO/AGENTS.md"
    mkdir -p "$REPO/tests/integration"
    cat > "$REPO/tests/integration/broker_test.js" <<'EOF'
// quality-audit: mock-exception local broker contract fixture
import { wiremockBroker } from 'wiremock';
EOF
    run bash "$AUDIT" --repo "$REPO" --json "$TEST_TMP/audit.json" --markdown "$TEST_TMP/audit.md"
    [ "$status" -eq 0 ]
    ! jq -e '.findings[] | select(.probe=="mock-policy")' "$TEST_TMP/audit.json"
}

@test "async-aware lock probe reports only locks with async evidence" {
    mkdir -p "$REPO/src"
    cat > "$REPO/src/locks.rs" <<'EOF'
async fn worker(lock: std::sync::Mutex<i32>) { let _guard = lock.lock().unwrap(); do_work().await; }





































fn callback(lock: std::sync::Mutex<i32>) { let _guard = lock.lock().unwrap(); }
EOF
    OUT_JSON="$TEST_TMP/locks.json"; OUT_MD="$TEST_TMP/locks.md"
    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    jq -e '.findings[] | select(.probe=="sync-lock-async-aware" and .classification=="production-async-lock")' "$OUT_JSON"
    ! jq -e '.findings[] | select(.probe=="sync-lock-async-aware" and .file=="src/locks.rs" and .line>1)' "$OUT_JSON"
}

@test "annotated synchronous boundary and test-only locks are classified" {
    mkdir -p "$REPO/src" "$REPO/tests"
    printf '%s\n' 'fn callback(lock: std::sync::Mutex<i32>) { // quality-audit: sync-boundary callback' 'let _guard = lock.lock().unwrap(); }' > "$REPO/src/callback.rs"
    printf '%s\n' 'async fn test_lock(lock: std::sync::Mutex<i32>) { let _guard = lock.lock().unwrap(); work().await; }' > "$REPO/tests/locks.rs"
    OUT_JSON="$TEST_TMP/locks.json"; OUT_MD="$TEST_TMP/locks.md"
    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"
    [ "$status" -eq 0 ]
    ! jq -e '.findings[] | select(.file=="src/callback.rs" and .probe=="sync-lock-async-aware")' "$OUT_JSON"
    jq -e '.findings[] | select(.file=="tests/locks.rs" and .classification=="test-only-async-lock")' "$OUT_JSON"
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

@test "audit assembles reports when findings exceed the per-argument size limit" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    : > "$REPO/src/app.js"
    for index in $(seq 1 500); do
        printf "localStorage.setItem('token_%04d', token);\\n" "$index"
    done > "$REPO/src/storage.js"

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD"

    [ "$status" -eq 0 ]
    jq -e '[.findings[] | select(.probe == "security-sensitive-storage")] | length == 500' "$OUT_JSON"
    jq -e '.summary.total_findings == (.findings | length)' "$OUT_JSON"
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

@test "audit normalizes foreign worktree paths in files titles and dedupe identities" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    FOREIGN="$TEST_TMP/foreign-worktree"
    UNRELATED="$TEST_TMP/unrelated"
    mkdir -p "$REPO/src/app/components" "$UNRELATED/src" "$TEST_TMP/bin"
    touch "$REPO/src/app/components/panel.component.html"
    touch "$REPO/src/shared.html"
    cat > "$REPO/package.json" <<'EOF'
{
  "engines": { "node": ">=20" },
  "scripts": {
    "test": "node --test",
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "lint:templates": "node scripts/lint-templates.js"
  }
}
EOF
    git -C "$REPO" init -q
    git -C "$REPO" config user.email fixture@example.com
    git -C "$REPO" config user.name Fixture
    git -C "$REPO" add .
    git -C "$REPO" commit -qm fixture
    git -C "$REPO" worktree add -q -b foreign-fixture "$FOREIGN"
    cat > "$TEST_TMP/bin/npm" <<EOF
#!/usr/bin/env bash
case "\$*" in
  "--version") echo "11.16.0"; exit 0 ;;
  "run -s test"|"run -s typecheck"|"run -s lint") exit 0 ;;
  "run -s lint:templates")
    echo "$FOREIGN/src/app/components/panel.component.html:7: template guard [alert-primary]"
    echo "$UNRELATED/src/shared.html:8: template guard [alert-secondary]"
    echo "/outside/unrelated/etc/passwd.html:9: template guard [alert-warning]"
    echo "/../../../etc/passwd.html:10: template guard [alert-danger]"
    exit 1
    ;;
esac
exit 0
EOF
    export GH_LOG="$TEST_TMP/gh.log"
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"issue list"*) printf '[]\n'; exit 0 ;;
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
    jq -e '.findings[] | select(.probe=="design-template-guard" and .file=="src/app/components/panel.component.html" and .normalized_path=="src/app/components/panel.component.html" and .normalized_title=="design-template-guard-failure-in-src-app-components-panel-component-html" and (.dedupe_key | contains("|path=src/app/components/panel.component.html|title=design-template-guard-failure-in-src-app-components-panel-component-html")))' "$OUT_JSON"
    jq -e '[.findings[] | select(.probe=="design-template-guard" and (.normalized_path | startswith("external/"))) | .normalized_path] | (length==3 and (unique|length)==3)' "$OUT_JSON"
    ! jq -e '.findings[] | select(.normalized_path=="src/shared.html")' "$OUT_JSON"
    ! jq -e '[.findings[] | .file, .title, .dedupe_key] | any(contains("/tmp/"))' "$OUT_JSON"
    ! grep -q '/tmp/' "$GH_LOG"
}

@test "audit normalizes a legacy raw-worktree closed match before exact reopen" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    FOREIGN="$TEST_TMP/new-worktree"
    mkdir -p "$REPO/src/app/components" "$TEST_TMP/bin"
    touch "$REPO/src/app/components/panel.component.html"
    cat > "$REPO/package.json" <<'EOF'
{"engines":{"node":">=20"},"scripts":{"test":"node --test","typecheck":"tsc --noEmit","lint":"eslint .","lint:templates":"node guard.js"}}
EOF
    git -C "$REPO" init -q
    git -C "$REPO" config user.email fixture@example.com
    git -C "$REPO" config user.name Fixture
    git -C "$REPO" add .
    git -C "$REPO" commit -qm fixture
    git -C "$REPO" worktree add -q -b legacy-fixture "$FOREIGN"
    cat > "$TEST_TMP/bin/npm" <<EOF
#!/usr/bin/env bash
case "\$*" in
  "--version") echo "11.16.0"; exit 0 ;;
  "run -s test"|"run -s typecheck"|"run -s lint") exit 0 ;;
  "run -s lint:templates") echo "$FOREIGN/src/app/components/panel.component.html:7: template guard [alert-primary]"; exit 1 ;;
esac
exit 0
EOF
    export GH_LOG="$TEST_TMP/gh.log"
    export GH_CLOSED="$TEST_TMP/closed.json"
    cat > "$GH_CLOSED" <<'EOF'
[{"number":5,"state":"CLOSED","title":"autospec audit: design/template guard failure in /tmp/old-worktree/src/app/components/panel.component.html","body":"## Evidence\n- dedupe_key: design-template-guard:lint:templates:/tmp/old-worktree/src/app/components/panel.component.html","url":"https://github.com/example/repo/issues/5"}]
EOF
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"issue list --state open"*) printf '[]\n'; exit 0 ;;
  *"issue list --state closed"*) cat "$GH_CLOSED"; exit 0 ;;
  *"label create"*) exit 0 ;;
  *"issue comment 5 --body-file"*) exit 0 ;;
  *"issue reopen 5"*) exit 0 ;;
  *"issue create"*) echo "https://github.com/example/repo/issues/10"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh" "$TEST_TMP/bin/npm"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1
    export AUTOSPEC_QUALITY_AUDIT_RUN_COMMANDS=1

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD" --file-issues
    [ "$status" -eq 0 ]
    grep -q 'issue comment 5 --body-file' "$GH_LOG"
    grep -q 'issue reopen 5' "$GH_LOG"
    ! grep -q 'issue create --title autospec audit: design/template guard failure' "$GH_LOG"
    jq -e '.issue_links[] | select(.url=="https://github.com/example/repo/issues/5" and .existing==true and .reopened==true)' "$OUT_JSON"
}

@test "audit reopens a legacy embedded-path storage identity only on full semantic match" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$TEST_TMP/bin"
    export GH_LOG="$TEST_TMP/gh.log"
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"issue list --state open"*) printf '[]\n'; exit 0 ;;
  *"issue list --state closed"*) printf '%s\n' '[{"number":8,"state":"CLOSED","title":"autospec audit: security-sensitive browser storage in /tmp/old-worktree/src/app.js","body":"- dedupe_key: security-sensitive-storage:/tmp/old-worktree/src/app.js:localStorage:token:token","url":"https://github.com/example/repo/issues/8"},{"number":9,"state":"CLOSED","title":"autospec audit: security-sensitive browser storage in /tmp/other/src/other.js","body":"- dedupe_key: security-sensitive-storage:/tmp/other/src/other.js:sessionStorage:credential","url":"https://github.com/example/repo/issues/9"}]'; exit 0 ;;
  *"label create"*) exit 0 ;;
  *"issue comment 8 --body-file"*) exit 0 ;;
  *"issue reopen 8"*) exit 0 ;;
  *"issue create"*) echo "https://github.com/example/repo/issues/10"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD" --file-issues
    [ "$status" -eq 0 ]
    grep -q 'issue comment 8 --body-file' "$GH_LOG"
    grep -q 'issue reopen 8' "$GH_LOG"
    ! grep -q 'issue comment 9\|issue reopen 9' "$GH_LOG"
    jq -e '.issue_links[] | select(.url=="https://github.com/example/repo/issues/8" and .existing==true and .reopened==true)' "$OUT_JSON"
}

@test "audit comments exact closed matches before reopening without creating a replacement" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$TEST_TMP/bin"
    export GH_LOG="$TEST_TMP/gh.log"
    export GH_CLOSED="$TEST_TMP/closed.json"
    cat > "$GH_CLOSED" <<'EOF'
[{"number":5,"state":"CLOSED","title":"autospec audit: focused test markers present","body":"<!-- autospec-quality-audit-dedupe:v2:focused-skipped-tests:tests/app.spec.js|path=tests/app.spec.js|title=focused-test-markers-present -->","url":"https://github.com/example/repo/issues/5","labels":[{"name":"quality-audit"}]}]
EOF
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"issue list --state open"*) printf '[]\n'; exit 0 ;;
  *"issue list --state closed"*) cat "$GH_CLOSED"; exit 0 ;;
  *"label create"*) exit 0 ;;
  *"issue comment 5 --body-file"*) grep -q 'focused-skipped-tests:tests/app.spec.js' "${*: -1}"; exit $? ;;
  *"issue reopen 5"*) exit 0 ;;
  *"issue create"*) echo "https://github.com/example/repo/issues/10"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1

    run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD" --file-issues
    [ "$status" -eq 0 ]
    comment_line="$(grep -n 'issue comment 5 --body-file' "$GH_LOG" | cut -d: -f1)"
    reopen_line="$(grep -n 'issue reopen 5' "$GH_LOG" | cut -d: -f1)"
    [ -n "$comment_line" ]
    [ -n "$reopen_line" ]
    [ "$comment_line" -lt "$reopen_line" ]
    ! grep -q 'issue create --title autospec audit: focused test markers present' "$GH_LOG"
    jq -e '.issue_links[] | select(.url=="https://github.com/example/repo/issues/5" and .existing==true and .reopened==true)' "$OUT_JSON"
}

@test "audit files fresh when the same title belongs to a different canonical rule and path" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    mkdir -p "$TEST_TMP/bin"
    export GH_LOG="$TEST_TMP/gh.log"
    export GH_CLOSED="$TEST_TMP/closed.json"
    cat > "$GH_CLOSED" <<'EOF'
[{"number":6,"state":"CLOSED","title":"autospec audit: focused test markers present","body":"<!-- autospec-quality-audit-dedupe:v2:any-usage:tests/other.spec.js|path=tests/other.spec.js|title=focused-test-markers-present -->","url":"https://github.com/example/repo/issues/6","labels":[{"name":"quality-audit"}]},{"number":7,"state":"CLOSED","title":"autospec audit: focused test markers present","body":"## Evidence\n- dedupe_key: any-usage:/tmp/old-worktree/tests/other.spec.js","url":"https://github.com/example/repo/issues/7","labels":[{"name":"quality-audit"}]}]
EOF
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"issue list --state open"*) printf '[]\n'; exit 0 ;;
  *"issue list --state closed"*) cat "$GH_CLOSED"; exit 0 ;;
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
    grep -q 'issue create --title autospec audit: focused test markers present' "$GH_LOG"
    ! grep -q 'issue comment [67]\|issue reopen [67]' "$GH_LOG"
}

@test "audit open or closed lookup errors and malformed catalogs fail closed without GitHub mutation" {
    mkdir -p "$TEST_TMP/bin"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"issue list --state open"*)
    case "$LOOKUP_MODE" in
      failed-open) exit 1 ;;
      malformed-open) printf '{malformed\n'; exit 0 ;;
      *) printf '[]\n'; exit 0 ;;
    esac
    ;;
  *"issue list --state closed"*)
    case "$LOOKUP_MODE" in
      failed-closed) exit 1 ;;
      malformed-closed) printf '{malformed\n'; exit 0 ;;
      *) printf '[]\n'; exit 0 ;;
    esac
    ;;
  *) echo "unexpected mutation: $*" >> "$GH_LOG"; exit 0 ;;
esac
EOF
    chmod +x "$TEST_TMP/bin/gh"

    for LOOKUP_MODE in failed-open malformed-open failed-closed malformed-closed; do
        export LOOKUP_MODE
        export GH_LOG="$TEST_TMP/gh-$LOOKUP_MODE.log"
        OUT_JSON="$TEST_TMP/audit-$LOOKUP_MODE.json"
        OUT_MD="$TEST_TMP/audit-$LOOKUP_MODE.md"
        run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD" --file-issues
        [ "$status" -eq 0 ]
        ! grep -q 'unexpected mutation' "$GH_LOG"
        ! grep -Eq 'issue (create|comment|reopen)|label create' "$GH_LOG"
        jq -e '.summary.issue_links == 0 and .summary.unfiled_residual_risks > 0' "$OUT_JSON"
    done
}

@test "audit never creates a replacement when closed comment or reopen mutation fails" {
    mkdir -p "$TEST_TMP/bin"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1
    export GH_LOG="$TEST_TMP/gh.log"
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"issue list --state open"*) printf '[]\n'; exit 0 ;;
  *"issue list --state closed"*) printf '%s\n' '[{"number":5,"state":"CLOSED","title":"autospec audit: focused test markers present","body":"<!-- autospec-quality-audit-dedupe:v2:focused-skipped-tests:tests/app.spec.js|path=tests/app.spec.js|title=focused-test-markers-present -->","url":"https://github.com/example/repo/issues/5"}]'; exit 0 ;;
  *"label create"*) exit 0 ;;
  *"issue comment 5 --body-file"*) [ "$MUTATION_MODE" = "comment-fails" ] && exit 1 || exit 0 ;;
  *"issue reopen 5"*) [ "$MUTATION_MODE" = "reopen-fails" ] && exit 1 || exit 0 ;;
  *"issue create"*) echo "https://github.com/example/repo/issues/10"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    for MUTATION_MODE in comment-fails reopen-fails; do
        export MUTATION_MODE
        export GH_LOG="$TEST_TMP/gh-$MUTATION_MODE.log"
        OUT_JSON="$TEST_TMP/audit-$MUTATION_MODE.json"
        OUT_MD="$TEST_TMP/audit-$MUTATION_MODE.md"
        run bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD" --file-issues
        [ "$status" -eq 0 ]
        grep -q 'issue comment 5 --body-file' "$GH_LOG"
        ! grep -q 'issue create --title autospec audit: focused test markers present' "$GH_LOG"
        ! jq -e '.issue_links[] | select(.url=="https://github.com/example/repo/issues/5")' "$OUT_JSON"
        jq -e '.residual_risks[] | select(contains("focused test markers present"))' "$OUT_JSON"
    done
}


@test "Phase 6 defaults quality audit issue filing on and marks residual risks incomplete" {
    OUT_JSON="$TEST_TMP/audit.json"
    OUT_MD="$TEST_TMP/audit.md"
    RUN_SUMMARY="$TEST_TMP/run-summary.md"
    CHALLENGE="$TEST_TMP/challenge.md"
    mkdir -p "$TEST_TMP/bin"
    export GH_LOG="$TEST_TMP/gh.log"
    export GH_OPEN="$TEST_TMP/open.json"
    printf '[]\n' > "$GH_OPEN"
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s|%s\n' "$PWD" "$*" >> "$GH_LOG"
case "$*" in
  *"issue list"*) cat "$GH_OPEN"; exit 0 ;;
  *"label create"*) exit 0 ;;
  *"issue create"*) exit 1 ;;
esac
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    unset AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES

    grep -q 'AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES="${AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES:-1}"' "$RUN_END_OF_RUN"
    run env AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES="${AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES:-1}" \
        bash "$AUDIT" --repo "$REPO" --json "$OUT_JSON" --markdown "$OUT_MD" \
        $([ "${AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES:-1}" != "0" ] && printf '%s' "--file-issues")
    [ "$status" -eq 0 ]
    grep -qF -- "$REPO|issue create" "$GH_LOG"
    jq -e '.status == "fail"' "$OUT_JSON"
    jq -e '.summary.unfiled_residual_risks > 0' "$OUT_JSON"

    printf -- '- Done verdict: residual quality risks remain after filing attempt.\n' > "$CHALLENGE"
    run bash "$SUMMARY_HELPER" \
        --done-challenge-file "$CHALLENGE" \
        --quality-audit-json "$OUT_JSON" \
        --output "$RUN_SUMMARY" \
        --sha abc123 \
        --branch main
    [ "$status" -eq 0 ]
    grep -qF -- '- Status: incomplete' "$RUN_SUMMARY"
    grep -qF -- '- Unfiled residual risks:' "$RUN_SUMMARY"
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
    jq -e '.findings[] | select(.probe=="verification-command" and .classification=="verification-contract-drift" and .dedupe_key=="verification-command:lint|path=package.json|title=npm-lint-script-fails")' "$OUT_JSON"
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
    jq -e '.issue_links | map(select(.dedupe_key=="maintainability-hotspot:src/app/feature/giant.service.ts|path=src/app/feature/giant.service.ts|title=ranked-maintainability-hotspot-1-src-app-feature-giant-service-ts")) | length == 1' "$OUT_JSON"
    jq -e '.issue_links | map(select(.dedupe_key=="any-usage:src/app/feature/giant.service.ts|path=src/app/feature/giant.service.ts|title=typescript-any-usage" or .dedupe_key=="debug-logging-hotspots:src/app/feature/giant.service.ts|path=src/app/feature/giant.service.ts|title=debug-logging-hotspot" or .dedupe_key=="eslint-disable:src/app/feature/giant.service.ts|path=src/app/feature/giant.service.ts|title=eslint-disable-usage" or .dedupe_key=="ts-ignore:src/app/feature/giant.service.ts|path=src/app/feature/giant.service.ts|title=typescript-suppression-usage")) | length == 0' "$OUT_JSON"
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
