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

# ---------------------------------------------------------------------------
# Stale lock reclamation (issue #3937)
#
# A lock directory is a mkdir marker with an owner file
# (<pid> <epoch> <iso>). A lock whose owner is dead — or which has no owner —
# is stale and must be reclaimed, not mistaken for a concurrent run.
# ---------------------------------------------------------------------------

# Write a URL-aware curl shim: the API call returns a SHA, the bootstrap
# download returns a no-op installer script. Everything else fails loudly.
_install_curl_shim() {
    cat > "$SHIMDIR/curl" <<'CURLSHIM'
#!/usr/bin/env bash
for a in "$@"; do
    case "$a" in
        *api.github.com*) printf '{"sha":"newsha1"}\n'; exit 0 ;;
        *bootstrap.sh*) printf '#!/usr/bin/env bash\nexit 0\n'; exit 0 ;;
    esac
done
echo "UNEXPECTED curl call" >&2
exit 1
CURLSHIM
    chmod +x "$SHIMDIR/curl"
}

# A PID that is guaranteed to be dead: spawn, reap, reuse its number.
_dead_pid() {
    sleep 0.01 &
    local p=$!
    wait "$p"
    printf '%s\n' "$p"
}

@test "stale lock with dead owner is reclaimed and the update proceeds" {
    # The populated case (#3793): a lock dir that carries an owner file, but
    # the owner process is gone.
    local lock="$SANDBOX_HOME/.autospec/.update.lock.d"
    mkdir -p "$lock"
    local dead
    dead="$(_dead_pid)"
    if kill -0 "$dead" 2>/dev/null; then
        fail "test setup: PID $dead unexpectedly alive"
    fi
    printf '%s %s %s\n' "$dead" "$(date -u +%s)" "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" > "$lock/owner"
    # Backdate past the 1800s stale threshold (portable to BSD + GNU touch).
    touch -t 2001010000 "$lock"
    echo "oldsha1" > "$SANDBOX_HOME/.autospec/installed-version"
    date -u -v-25H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '25 hours ago' +'%Y-%m-%dT%H:%M:%SZ' \
        > "$SANDBOX_HOME/.autospec/last-update-check"
    _install_curl_shim
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "reclaiming stale update lock"
    echo "$output" | grep -q "not running"
    echo "$output" | grep -q "\[autospec\] updated oldsha1"
    [ "$(cat "$SANDBOX_HOME/.autospec/installed-version")" = "newsha1" ]
    # The reclaimed lock is released again on exit.
    [ ! -e "$lock" ]
}

@test "stale ownerless lock is reclaimed and the update proceeds" {
    # The empty case: a bare mkdir marker left by a crashed run, backdated.
    local lock="$SANDBOX_HOME/.autospec/.update.lock.d"
    mkdir -p "$lock"
    touch -t 2001010000 "$lock"
    echo "oldsha1" > "$SANDBOX_HOME/.autospec/installed-version"
    date -u -v-25H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '25 hours ago' +'%Y-%m-%dT%H:%M:%SZ' \
        > "$SANDBOX_HOME/.autospec/last-update-check"
    _install_curl_shim
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "reclaiming stale update lock"
    echo "$output" | grep -q "no live owner found"
    [ "$(cat "$SANDBOX_HOME/.autospec/installed-version")" = "newsha1" ]
    [ ! -e "$lock" ]
}

@test "fresh lock with no owner is not reclaimed (write-grace window)" {
    local lock="$SANDBOX_HOME/.autospec/.update.lock.d"
    mkdir -p "$lock"
    printf '#!/usr/bin/env bash\necho "UNEXPECTED curl call" >&2\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "write-grace window"
    [ -d "$lock" ]
    [[ "$output" != *"UNEXPECTED"* ]]
}

@test "fresh lock with a live owner is not reclaimed (verified concurrent run)" {
    local lock="$SANDBOX_HOME/.autospec/.update.lock.d"
    mkdir -p "$lock"
    printf '%s %s %s\n' "$$" "$(date -u +%s)" "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" > "$lock/owner"
    printf '#!/usr/bin/env bash\necho "UNEXPECTED curl call" >&2\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "concurrent update in progress"
    [ -d "$lock" ]
    [ -f "$lock/owner" ]
    [[ "$output" != *"UNEXPECTED"* ]]
}

