#!/usr/bin/env bats
# tests/validate-shared-script-install.bats — bats tests for scripts/validate.sh
# check_shared_script_install() dynamic + conditional behavior (issue #1278).
#
# The old check iterated a hardcoded ~14-skill list against a fixed 8-helper set,
# so new per-skill installers silently escaped the shared-helper-install
# assertion. The new check is DYNAMIC over skills/*/install.sh and CONDITIONAL:
# a skill is only required to install into ~/.autospec/scripts when it actually
# references a SHARED ${AUTOSPEC_SCRIPTS_DIR}/<helper> runtime helper (one that
# lives in repo-root scripts/ or skills/autospec-shared/scripts/ and is NOT one
# of the skill's own scripts/<helper>).
#
# These tests source the REAL functions out of validate.sh (so they stay in
# lockstep with the source) and run them against a synthetic skills/ tree.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/validate.sh"

# Extract the two functions under test from the live validate.sh into a sourceable
# snippet, with minimal fail/info stubs. We extract by function name with awk so
# edits to the source flow straight into the test.
extract_funcs() {
    awk '
        $0 ~ /^referenced_shared_helpers\(\) \{/ {grab=1}
        $0 ~ /^check_shared_script_install\(\) \{/ {grab=1}
        grab {print}
        grab && /^\}$/ {grab=0}
    ' "$SCRIPT"
}

# Run check_shared_script_install against a synthetic skills/ tree rooted at $1.
# Prints output, returns the check exit status.
run_check_in_tree() {
    local tree="$1"
    (
        cd "$tree"
        bash -c "
            set -u
            fail() { printf 'validate: FAIL — %s\n' \"\$*\" >&2; exit 1; }
            info() { printf 'validate: %s\n' \"\$*\"; }
            $(extract_funcs)
            check_shared_script_install
        "
    )
}

# Build a synthetic repo tree: repo-root scripts/<shared-helper>, an
# autospec-shared/scripts/<lib-helper>, and per-skill dirs created by the caller.
new_tree() {
    local t
    t="$(mktemp -d)"
    mkdir -p "$t/scripts" "$t/skills/autospec-shared/scripts"
    # A shared repo-root helper and a shared-lib helper.
    printf '#!/bin/sh\n' > "$t/scripts/shared-helper.sh"
    printf '#!/bin/sh\n' > "$t/skills/autospec-shared/scripts/lib-helper.sh"
    printf '%s' "$t"
}

# Add a skill that references a SHARED helper but whose install.sh does NOT
# install into ~/.autospec/scripts (the real-gap shape, e.g. pre-fix review).
add_skill_needs_no_install() {
    local tree="$1" name="$2"
    mkdir -p "$tree/skills/$name"
    cat > "$tree/skills/$name/SKILL.md" <<EOF
---
name: $name
---
Runtime call:
\`bash "\${AUTOSPEC_SCRIPTS_DIR:-\$HOME/.autospec/scripts}/shared-helper.sh" --go\`
EOF
    cat > "$tree/skills/$name/install.sh" <<'EOF'
#!/usr/bin/env sh
# installs nothing into the shared runtime dir
echo "installed skill files only"
EOF
}

# Add a skill that references a SHARED helper AND installs into ~/.autospec/scripts.
add_skill_needs_with_install() {
    local tree="$1" name="$2"
    mkdir -p "$tree/skills/$name"
    cat > "$tree/skills/$name/SKILL.md" <<EOF
---
name: $name
---
Runtime call:
\`bash "\${AUTOSPEC_SCRIPTS_DIR:-\$HOME/.autospec/scripts}/lib-helper.sh" --go\`
EOF
    cat > "$tree/skills/$name/install.sh" <<'EOF'
#!/usr/bin/env sh
install_shared_scripts() {
    cp "$SRC/lib-helper.sh" "$HOME/.autospec/scripts/lib-helper.sh"
}
EOF
}

# Add a skill that references NO shared helper (exempt). It only references its
# OWN scripts/<helper>, so it must NOT be required to install the shared dir.
add_skill_exempt() {
    local tree="$1" name="$2"
    mkdir -p "$tree/skills/$name/scripts"
    printf '#!/bin/sh\n' > "$tree/skills/$name/scripts/own-helper.sh"
    cat > "$tree/skills/$name/SKILL.md" <<EOF
---
name: $name
---
Runtime call (own script):
\`bash "\${AUTOSPEC_SCRIPTS_DIR:-\$HOME/.autospec/scripts}/own-helper.sh" --go\`
EOF
    cat > "$tree/skills/$name/install.sh" <<'EOF'
#!/usr/bin/env sh
echo "no shared runtime dir install needed"
EOF
}

teardown() {
    [ -n "${TREE:-}" ] && rm -rf "$TREE"
    return 0
}

@test "needs-shared skill WITHOUT ~/.autospec/scripts install FAILS the check" {
    TREE="$(new_tree)"
    add_skill_needs_no_install "$TREE" "skill-gap"
    run run_check_in_tree "$TREE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"skill-gap"* ]]
    [[ "$output" == *".autospec/scripts"* ]]
}

@test "skill referencing NO shared helper is EXEMPT (passes)" {
    TREE="$(new_tree)"
    add_skill_exempt "$TREE" "skill-exempt"
    run run_check_in_tree "$TREE"
    [ "$status" -eq 0 ]
    [[ "$output" == *"exempt"* ]]
}

@test "needs-shared skill WITH ~/.autospec/scripts install passes" {
    TREE="$(new_tree)"
    add_skill_needs_with_install "$TREE" "skill-ok"
    run run_check_in_tree "$TREE"
    [ "$status" -eq 0 ]
}

@test "exempt + ok pass together while the gap skill still fails (mixed tree)" {
    TREE="$(new_tree)"
    add_skill_exempt "$TREE" "skill-exempt"
    add_skill_needs_with_install "$TREE" "skill-ok"
    add_skill_needs_no_install "$TREE" "skill-gap"
    run run_check_in_tree "$TREE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"skill-gap"* ]]
}

@test "referenced_shared_helpers skips the skill's OWN scripts/<helper>" {
    TREE="$(new_tree)"
    # Skill references BOTH its own helper and a shared helper; only the shared
    # one should make it needs-shared. Remove the install -> must fail naming the
    # shared helper, not the own one.
    mkdir -p "$TREE/skills/skill-mixed/scripts"
    printf '#!/bin/sh\n' > "$TREE/skills/skill-mixed/scripts/own-helper.sh"
    cat > "$TREE/skills/skill-mixed/SKILL.md" <<'EOF'
---
name: skill-mixed
---
Own: `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/own-helper.sh"`
Shared: `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/shared-helper.sh"`
EOF
    printf '#!/usr/bin/env sh\necho noop\n' > "$TREE/skills/skill-mixed/install.sh"
    run run_check_in_tree "$TREE"
    [ "$status" -ne 0 ]
    [[ "$output" == *"shared-helper.sh"* ]]
    [[ "$output" != *"own-helper.sh"* ]]
}
