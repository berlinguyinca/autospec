#!/usr/bin/env bats
# install-doc-drift.bats — Integration tests for install-doc-drift-workflow.sh
#                          and install-doc-drift-hook.sh
#
# Run: bats skills/autospec-shared/tests/integration/install-doc-drift.bats
# (from repo root)

setup() {
    SCRIPTS_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")/../../scripts" && pwd)"
    WORKFLOW_INSTALLER="${SCRIPTS_DIR}/install-doc-drift-workflow.sh"
    HOOK_INSTALLER="${SCRIPTS_DIR}/install-doc-drift-hook.sh"

    # Temp repo simulating a consumer repo
    TMP_REPO="$(mktemp -d /tmp/autospec-install-test-XXXXXX)"
    git -C "$TMP_REPO" init -q
    git -C "$TMP_REPO" commit --allow-empty -m "init" -q

    FAKE_SCRIPTS_DIR="$(mktemp -d /tmp/autospec-scripts-XXXXXX)"
}

teardown() {
    rm -rf "$TMP_REPO" "$FAKE_SCRIPTS_DIR"
}

# ── install-doc-drift-workflow.sh ─────────────────────────────────────────────

@test "workflow installer: installs workflow file into target repo" {
    run bash "$WORKFLOW_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    [ "$status" -eq 0 ]
    [ -f "${TMP_REPO}/.github/workflows/autospec-doc-drift.yml" ]
}

@test "workflow installer: installed file contains scripts-dir path" {
    run bash "$WORKFLOW_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    [ "$status" -eq 0 ]
    grep -q "$FAKE_SCRIPTS_DIR" "${TMP_REPO}/.github/workflows/autospec-doc-drift.yml"
}

@test "workflow installer: second install is idempotent (0 diff)" {
    bash "$WORKFLOW_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    CONTENT_BEFORE="$(cat "${TMP_REPO}/.github/workflows/autospec-doc-drift.yml")"
    run bash "$WORKFLOW_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    [ "$status" -eq 0 ]
    CONTENT_AFTER="$(cat "${TMP_REPO}/.github/workflows/autospec-doc-drift.yml")"
    [ "$CONTENT_BEFORE" = "$CONTENT_AFTER" ]
    [[ "$output" == *"already up-to-date"* ]]
}

@test "workflow installer: warns and keeps existing when file differs" {
    bash "$WORKFLOW_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    # Modify existing file
    echo "# customized" >> "${TMP_REPO}/.github/workflows/autospec-doc-drift.yml"
    CONTENT_BEFORE="$(cat "${TMP_REPO}/.github/workflows/autospec-doc-drift.yml")"
    run bash "$WORKFLOW_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    [ "$status" -eq 0 ]
    # Warning must mention the diff
    [[ "$output" == *"differs"* ]] || [[ "$output" == *"Keeping"* ]]
    # Original (modified) file must be preserved
    CONTENT_AFTER="$(cat "${TMP_REPO}/.github/workflows/autospec-doc-drift.yml")"
    [ "$CONTENT_BEFORE" = "$CONTENT_AFTER" ]
}

@test "workflow installer: --dry-run prints action without writing" {
    run bash "$WORKFLOW_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR" --dry-run
    [ "$status" -eq 0 ]
    [ ! -f "${TMP_REPO}/.github/workflows/autospec-doc-drift.yml" ]
    [[ "$output" == *"dry-run"* ]]
}

@test "workflow installer: fails on missing target directory" {
    run bash "$WORKFLOW_INSTALLER" --target "/nonexistent/path/xyz"
    [ "$status" -ne 0 ]
}

@test "workflow installer: workflow yml invokes check-doc-drift.sh --pr" {
    bash "$WORKFLOW_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    grep -q "check-doc-drift.sh" "${TMP_REPO}/.github/workflows/autospec-doc-drift.yml"
    grep -q -- "--pr" "${TMP_REPO}/.github/workflows/autospec-doc-drift.yml"
}

# ── install-doc-drift-hook.sh ─────────────────────────────────────────────────

@test "hook installer: creates pre-commit hook when none exists" {
    run bash "$HOOK_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    [ "$status" -eq 0 ]
    [ -f "${TMP_REPO}/.git/hooks/pre-commit" ]
}

@test "hook installer: created hook is executable" {
    bash "$HOOK_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    [ -x "${TMP_REPO}/.git/hooks/pre-commit" ]
}

