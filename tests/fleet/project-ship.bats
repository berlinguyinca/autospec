#!/usr/bin/env bats
# tests/fleet/project-ship.bats
#
# scripts/project-ship.sh is the `ship` mode chain for autospec-project:
# resolve board -> filter to project_board.repo_allowlist -> write
# autospec-fleet.yml -> provision each allowlisted checkout -> launch a
# per-repo conductor. Every test stubs `git`, `gh`-shaped board readers,
# `autospec-autonomous`, and `autospec` (the queue probe) — no test may
# clone a real remote, spawn a real conductor, or touch the operator's real
# $HOME/~/.autospec.

bats_require_minimum_version 1.5.0

setup() {
    TMP="$(mktemp -d)"
    mkdir -p "$TMP/bin" "$TMP/ws" "$TMP/hb"
    REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
    SHIP="$REPO_ROOT/scripts/project-ship.sh"

    export AUTOSPEC_FLEET_LIB_SCRIPT="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-lib.sh"
    export AUTOSPEC_FLEET_RUN_SCRIPT="$REPO_ROOT/skills/autospec-fleet/scripts/fleet-run.sh"

    export CALL_LOG="$TMP/calls.log"
    : > "$CALL_LOG"
    export GIT_LOG="$TMP/git.log"
    : > "$GIT_LOG"
    export FLEET_SPAWN_LOG="$TMP/spawn.log"
    : > "$FLEET_SPAWN_LOG"
    export AUTOSPEC_HEARTBEAT_DIR="$TMP/hb"

    # ── board-config bridge stub (stands in for `autospec autonomous
    # project-board-config`) — controlled per-test via PB_CONFIG_JSON. ──────
    cat > "$TMP/bin/board-config.sh" <<'SH'
#!/usr/bin/env bash
printf 'board-config %s\n' "$*" >> "$CALL_LOG"
if [ -n "${PB_CONFIG_JSON:-}" ]; then
    printf '%s' "$PB_CONFIG_JSON"
else
    printf '{"url":null,"allowlist":[]}'
fi
SH
    chmod +x "$TMP/bin/board-config.sh"
    export AUTOSPEC_PROJECT_BOARD_CONFIG_BIN="$TMP/bin/board-config.sh"

    # ── board resolve stub (stands in for project-board-resolve.sh --emit
    # repos) — controlled per-test via RESOLVE_REPOS_JSON. Never calls a
    # real `gh`. ─────────────────────────────────────────────────────────────
    cat > "$TMP/bin/resolve.sh" <<'SH'
#!/usr/bin/env bash
printf 'resolve %s\n' "$*" >> "$CALL_LOG"
printf '%s' "${RESOLVE_REPOS_JSON:-[]}"
exit "${RESOLVE_EXIT:-0}"
SH
    chmod +x "$TMP/bin/resolve.sh"
    export AUTOSPEC_PROJECT_BOARD_RESOLVE_BIN="$TMP/bin/resolve.sh"

    # ── git stub: logs every invocation; clone/fetch/status/merge results
    # are steered per-test via env vars, and clone/status can be made to
    # behave differently per checkout path (matches
    # tests/fleet/fleet-provisioning.bats' own stub idiom). ─────────────────
    cat > "$TMP/bin/git" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
args=("$@")
printf '%s\n' "${args[*]}" >> "$GIT_LOG"

case "${args[0]:-}" in
    clone)
        dest="${args[${#args[@]}-1]}"
        rc=0
        case "$dest" in
            *"${GIT_STUB_CLONE_FAIL_SUFFIX:-__NEVER__}") rc=1 ;;
        esac
        if [ "$rc" -eq 0 ]; then
            mkdir -p "$dest/.git"
        fi
        exit "$rc"
        ;;
    -C)
        path="${args[1]:-}"
        subcmd="${args[2]:-}"
        case "$subcmd" in
            status)
                case "$path" in
                    *"${GIT_STUB_DIRTY_SUFFIX:-__NEVER__}") printf ' M file.txt' ;;
                esac
                exit 0
                ;;
            fetch) exit "${GIT_STUB_FETCH_RESULT:-0}" ;;
            merge) exit "${GIT_STUB_MERGE_RESULT:-0}" ;;
            *) exit 1 ;;
        esac
        ;;
    *) exit 1 ;;
