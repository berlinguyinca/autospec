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

# Stop a live-owner stand-in (a `sleep` child) without masking a test failure:
# the kill itself may fail if the child already exited, which is not a defect.
_stop_stood_in_owner() {
    if kill -0 "$1" 2>/dev/null; then
        kill "$1" 2>/dev/null
    fi
    return 0
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
# Issue #3937 — lock liveness, honest diagnostics, staleness surfacing, doctor
# ---------------------------------------------------------------------------

# Shim curl: records that it was invoked, serves a new sha, and installs a
# no-op bootstrap so the run completes inside the sandbox.
_shim_curl_succeeding() {
    cat > "$SHIMDIR/curl" <<'SHIM'
#!/usr/bin/env bash
: > "${CURL_MARKER:?}/curl-invoked"
for arg in "$@"; do
    case "$arg" in
        *commits/main*) printf '{"sha":"newsha77"}\n'; exit 0 ;;
    esac
done
printf '#!/usr/bin/env bash\nexit 0\n'
SHIM
    chmod +x "$SHIMDIR/curl"
}

_shim_curl_failing() {
    printf '#!/usr/bin/env bash\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
}

@test "stale lock with a dead owner PID is reclaimed and the update proceeds" {
    lock="$SANDBOX_HOME/.autospec/.update.lock.d"
    mkdir -p "$lock"
    printf '%s\n' "999999" > "$lock/owner.pid"
    printf '%s\n' "$(date -u +%s)" > "$lock/owner.epoch"
    _shim_curl_succeeding
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" CURL_MARKER="$SANDBOX_HOME" \
        bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "reclaiming stale lock"
    [ -f "$SANDBOX_HOME/curl-invoked" ]
    # The script truncates the remote sha to 7 characters.
    [ "$(cat "$SANDBOX_HOME/.autospec/installed-version")" = "newsha7" ]
    [ ! -d "$lock" ]
}

@test "legacy lock with no owner record is reclaimed by age and says so" {
    lock="$SANDBOX_HOME/.autospec/.update.lock.d"
    mkdir -p "$lock"
    # An 18-day-old bare lock directory: exactly the #3937 incident.
    older="$(date -u -d '18 days ago' +'%Y%m%d%H%M' 2>/dev/null \
        || date -u -v-18d +'%Y%m%d%H%M')"
    touch -t "$older" "$lock"
    _shim_curl_failing
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "reclaiming stale lock"
    echo "$output" | grep -q "no owner recorded"
    echo "$output" | grep -Eq "age [0-9]+s"
    [ ! -d "$lock" ]
}

@test "lock held by a live PID is honored, with the observed age and owner" {
    lock="$SANDBOX_HOME/.autospec/.update.lock.d"
    mkdir -p "$lock"
    sleep 30 &
    live_pid=$!
    printf '%s\n' "$live_pid" > "$lock/owner.pid"
    printf '%s\n' "$(( $(date -u +%s) - 12 ))" > "$lock/owner.epoch"
    _shim_curl_succeeding
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" CURL_MARKER="$SANDBOX_HOME" \
        bash "$SCRIPT" autospec-run
    _stop_stood_in_owner "$live_pid"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "concurrent update in progress: owner pid $live_pid is live"
    echo "$output" | grep -Eq "lock age [0-9]+s"
    [ ! -e "$SANDBOX_HOME/curl-invoked" ]
    [ -d "$lock" ]
}

@test "version drift between installed-version and remote-version is surfaced" {
    mkdir -p "$SANDBOX_HOME/.autospec"
    printf 'oldsha1\n' > "$SANDBOX_HOME/.autospec/installed-version"
    printf 'newsha2\n' > "$SANDBOX_HOME/.autospec/remote-version"
    date -u +'%Y-%m-%dT%H:%M:%SZ' > "$SANDBOX_HOME/.autospec/last-update-check"
    _shim_curl_failing
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "self-update drift: installed oldsha1 is behind remote newsha2"
    [ -s "$SANDBOX_HOME/.autospec/self-update-health.json" ]
    run jq -e '.reason | contains("version-drift")' "$SANDBOX_HOME/.autospec/self-update-health.json"
    [ "$status" -eq 0 ]
}