# ---------------------------------------------------------------------------
# Doctor mode: out-of-band health check (issue #3937)
# ---------------------------------------------------------------------------

@test "--doctor on a healthy state exits 0 and reports healthy" {
    date -u +'%Y-%m-%dT%H:%M:%SZ' > "$SANDBOX_HOME/.autospec/last-update-check"
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" --doctor
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "doctor: healthy"
    echo "$output" | grep -q "throttle stamp:"
    echo "$output" | grep -q "last failure:     none"
}

@test "--doctor flags a stale lock and exits 1" {
    local lock="$SANDBOX_HOME/.autospec/.update.lock.d"
    mkdir -p "$lock"
    touch -t 2001010000 "$lock"
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" --doctor
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "lock:.*STALE"
    echo "$output" | grep -q "finding(s)"
    # Doctor is read-only without --clear-stale-lock.
    [ -d "$lock" ]
}

@test "--doctor --clear-stale-lock clears a stale lock and exits 0" {
    local lock="$SANDBOX_HOME/.autospec/.update.lock.d"
    mkdir -p "$lock"
    touch -t 2001010000 "$lock"
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" --doctor --clear-stale-lock
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "lock:.*cleared"
    echo "$output" | grep -q "doctor: healthy"
    [ ! -e "$lock" ]
}

@test "--doctor flags a throttle stamp older than 3x the interval" {
    date -u -v-4d +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '4 days ago' +'%Y-%m-%dT%H:%M:%SZ' \
        > "$SANDBOX_HOME/.autospec/last-update-check"
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" --doctor
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "throttle stamp:.*STALE"
}

@test "--doctor flags installed-vs-remote drift" {
    date -u +'%Y-%m-%dT%H:%M:%SZ' > "$SANDBOX_HOME/.autospec/last-update-check"
    echo "v1" > "$SANDBOX_HOME/.autospec/installed-version"
    echo "v2" > "$SANDBOX_HOME/.autospec/remote-version"
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" --doctor
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "version drift:.*installed v1 vs remote v2"
}

@test "--doctor reports the last failure record" {
    date -u +'%Y-%m-%dT%H:%M:%SZ' > "$SANDBOX_HOME/.autospec/last-update-check"
    printf '{"timestamp":"2026-01-01T00:00:00Z","installer_exit_code":1}\n' \
        > "$SANDBOX_HOME/.autospec/last-update-failure.json"
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" --doctor
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "last failure:.*2026-01-01T00:00:00Z"
    echo "$output" | grep -q "installer exit 1"
}

@test "--doctor bypasses AUTOSPEC_NO_SELF_UPDATE (operator-invoked repair)" {
    date -u +'%Y-%m-%dT%H:%M:%SZ' > "$SANDBOX_HOME/.autospec/last-update-check"
    run env HOME="$SANDBOX_HOME" AUTOSPEC_NO_SELF_UPDATE=1 \
        PATH="$SHIMDIR:$PATH" bash "$SCRIPT" --doctor
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "doctor: healthy"
}

@test "preflight emits the stale-throttle alarm even when the update then succeeds" {
    # A 4-day-old stamp must fail loudly (invariant 4) regardless of the
    # update outcome; the update still runs and heals the stamp.
    date -u -v-4d +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '4 days ago' +'%Y-%m-%dT%H:%M:%SZ' \
        > "$SANDBOX_HOME/.autospec/last-update-check"
    echo "oldsha1" > "$SANDBOX_HOME/.autospec/installed-version"
    _install_curl_shim
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "self-update has not completed"
    echo "$output" | grep -q -- "--doctor"
    [ "$(cat "$SANDBOX_HOME/.autospec/installed-version")" = "newsha1" ]
    # The healed stamp is fresh again.
    local fresh
    fresh="$(date -u -v-1H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date -u -d '1 hour ago' +'%Y-%m-%dT%H:%M:%SZ')"
    [ "$(cat "$SANDBOX_HOME/.autospec/last-update-check")" \> "$fresh" ]
}