esac
SH
    chmod +x "$TMP/bin/git"

    # ── autospec-autonomous stub: validates arguments the way the real
    # conductor parser does (matches tests/fleet/project-board-fleet.bats'
    # stub) — an argument-blind stub would hide a wrong-flag regression. ────
    cat > "$TMP/bin/autospec-autonomous" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
orig="$*"
sub="${1:-}"
shift || true
case "$sub" in
    start) ;;
    *) printf 'autospec-autonomous: unknown subcommand: %s\n' "$sub" >&2; exit 1 ;;
esac
while [ $# -gt 0 ]; do
    case "$1" in
        --repo-dir|--repo) shift 2 ;;
        --repo-dir=*|--repo=*) shift ;;
        --foreground|--force) shift ;;
        *) printf 'autospec-autonomous: unknown argument: %s\n' "$1" >&2; exit 1 ;;
    esac
done
printf '%s\n' "$orig" >> "$FLEET_SPAWN_LOG"
SH
    chmod +x "$TMP/bin/autospec-autonomous"

    # ── queue probe stub: every repo always has ready work. ─────────────────
    cat > "$TMP/bin/autospec" <<'SH'
#!/usr/bin/env bash
case "$*" in *"queue ready"*) printf '{"batch":[{"number":1}]}' ;; *) printf '' ;; esac
SH
    chmod +x "$TMP/bin/autospec"
    export PATH="$TMP/bin:$PATH"
    # See project-board-fleet.bats: target/debug/autospec (if ever built in
    # this worktree) outranks PATH in fleet-run.sh's own resolution order,
    # so this seam must be pinned explicitly or the stub above is bypassed.
    export AUTOSPEC_FLEET_QUEUE_BIN="$TMP/bin/autospec"
}

teardown() {
    rm -rf "$TMP"
}

@test "only the allowlisted repo reaches the fleet config, is provisioned, and is launched; the other never appears in any git or autospec-autonomous call" {
    export PB_CONFIG_JSON='{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/allow"]}'
    export RESOLVE_REPOS_JSON='["o/allow","o/deny"]'

    run bash "$SHIP" --url https://github.com/orgs/o/projects/1 --repo-dir "$TMP/repo" \
        --workspace "$TMP/ws" --fleet-config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    # $output is clobbered by every subsequent `run` below, so the ship
    # invocation's own output must be captured under its own name first.
    ship_output="$output"

    # fleet config carries only the allowlisted repo.
    grep -q 'o/allow.git' "$TMP/fleet.yml"
    run grep -q 'o/deny' "$TMP/fleet.yml"
    [ "$status" -ne 0 ]

    # Reporting: allowed repo provisioned + launched; denied repo skipped.
    echo "$ship_output" | grep -q 'repo=o/deny allowlisted=no action=skipped reason=not-allowlisted'
    echo "$ship_output" | grep -q 'repo=o/allow allowlisted=yes provision=ok'
    echo "$ship_output" | grep -q 'repo=o/allow allowlisted=yes launch=launched'

    # The non-allowlisted repo was never named in any git call.
    run grep -q 'o/deny' "$GIT_LOG"
    [ "$status" -ne 0 ]
    [ -s "$GIT_LOG" ]

    # ...nor in any autospec-autonomous spawn.
    run grep -q 'o/deny' "$FLEET_SPAWN_LOG"
    [ "$status" -ne 0 ]
    grep -q 'o/allow' "$FLEET_SPAWN_LOG"
}

