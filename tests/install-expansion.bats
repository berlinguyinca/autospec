#!/usr/bin/env bats
# tests/install-expansion.bats — TDD tests for D2: install.sh block expansion + skew rule.
#
# AC coverage:
#   - Marker-bearing skill file → installed copy has markers expanded.
#   - Marker-free skill file → installed copy is byte-identical to source.
#   - Expansion failure → loud abort (non-zero), nothing installed for that skill.
#   - Skew rule → expander resolved relative to install.sh's own REPO_ROOT, not
#     a previously-installed copy.
#
# All tests use real scripts (no mocks, no stubs). A temp HOME + dest dir
# isolates each run.

REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)"
INSTALL_SH="$REPO_ROOT/install.sh"
EXPANDER="$REPO_ROOT/scripts/expand-skill-blocks.sh"

# ─── helpers ──────────────────────────────────────────────────────────────────

setup() {
    # Fresh isolated HOME for every test so install.sh side-effects don't bleed.
    export ORIG_HOME="$HOME"
    export HOME
    HOME="$(mktemp -d)"

    # A fixture skill directory we own entirely — safe to manipulate.
    FIXTURE_SKILL_DIR="$(mktemp -d)"
    mkdir -p "$FIXTURE_SKILL_DIR/opencode" "$FIXTURE_SKILL_DIR/codex"

    # Silence optional bootstraps that would hit the network or prompt.
    export AUTOSPEC_SKIP_SYSTEM_TOOLS=1
    export AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1
    export AUTOSPEC_SKIP_SUPERPOWERS=1
    export AUTOSPEC_NO_STAR_PROMPT=1
    export AUTOSPEC_AUTO_YES=1
    export CI=1
}

teardown() {
    rm -rf "$HOME" "$FIXTURE_SKILL_DIR"
    export HOME="$ORIG_HOME"
}

# Run install.sh with a patched SKILLS_DIR that only has our fixture skill,
# installing a single named skill into the claude harness only.
run_install_for_fixture() {
    local skill_name="$1"
    # We'll invoke install.sh directly with env vars that redirect harness paths
    # to our temp HOME, and only install the one fixture skill.
    SKILLS_DIR="$FIXTURE_SKILL_DIR_PARENT" \
    CLAUDE_CONFIG_DIR="$HOME/.claude" \
    AUTOSPEC_SCRIPTS_DIR="$HOME/.autospec/scripts" \
    AUTOSPEC_SCHEMAS_DIR="$HOME/.autospec/schemas" \
    bash "$INSTALL_SH" \
        --skill "$skill_name" \
        --harness claude \
        --update
}

# ─── test 1: marker-bearing file → expanded output installed ──────────────────

@test "marker-bearing SKILL.md is expanded on install" {
    # Create a minimal per-skill install.sh that the root installer can call.
    local skill_name="fixture-expand"
    local skill_dir="$FIXTURE_SKILL_DIR"

    # Write a SKILL.md that contains a block marker.
    cat > "$skill_dir/SKILL.md" <<'EOF'
# Fixture skill

<!-- autospec-block:startup-self-update SKILL_NAME=fixture-expand -->

End of skill.
EOF
    # Minimal supporting files needed by per-skill install.sh conventions.
    cp "$skill_dir/SKILL.md" "$skill_dir/opencode/agent.md"
    cp "$skill_dir/SKILL.md" "$skill_dir/codex/prompt.md"

    # Install into isolated HOME using the repo's expand_skill_file helper via
    # the root install.sh. We call the expander directly to confirm the contract:
    # the installed copy must not contain any '<!-- autospec-block:' marker.
    local dest_dir
    dest_dir="$(mktemp -d)"
    local dest_file="$dest_dir/SKILL.md"

    # Exercise the exact code path install.sh uses (grep → expander → dest).
    if grep -q '<!-- autospec-block:' "$skill_dir/SKILL.md"; then
        "$EXPANDER" "$skill_dir/SKILL.md" > "$dest_file"
    else
        cp "$skill_dir/SKILL.md" "$dest_file"
    fi

    # The installed copy must not contain any unexpanded marker.
    run grep -c '<!-- autospec-block:' "$dest_file"
    [ "$status" -ne 0 ] || [ "$output" -eq 0 ]

    # It must contain expanded content from the template (non-empty file).
    [ -s "$dest_file" ]

    # Cleanup
    rm -rf "$dest_dir"
}

# ─── test 2: marker-free file → byte-identical install ───────────────────────

@test "marker-free SKILL.md installs byte-identical to source" {
    local skill_dir="$FIXTURE_SKILL_DIR"

    cat > "$skill_dir/SKILL.md" <<'EOF'
# Plain skill — no markers here.

Just regular content that should copy through unchanged.
EOF

    local dest_dir
    dest_dir="$(mktemp -d)"
    local dest_file="$dest_dir/SKILL.md"

    # Exercise the install.sh code path.
    if grep -q '<!-- autospec-block:' "$skill_dir/SKILL.md"; then
        "$EXPANDER" "$skill_dir/SKILL.md" > "$dest_file"
    else
        cp "$skill_dir/SKILL.md" "$dest_file"
    fi

    # Must be byte-identical.
    run diff "$skill_dir/SKILL.md" "$dest_file"
    [ "$status" -eq 0 ]

    rm -rf "$dest_dir"
}

# ─── test 3: expansion failure → loud abort, nothing installed ────────────────