@test "throttle stamp older than the alarm threshold fails loudly" {
    mkdir -p "$SANDBOX_HOME/.autospec"
    older="$(date -u -d '4 days ago' +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -v-4d +'%Y-%m-%dT%H:%M:%SZ')"
    printf '%s\n' "$older" > "$SANDBOX_HOME/.autospec/last-update-check"
    _shim_curl_failing
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" \
        AUTOSPEC_SELF_UPDATE_STALE_ALARM_SECS=86400 bash "$SCRIPT" autospec-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -Eq "self-update has not completed for [0-9]+h"
    [ -s "$SANDBOX_HOME/.autospec/self-update-health.json" ]
    run jq -e '.reason | contains("throttle-stamp-stale")' "$SANDBOX_HOME/.autospec/self-update-health.json"
    [ "$status" -eq 0 ]
}

@test "doctor reports a healthy install and exits 0" {
    mkdir -p "$SANDBOX_HOME/.autospec"
    printf 'abc1234\n' > "$SANDBOX_HOME/.autospec/installed-version"
    printf 'abc1234\n' > "$SANDBOX_HOME/.autospec/remote-version"
    date -u +'%Y-%m-%dT%H:%M:%SZ' > "$SANDBOX_HOME/.autospec/last-update-check"
    _shim_curl_succeeding
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" CURL_MARKER="$SANDBOX_HOME" \
        bash "$SCRIPT" --doctor
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "lock: absent"
    echo "$output" | grep -q "throttle: ok"
    echo "$output" | grep -q "version: ok installed abc1234 == remote abc1234"
    echo "$output" | grep -q "last failure: none"
    [ ! -e "$SANDBOX_HOME/curl-invoked" ]
}

@test "doctor reports lock age, owner and drift, exits 1, and clears the stale lock" {
    lock="$SANDBOX_HOME/.autospec/.update.lock.d"
    mkdir -p "$lock"
    printf '%s\n' "999999" > "$lock/owner.pid"
    printf '%s\n' "$(( $(date -u +%s) - 7200 ))" > "$lock/owner.epoch"
    printf 'oldsha1\n' > "$SANDBOX_HOME/.autospec/installed-version"
    printf 'newsha2\n' > "$SANDBOX_HOME/.autospec/remote-version"
    _shim_curl_succeeding
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" CURL_MARKER="$SANDBOX_HOME" \
        bash "$SCRIPT" --doctor
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "lock: STALE"
    echo "$output" | grep -q "owner pid 999999 is not a live process"
    echo "$output" | grep -Eq "age [0-9]+s, threshold [0-9]+s"
    echo "$output" | grep -q "version: DRIFT installed oldsha1 != remote newsha2"
    [ -d "$lock" ]

    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" CURL_MARKER="$SANDBOX_HOME" \
        bash "$SCRIPT" --clear-stale-lock
    echo "$output" | grep -q "lock: cleared"
    [ ! -d "$lock" ]
    [ ! -e "$SANDBOX_HOME/curl-invoked" ]
}

@test "doctor runs even when AUTOSPEC_NO_SELF_UPDATE=1 opts out the preflight" {
    mkdir -p "$SANDBOX_HOME/.autospec"
    printf 'abc1234\n' > "$SANDBOX_HOME/.autospec/installed-version"
    printf 'abc1234\n' > "$SANDBOX_HOME/.autospec/remote-version"
    date -u +'%Y-%m-%dT%H:%M:%SZ' > "$SANDBOX_HOME/.autospec/last-update-check"
    _shim_curl_succeeding
    run env HOME="$SANDBOX_HOME" PATH="$SHIMDIR:$PATH" CURL_MARKER="$SANDBOX_HOME" \
        AUTOSPEC_NO_SELF_UPDATE=1 bash "$SCRIPT" --doctor
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "throttle: ok"
    [ ! -e "$SANDBOX_HOME/curl-invoked" ]
}
