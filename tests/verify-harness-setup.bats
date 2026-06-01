#!/usr/bin/env bats
# tests/verify-harness-setup.bats — tests for skills/autospec-shared/scripts/verify-harness-setup.sh
# Covers: --check exits non-zero on missing CLAUDE.md, fix mode creates CLAUDE.md,
# fix mode is idempotent, AGENTS.md missing → non-zero exit.

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/verify-harness-setup.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    # macOS: mktemp returns /var/folders/... but realpath is /private/var/...
    # git rev-parse --show-toplevel returns the resolved path, so match it here
    # so the CC_SLUG computed inside the script lines up with what we stub.
    TEST_TMP="$(cd "$TEST_TMP" && pwd -P)"
    cd "$TEST_TMP"
    git init -q .
    git config user.email "t@t.t"
    git config user.name "t"
    mkdir -p docs/memory
    printf '# Memory index\n' > docs/memory/MEMORY.md

    # AGENTS.md with the inventory block (otherwise auto-init-memory.sh adds it)
    cat > AGENTS.md <<'EOF'
# AGENTS

## Memory inventory

See docs/memory/.
EOF

    # Stub the CC symlink so the symlink check passes without HOME pollution
    HOME_BAK="$HOME"
    export HOME="$TEST_TMP/home"
    SLUG=$(echo "$TEST_TMP" | tr '/' '-')
    mkdir -p "$HOME/.claude/projects/${SLUG}"
    ln -s "$TEST_TMP/docs/memory" "$HOME/.claude/projects/${SLUG}/memory"

    # Don't let auto-init-memory.sh mutate AGENTS.md (already correct)
    export AUTOSPEC_SKIP_MEMPALACE_INSTALL=1
}

teardown() {
    export HOME="$HOME_BAK"
    rm -rf "$TEST_TMP"
}

@test "check mode: passes when everything is wired up + CLAUDE.md exists" {
    printf '# Claude\n' > CLAUDE.md
    run bash "$SCRIPT" --check
    [ "$status" -eq 0 ]
    [[ "$output" == *"AGENTS.md has Memory inventory"* ]]
}

@test "check mode: exits 1 when CLAUDE.md is missing" {
    run bash "$SCRIPT" --check
    [ "$status" -eq 1 ]
    [[ "$output" == *"CLAUDE.md missing"* ]]
}

@test "fix mode: creates CLAUDE.md pointer when missing" {
    run bash "$SCRIPT"
    [ "$status" -eq 0 ]
    [ -f CLAUDE.md ]
    grep -q "AGENTS.md" CLAUDE.md
}

@test "fix mode: idempotent on already-configured repo" {
    bash "$SCRIPT" >/dev/null
    run bash "$SCRIPT" --check
    [ "$status" -eq 0 ]
}

@test "check mode: exits 1 when AGENTS.md is missing" {
    rm AGENTS.md
    run bash "$SCRIPT" --check
    [ "$status" -eq 1 ]
    [[ "$output" == *"AGENTS.md missing"* ]]
}