@test "a provisioning failure on one allowlisted repo does not stop the other" {
    export PB_CONFIG_JSON='{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/a","o/b"]}'
    export RESOLVE_REPOS_JSON='["o/a","o/b"]'
    export GIT_STUB_CLONE_FAIL_SUFFIX="o__a"

    run bash "$SHIP" --url https://github.com/orgs/o/projects/1 --repo-dir "$TMP/repo" \
        --workspace "$TMP/ws" --fleet-config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]

    echo "$output" | grep -q 'repo=o/a allowlisted=yes provision=failed'
    echo "$output" | grep -q 'repo=o/b allowlisted=yes provision=ok'
    echo "$output" | grep -q 'repo=o/b allowlisted=yes launch=launched'
    # o/a never got a checkout, so it cannot have been launched.
    echo "$output" | grep -q 'repo=o/a allowlisted=yes launch=skipped:checkout-not-found'
    grep -q 'o/b' "$FLEET_SPAWN_LOG"
    run grep -q 'o/a' "$FLEET_SPAWN_LOG"
    [ "$status" -ne 0 ]
}

@test "a dirty checkout is skipped with its reason surfaced, not silently reported as provisioned" {
    export PB_CONFIG_JSON='{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/allow"]}'
    export RESOLVE_REPOS_JSON='["o/allow"]'
    mkdir -p "$TMP/ws/o__allow/.git"
    export GIT_STUB_DIRTY_SUFFIX="o__allow"

    run bash "$SHIP" --url https://github.com/orgs/o/projects/1 --repo-dir "$TMP/repo" \
        --workspace "$TMP/ws" --fleet-config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]

    echo "$output" | grep -q 'repo=o/allow allowlisted=yes provision=skipped:dirty'
    run grep -q 'o/allow allowlisted=yes provision=ok' <<< "$output"
    [ "$status" -ne 0 ]
    # The dirty checkout was fetched-from-status but never reset/cleaned.
    run grep -q -- '-C .*clean' "$GIT_LOG"
    [ "$status" -ne 0 ]
}

@test "no allowlist configured (no board wired in) is a clean no-op: refuses before any git or resolve call" {
    export PB_CONFIG_JSON='{"url":null,"allowlist":[]}'
    export RESOLVE_REPOS_JSON='["o/allow","o/deny"]'

    run bash "$SHIP" --url https://github.com/orgs/o/projects/1 --repo-dir "$TMP/repo" \
        --workspace "$TMP/ws" --fleet-config "$TMP/fleet.yml"
    [ "$status" -eq 3 ]

    [ ! -f "$TMP/fleet.yml" ]
    [ ! -s "$GIT_LOG" ]
    [ ! -s "$FLEET_SPAWN_LOG" ]
    run grep -q 'resolve' "$CALL_LOG"
    [ "$status" -ne 0 ]
}

@test "an empty resolved-and-allowlisted intersection ships nothing but still exits clean" {
    export PB_CONFIG_JSON='{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/only-on-allowlist"]}'
    export RESOLVE_REPOS_JSON='["o/deny"]'

    run bash "$SHIP" --url https://github.com/orgs/o/projects/1 --repo-dir "$TMP/repo" \
        --workspace "$TMP/ws" --fleet-config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'no allowlisted repos found on this board'
    [ ! -s "$GIT_LOG" ]
    [ ! -s "$FLEET_SPAWN_LOG" ]
}

@test "dry-run previews provisioning and launch without touching git or spawning anything" {
    export PB_CONFIG_JSON='{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/allow"]}'
    export RESOLVE_REPOS_JSON='["o/allow"]'

    run bash "$SHIP" --url https://github.com/orgs/o/projects/1 --repo-dir "$TMP/repo" \
        --workspace "$TMP/ws" --fleet-config "$TMP/fleet.yml" --dry-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'repo=o/allow allowlisted=yes action=plan-provision'
    [ ! -s "$GIT_LOG" ]
    [ ! -s "$FLEET_SPAWN_LOG" ]
}

@test "a board resolve failure aborts with the resolver's own exit code and writes no fleet config" {
    export PB_CONFIG_JSON='{"url":"https://github.com/orgs/o/projects/1","allowlist":["o/allow"]}'
    export RESOLVE_REPOS_JSON='[]'
    export RESOLVE_EXIT=3

    run bash "$SHIP" --url https://github.com/orgs/o/projects/1 --repo-dir "$TMP/repo" \
        --workspace "$TMP/ws" --fleet-config "$TMP/fleet.yml"
    [ "$status" -eq 3 ]
    [ ! -f "$TMP/fleet.yml" ]
    [ ! -s "$GIT_LOG" ]
}