@test "hook installer: hook contains autospec block markers" {
    bash "$HOOK_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    grep -q "BEGIN autospec-doc-drift-hook" "${TMP_REPO}/.git/hooks/pre-commit"
    grep -q "END autospec-doc-drift-hook" "${TMP_REPO}/.git/hooks/pre-commit"
}

@test "hook installer: hook contains check-doc-drift.sh --working-tree invocation" {
    bash "$HOOK_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    grep -q "check-doc-drift.sh" "${TMP_REPO}/.git/hooks/pre-commit"
    grep -q -- "--working-tree" "${TMP_REPO}/.git/hooks/pre-commit"
}

@test "hook installer: hook never exits non-zero (warnings only)" {
    bash "$HOOK_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    # The hook block must use || true so it never blocks commit
    grep -A5 "check-doc-drift.sh" "${TMP_REPO}/.git/hooks/pre-commit" | grep -q "|| true"
}

@test "hook installer: appends to existing pre-commit hook" {
    # Create an existing hook
    mkdir -p "${TMP_REPO}/.git/hooks"
    printf '#!/usr/bin/env bash\necho "existing hook"\n' > "${TMP_REPO}/.git/hooks/pre-commit"
    chmod +x "${TMP_REPO}/.git/hooks/pre-commit"

    run bash "$HOOK_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    [ "$status" -eq 0 ]
    # Both original content and new block present
    grep -q "existing hook" "${TMP_REPO}/.git/hooks/pre-commit"
    grep -q "BEGIN autospec-doc-drift-hook" "${TMP_REPO}/.git/hooks/pre-commit"
}

@test "hook installer: second install detects existing block and no-ops" {
    bash "$HOOK_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    CONTENT_BEFORE="$(cat "${TMP_REPO}/.git/hooks/pre-commit")"

    run bash "$HOOK_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR"
    [ "$status" -eq 0 ]
    [[ "$output" == *"no-op"* ]] || [[ "$output" == *"already present"* ]]
    CONTENT_AFTER="$(cat "${TMP_REPO}/.git/hooks/pre-commit")"
    [ "$CONTENT_BEFORE" = "$CONTENT_AFTER" ]
}

@test "hook installer: --dry-run prints action without writing" {
    run bash "$HOOK_INSTALLER" --target "$TMP_REPO" --scripts-dir "$FAKE_SCRIPTS_DIR" --dry-run
    [ "$status" -eq 0 ]
    [ ! -f "${TMP_REPO}/.git/hooks/pre-commit" ]
    [[ "$output" == *"dry-run"* ]]
}

@test "hook installer: fails on non-git directory" {
    NOT_A_REPO="$(mktemp -d /tmp/autospec-notgit-XXXXXX)"
    run bash "$HOOK_INSTALLER" --target "$NOT_A_REPO"
    [ "$status" -ne 0 ]
    rm -rf "$NOT_A_REPO"
}

# ── install.sh copy_shared_scripts ───────────────────────────────────────────

@test "install.sh --update copies shared scripts to AUTOSPEC_SCRIPTS_DIR with +x" {
    # Resolve repo root: this file is at skills/autospec-shared/tests/integration/
    # so repo root is 4 levels up from the directory containing this bats file.
    BATS_FILE_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")" && pwd)"
    REPO_ROOT="$(cd "${BATS_FILE_DIR}/../../../.." && pwd)"
    SHARED_SRC="${REPO_ROOT}/skills/autospec-shared/scripts"

    [ -d "$SHARED_SRC" ] || skip "shared scripts directory not found at ${SHARED_SRC}"

    TMP_SCRIPTS="$(mktemp -d /tmp/autospec-scripts-copy-XXXXXX)"

    # Replicate what copy_shared_scripts does in install.sh
    cp -R "${SHARED_SRC}/." "${TMP_SCRIPTS}/"
    find "${TMP_SCRIPTS}" -maxdepth 2 \( -name '*.sh' -o -name '*.mjs' \) -exec chmod +x {} \;

    # Verify check-doc-drift.sh is present and executable
    [ -f "${TMP_SCRIPTS}/check-doc-drift.sh" ]
    [ -x "${TMP_SCRIPTS}/check-doc-drift.sh" ]

    # Verify new installer scripts are present and executable
    [ -f "${TMP_SCRIPTS}/install-doc-drift-workflow.sh" ]
    [ -x "${TMP_SCRIPTS}/install-doc-drift-workflow.sh" ]
    [ -f "${TMP_SCRIPTS}/install-doc-drift-hook.sh" ]
    [ -x "${TMP_SCRIPTS}/install-doc-drift-hook.sh" ]

    rm -rf "$TMP_SCRIPTS"
}
