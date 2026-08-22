#!/usr/bin/env bats
# tests/autospec-startup-self-update.bats — issue #3177.
#
# The startup self-update logic used to live as inlined bash inside
# templates/skill-blocks/startup-self-update.md, which the installer expands
# into every skill body (Claude SKILL.md, Codex prompts/<skill>.md, OpenCode
# agent/<skill>.md). A harness substitutes `$1` in a *rendered skill body* at
# load time, so `target="$1"` in that block became
# `target="<first argument to the slash command>"` — and
# heal_autonomous_operator_wrappers() then wrote a wrapper script over that
# path and chmod +x'd it, before the daily throttle ever ran.
#
# The fix moves the shell into scripts/autospec-startup-self-update.sh, which no
# harness ever renders. These tests pin that:
#   - no skill-block template may assign a positional parameter (regression),
#   - the expanded output carries no positional assignment either,
#   - the extracted script exists, is executable and parses,
#   - the opt-out and the daily throttle still short-circuit without network.
#
# bash 3.2 compatible: no `run` wrapped in helpers, real temp files only.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-startup-self-update.sh"
    TEMPLATE_DIR="$REPO_ROOT/templates/skill-blocks"
    TMP="$(mktemp -d)"
    SHIMDIR="$TMP/shim"
    mkdir -p "$SHIMDIR"
    SANDBOX_HOME="$TMP/home"
    mkdir -p "$SANDBOX_HOME/.autospec"
}

teardown() {
    rm -rf "$TMP"
}

# ---------------------------------------------------------------------------
# Regression: the defect itself
# ---------------------------------------------------------------------------

@test "no skill-block template assigns a positional parameter" {
    # A harness substitutes $1 inside a rendered skill body, so any
    # foo="$1" / foo="$2" in an injected template becomes attacker- (or
    # accident-) controlled data with the caller's argument.
    offenders="$TMP/offenders.txt"
    grep -rn '="\$[12]"' "$TEMPLATE_DIR" > "$offenders" 2>/dev/null || true
    run cat "$offenders"
    [ "$status" -eq 0 ]
    [ ! -s "$offenders" ]
}

@test "no rendered skill body assigns a positional parameter" {
    # Issue #3101: the scan above covers templates/skill-blocks/ ONLY, so a
    # positional assignment written directly into a trio body (SKILL.md,
    # codex/prompt.md, opencode/agent.md) or into a reference a body pulls in
    # was never seen. Those files are rendered by the same harness, so the same
    # substitution applies to them.
    scan="$TMP/scan-rendered-positionals.sh"
    cat > "$scan" <<'EOS'
cd "$1" || exit 2
grep -rn '="\$[0-9]"' skills/*/SKILL.md skills/*/codex/*.md \
    skills/*/opencode/*.md skills/*/references/*.md
EOS
    run bash "$scan" "$REPO_ROOT"
    # grep exits 1 when nothing matched, and that is the only passing outcome:
    # 0 means offenders, 2 means the scan itself broke.
    [ "$status" -eq 1 ] || { echo "rendered-body positional assignments:"; echo "$output"; return 1; }
}

@test "expanded skill-block output carries no positional-parameter assignment" {
    synth="$TMP/synth.md"
    printf '<!-- autospec-block:startup-self-update SKILL_NAME=autospec-run -->\n' > "$synth"
    expanded="$TMP/expanded.md"
    run bash "$REPO_ROOT/scripts/expand-skill-blocks.sh" "$synth"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" > "$expanded"
    # The placeholder still resolves — the block is not silently emptied.
    grep -q 'SKILL_NAME=autospec-run' "$expanded"
    ! grep -q '="\$1"' "$expanded"
    ! grep -q '="\$2"' "$expanded"
}

# ---------------------------------------------------------------------------
# The extracted script
# ---------------------------------------------------------------------------

@test "extracted self-update script exists, is executable and parses" {
    [ -f "$SCRIPT" ]
    [ -x "$SCRIPT" ]
    run bash -n "$SCRIPT"
    [ "$status" -eq 0 ]
}

@test "template invokes the extracted script instead of inlining its body" {
    template="$TEMPLATE_DIR/startup-self-update.md"
    [ -f "$template" ]
    grep -q 'autospec-startup-self-update.sh' "$template"
    # Three-way resolution fallback is preserved.
    grep -q 'SCRIPT_DIR' "$template"
    grep -q 'AUTOSPEC_SCRIPTS_DIR:-\$HOME/.autospec/scripts' "$template"
    # The wrapper-healing body no longer lives in the markdown.
    ! grep -q 'heal_autonomous_operator_wrappers()' "$template"
}

@test "extracted script ships with the installer's top-level scripts glob" {
    # install.sh copy_repo_scripts() globs $REPO_ROOT/scripts/*.sh, so a
    # top-level .sh lands in $AUTOSPEC_SCRIPTS_DIR on every install/--update.
    # A runtime file placed under scripts/lib/ would silently NOT ship.
    case "$SCRIPT" in
        "$REPO_ROOT"/scripts/*.sh) : ;;
        *) false ;;
    esac
    dirname_out="$(dirname "$SCRIPT")"
    [ "$dirname_out" = "$REPO_ROOT/scripts" ]
    grep -q 'copy_repo_scripts' "$REPO_ROOT/install.sh"
}

# ---------------------------------------------------------------------------
# Behavior parity: opt-out and throttle short-circuit with no network
# ---------------------------------------------------------------------------

@test "AUTOSPEC_NO_SELF_UPDATE=1 exits 0 immediately without touching the network" {
    printf '#!/usr/bin/env bash\necho "UNEXPECTED curl call" >&2\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    run env HOME="$SANDBOX_HOME" AUTOSPEC_NO_SELF_UPDATE=1 \
        PATH="$SHIMDIR:$PATH" bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    [[ "$output" != *"UNEXPECTED"* ]]
    [[ "$output" != *"WARN:"* ]]
    [ ! -e "$SANDBOX_HOME/.autospec/last-update-check" ]
}

@test "fresh last-update-check short-circuits the daily throttle without network" {
    fresh="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    printf '%s\n' "$fresh" > "$SANDBOX_HOME/.autospec/last-update-check"
    printf '#!/usr/bin/env bash\necho "UNEXPECTED curl call" >&2\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    [[ "$output" != *"UNEXPECTED"* ]]
    [ "$(cat "$SANDBOX_HOME/.autospec/last-update-check")" = "$fresh" ]
    # The throttle must not have taken the update lock.
    [ ! -d "$SANDBOX_HOME/.autospec/.update.lock.d" ]
}

@test "a filesystem path passed as the skill argument is never written to" {
    # The defect's payload: an argument that names a real file must not be
    # overwritten with a wrapper script. The script takes the skill name as $1
    # and must treat it as an inert label.
    victim="$TMP/victim.md"
    printf 'original spec content\n' > "$victim"
    printf '#!/usr/bin/env bash\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    mkdir -p "$SANDBOX_HOME/.autospec/bin"
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" "$victim"
    [ "$status" -eq 0 ]
    [ "$(cat "$victim")" = "original spec content" ]
    [ ! -x "$victim" ]
}
