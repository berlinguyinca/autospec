# Shared fixture for repo-quality-audit*.bats.

AUDIT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/repo-quality-audit.sh"
RUN_SKILL="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)/autospec-run/SKILL.md"
# The end-of-run tail moved out of autospec-run/SKILL.md in #3156. Assertions about that
# tail must follow it, or they force the extraction to be undone.
RUN_END_OF_RUN="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)/autospec-run/references/end-of-run.md"
SUMMARY_HELPER="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../../.." && pwd)/scripts/autospec-write-run-summary.sh"
PROJECT_SYNC_STUB="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../../.." && pwd)/tests/fixtures/autospec-project-sync-stub.sh"

setup_repo_quality_audit_fixture() {
    TEST_TMP="$(mktemp -d)"
    REPO="$TEST_TMP/repo"
    mkdir -p "$REPO/src" "$REPO/tests" "$REPO/.autospec"
    export AUTOSPEC_BIN="$PROJECT_SYNC_STUB"
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
    printf '{"accepted_debt":["debug-logging-hotspots:src/app.js"]}
'         > "$REPO/.autospec/quality-audit-accepted.json"
}

teardown_repo_quality_audit_fixture() {
    rm -rf "$TEST_TMP"
    unset AUTOSPEC_BIN
}