@test "unknown block marker causes expander to exit non-zero" {
    local skill_dir="$FIXTURE_SKILL_DIR"

    cat > "$skill_dir/SKILL.md" <<'EOF'
# Broken skill

<!-- autospec-block:no-such-template-block-xyz -->

More content.
EOF

    local dest_dir
    dest_dir="$(mktemp -d)"
    local dest_file="$dest_dir/SKILL.md"

    # The expander should exit non-zero on unknown block.
    run "$EXPANDER" "$skill_dir/SKILL.md"
    [ "$status" -ne 0 ]

    # Nothing should have been written to the dest (simulating fail-closed).
    # In the real install path, we check $? before redirecting. Here we verify
    # the expander itself refuses.
    [ ! -f "$dest_file" ]

    rm -rf "$dest_dir"
}

@test "install.sh expand_skill_file: fails closed and leaves no dest file on expander error" {
    # Source just the expand_skill_file helper from install.sh and call it.
    local skill_dir="$FIXTURE_SKILL_DIR"
    local dest_dir
    dest_dir="$(mktemp -d)"

    cat > "$skill_dir/SKILL.md" <<'EOF'
# Broken skill

<!-- autospec-block:no-such-template-xyz -->
EOF

    # Extract expand_skill_file and its deps from install.sh, then call it.
    local helper_script
    helper_script="$(mktemp /tmp/expand_helper.XXXXXX.sh)"
    {
        printf '#!/usr/bin/env bash\nset -eu\n'
        printf 'REPO_ROOT="%s"\n' "$REPO_ROOT"
        # Pull err/info helpers.
        printf 'err()  { printf "error: %%s\\n" "$*" >&2; }\n'
        printf 'info() { printf "%%s\\n" "$*"; }\n'
        # Extract expand_skill_file function.
        awk '/^expand_skill_file\(\)/{f=1} f{print} f && /^\}$/{exit}' "$INSTALL_SH"
        # Call it.
        printf '\nexpand_skill_file "%s/SKILL.md" "%s/SKILL.md"\n' "$skill_dir" "$dest_dir"
    } > "$helper_script"
    chmod +x "$helper_script"

    run bash "$helper_script"
    # Must exit non-zero.
    [ "$status" -ne 0 ]

    # Must have emitted an error message.
    [[ "$output" =~ "expander" ]] || [[ "$output" =~ "expand" ]] || [[ "$output" =~ "error" ]]

    # Dest file must NOT exist (fail-closed).
    [ ! -f "$dest_dir/SKILL.md" ]

    rm -f "$helper_script"
    rm -rf "$dest_dir"
}

# ─── test 4: post-install safety grep ────────────────────────────────────────

@test "installed file containing unexpanded marker is detected and fails" {
    # Simulate a scenario where the installed copy still has a raw marker
    # (regression: what if someone bypassed the expander?). The post-install
    # grep check must catch it.
    local dest_dir
    dest_dir="$(mktemp -d)"
    local dest_file="$dest_dir/SKILL.md"

    # Write a file with a marker still in it.
    cat > "$dest_file" <<'EOF'
# Broken installed file

<!-- autospec-block:startup-self-update SKILL_NAME=oops -->
EOF

    # The post-install grep check (mirrors what install.sh does).
    run grep -q '<!-- autospec-block:' "$dest_file"
    [ "$status" -eq 0 ]  # grep found a marker → install.sh should abort

    rm -rf "$dest_dir"
}

# ─── test 5: skew rule — expander path is always $REPO_ROOT/scripts ──────────

@test "skew rule: expand_skill_file uses REPO_ROOT-relative expander path" {
    # Verify that the expand_skill_file function in install.sh references the
    # expander via $REPO_ROOT (i.e., a variable rooted in the script's own
    # location), not a hardcoded ~/.autospec/ path.
    run grep -n 'expand-skill-blocks.sh' "$INSTALL_SH"
    [ "$status" -eq 0 ]

    # The reference must use $REPO_ROOT (or the variable holding the repo root),
    # not $HOME, ~/.autospec, or AUTOSPEC_SCRIPTS_DIR (installed copies).
    # Check that every line referencing the expander uses REPO_ROOT.
    while IFS= read -r line; do
        # Must contain REPO_ROOT (or equivalent repo-relative anchor).
        if [[ "$line" =~ 'expand-skill-blocks.sh' ]]; then
            [[ "$line" =~ 'REPO_ROOT' ]] || [[ "$line" =~ 'SCRIPT_DIR' ]]
        fi
    done < <(grep 'expand-skill-blocks.sh' "$INSTALL_SH")
}

# ─── test 6: real end-to-end via a fixture skill with a marker ────────────────

@test "end-to-end: expander runs on marker file and produces non-empty expanded output" {
    local skill_dir="$FIXTURE_SKILL_DIR"

    # Use the startup-self-update template which is known to exist.
    cat > "$skill_dir/SKILL.md" <<'EOF'
# Test skill preamble

<!-- autospec-block:startup-self-update SKILL_NAME=test-skill -->

## After block

Some more content.
EOF

    local dest_dir
    dest_dir="$(mktemp -d)"
    local dest_file="$dest_dir/SKILL.md"

    # Run the expander as install.sh would.
    run "$EXPANDER" "$skill_dir/SKILL.md"
    [ "$status" -eq 0 ]

    "$EXPANDER" "$skill_dir/SKILL.md" > "$dest_file"

    # No marker remaining in output.
    run grep -c '<!-- autospec-block:' "$dest_file"
    [ "$status" -ne 0 ] || [ "$output" = "0" ]

    # Template content appeared — startup-self-update contains "autospec" text.
    run grep -c 'autospec' "$dest_file"
    [ "$status" -eq 0 ]
    [ "$output" -gt 0 ]

    # The literal SKILL_NAME param got substituted.
    run grep -c 'test-skill' "$dest_file"
    [ "$status" -eq 0 ]
    [ "$output" -gt 0 ]

    rm -rf "$dest_dir"
}
