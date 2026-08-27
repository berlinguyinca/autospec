#!/usr/bin/env bats
# tests/fleet/fleet-provisioning.bats
#
# fleet-init.sh must actually provision checkouts (clone missing repos,
# fetch+fast-forward existing ones) so fleet-run.sh can find them and launch
# a conductor instead of reporting "checkout not found". Every test stubs
# `git` on PATH with a logging stub — no test may reach a real remote or
# mutate a real checkout with real git.

setup() {
    TMP="$(mktemp -d)"
    mkdir -p "$TMP/bin"
    INIT="${BATS_TEST_DIRNAME}/../../skills/autospec-fleet/scripts/fleet-init.sh"
    RUN="${BATS_TEST_DIRNAME}/../../skills/autospec-fleet/scripts/fleet-run.sh"
    FLEET_LIB="${BATS_TEST_DIRNAME}/../../skills/autospec-fleet/scripts/fleet-lib.sh"
    WORKSPACE="$TMP/ws"

    export GIT_LOG="$TMP/git.log"
    : > "$GIT_LOG"

    # Logging git stub. Never touches the network or a real remote/checkout.
    # Behavior is steered per-test via env vars so the same stub can prove
    # clone, idempotent update, dirty-skip, non-ff-skip, and failure paths
    # without ever invoking real git.
    cat > "$TMP/bin/git" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
args=("$@")
printf '%s\n' "${args[*]}" >> "$GIT_LOG"

case "${args[0]:-}" in
    clone)
        dest="${args[${#args[@]}-1]}"
        rc="${GIT_STUB_CLONE_RESULT:-0}"
        if [ "$rc" -eq 0 ]; then
            mkdir -p "$dest/.git"
        fi
        exit "$rc"
        ;;
    -C)
        subcmd="${args[2]:-}"
        case "$subcmd" in
            status)
                printf '%s' "${GIT_STUB_STATUS_OUTPUT:-}"
                exit 0
                ;;
            fetch)
                exit "${GIT_STUB_FETCH_RESULT:-0}"
                ;;
            merge)
                exit "${GIT_STUB_MERGE_RESULT:-0}"
                ;;
            *)
                exit 1
                ;;
        esac
        ;;
    *)
        exit 1
        ;;
esac
SH
    chmod +x "$TMP/bin/git"

    # Confirm the stub actually shadows the real binary before any test
    # relies on it — a stale PATH here would silently run real git.
    export PATH="$TMP/bin:$PATH"
    [ "$(command -v git)" = "$TMP/bin/git" ]
}

teardown() {
    rm -rf "$TMP"
}

# ── Idempotent provisioning ─────────────────────────────────────────────────

@test "missing repo is cloned" {
    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/org/repo-a.git"
    [ "$status" -eq 0 ]
    [ -d "$WORKSPACE/org__repo-a/.git" ]
    grep -q '^clone ' "$GIT_LOG"
}

@test "an already-cloned repo is fetched and fast-forwarded, not re-cloned" {
    mkdir -p "$WORKSPACE/org__repo-a/.git"

    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/org/repo-a.git"
    [ "$status" -eq 0 ]
    run grep -c '^clone ' "$GIT_LOG"
    [ "$status" -ne 0 ] || [ "$output" -eq 0 ]
    grep -q 'fetch' "$GIT_LOG"
    grep -q 'merge' "$GIT_LOG"
}

@test "re-running provisioning on a clean checkout is safe and cheap (no re-clone)" {
    bash "$INIT" --workspace "$WORKSPACE" "https://github.com/org/repo-a.git" >/dev/null
    clone_count_before="$(grep -c '^clone ' "$GIT_LOG" || true)"

    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/org/repo-a.git"
    [ "$status" -eq 0 ]
    clone_count_after="$(grep -c '^clone ' "$GIT_LOG" || true)"
    [ "$clone_count_after" -eq "$clone_count_before" ]
}

# ── Never destroy local work ────────────────────────────────────────────────

@test "a checkout with uncommitted changes is skipped, not reset" {
    mkdir -p "$WORKSPACE/org__repo-a/.git"
    export GIT_STUB_STATUS_OUTPUT=" M dirty-file.txt"

    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/org/repo-a.git"
    [ "$status" -eq 0 ]
    [ -n "$(printf '%s' "$output" | grep -F -- 'local changes present')" ]
    [ -n "$(printf '%s' "$output" | grep -F -- 'skipping')" ]
    [ -n "$(printf '%s' "$output" | grep -F -- 'code_health:fleet_provision_update_skipped')" ]
    [ -n "$(printf '%s' "$output" | grep -F -- 'reason=dirty_checkout')" ]
    run grep -q 'fetch' "$GIT_LOG"
    [ "$status" -ne 0 ]
}

@test "a non-fast-forwardable update is skipped, not force-updated" {
    mkdir -p "$WORKSPACE/org__repo-a/.git"
    export GIT_STUB_MERGE_RESULT=1

    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/org/repo-a.git"
    [ "$status" -eq 0 ]
    [ -n "$(printf '%s' "$output" | grep -F -- 'would not fast-forward')" ]
    [ -n "$(printf '%s' "$output" | grep -F -- 'code_health:fleet_provision_update_skipped')" ]
    [ -n "$(printf '%s' "$output" | grep -F -- 'reason=not_fast_forward')" ]
    run grep -q -- '--hard\|reset\|clean' "$GIT_LOG"
    [ "$status" -ne 0 ]
}

# ── Per-repo failure, not fatal ─────────────────────────────────────────────

@test "a clone failure for one repo does not abort provisioning of the next" {
    export GIT_STUB_CLONE_RESULT=1

    run bash "$INIT" --workspace "$WORKSPACE" \
        "https://github.com/org/repo-fails.git" \
        "https://github.com/org/repo-ok.git"
    [ "$status" -eq 0 ]
    [ -n "$(printf '%s' "$output" | grep -F -- 'code_health:fleet_provision_clone_failed')" ]

    # The stub only fails clone when GIT_STUB_CLONE_RESULT=1 is set for
    # every clone call, so re-run cleanly to prove the second repo alone
    # (unaffected by the first repo's failure) still provisions.
    unset GIT_STUB_CLONE_RESULT
    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/org/repo-ok.git"
    [ "$status" -eq 0 ]
    [ -d "$WORKSPACE/org__repo-ok/.git" ]
}

@test "a fetch failure emits a code_health marker and does not abort the run" {
    mkdir -p "$WORKSPACE/org__repo-a/.git"
    export GIT_STUB_FETCH_RESULT=1

    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/org/repo-a.git"
    [ "$status" -eq 0 ]
    [ -n "$(printf '%s' "$output" | grep -F -- 'code_health:fleet_provision_fetch_failed')" ]
}

# ── Path containment ─────────────────────────────────────────────────────────

@test "path containment: traversal-shaped repo URLs are rejected before any filesystem write" {
    # normalize_repo_url's owner/repo regex rejects a multi-slash path
    # outright (no clone/checkout is ever attempted); the process exits
    # nonzero for a lone malformed URL, but the *loop* around
    # fleet_provision_repo in fleet-init.sh still treats this the same as
    # any other per-repo failure — it never aborts a multi-repo run.
    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/../../etc.git"
    [ -n "$(printf '%s' "$output" | grep -F -- 'unsupported repo URL')" ]

    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/a/../../b.git"
    [ -n "$(printf '%s' "$output" | grep -F -- 'unsupported repo URL')" ]

    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com//etc/passwd"
    [ -n "$(printf '%s' "$output" | grep -F -- 'unsupported repo URL')" ]

    # The workspace root itself may exist (fleet-init.sh always ensures it),
    # but it must contain no repo checkout at all — nothing was cloned
    # anywhere, in or out of the workspace.
    [ ! -e "$TMP/etc" ]
    [ ! -e "/tmp/etc" ]
    if [ -d "$WORKSPACE" ]; then
        run find "$WORKSPACE" -mindepth 1
        [ -z "$output" ]
    fi
    [ ! -s "$GIT_LOG" ]
}

@test "path containment: leading-dash slug is created as a literal directory name, never as a flag" {
    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/-rf/repo"
    [ "$status" -eq 0 ]
    [ -d "$WORKSPACE/-rf__repo/.git" ]
    # Every checkout produced must resolve strictly under the workspace.
    run find "$WORKSPACE" -mindepth 1 -maxdepth 1 -type d
    for d in "${lines[@]}"; do
        case "$d" in
            "$WORKSPACE"/*) : ;;
            *) fail "checkout escaped workspace: $d" ;;
        esac
    done
}

@test "fleet_path_within_workspace rejects a computed path outside the workspace" {
    run bash -c 'source "$1"; fleet_path_within_workspace "$2" "$3"' _ \
        "$FLEET_LIB" "/tmp/ws" "/tmp/other/repo"
    [ "$status" -ne 0 ]

    run bash -c 'source "$1"; fleet_path_within_workspace "$2" "$3"' _ \
        "$FLEET_LIB" "/tmp/ws" "/tmp/ws/.."
    [ "$status" -ne 0 ]

    run bash -c 'source "$1"; fleet_path_within_workspace "$2" "$3"' _ \
        "$FLEET_LIB" "/tmp/ws" "/tmp/ws/org__repo"
    [ "$status" -eq 0 ]
}

# ── --dry-run stays completely inert ────────────────────────────────────────

@test "dry-run creates nothing and performs no git invocation" {
    run bash "$INIT" --dry-run --workspace "$WORKSPACE" "https://github.com/org/repo-a.git"
    [ "$status" -eq 0 ]
    [ -n "$(printf '%s' "$output" | grep -F -- "fleet: plan clone org/repo-a -> $WORKSPACE/org__repo-a")" ]
    [ ! -e "$WORKSPACE" ]
    [ ! -s "$GIT_LOG" ]
}

@test "dry-run is inert even when a checkout already exists" {
    mkdir -p "$WORKSPACE/org__repo-a/.git"
    : > "$GIT_LOG"

    run bash "$INIT" --dry-run --workspace "$WORKSPACE" "https://github.com/org/repo-a.git"
    [ "$status" -eq 0 ]
    [ ! -s "$GIT_LOG" ]
}

# ── End-to-end: provisioning closes the fleet-run chain ────────────────────

@test "end-to-end: provisioning + fleet-run produces a real conductor command instead of checkout-not-found" {
    mkdir -p "$TMP/bin"
    export FLEET_SPAWN_LOG="$TMP/spawn.log"
    : > "$FLEET_SPAWN_LOG"

    cat > "$TMP/bin/autospec-autonomous" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
orig="$*"
sub="${1:-}"
shift || true
case "$sub" in
    start) ;;
    *) exit 1 ;;
esac
while [ $# -gt 0 ]; do
    case "$1" in
        --repo-dir|--repo) shift 2 ;;
        *) exit 1 ;;
    esac
done
printf '%s\n' "$orig" >> "$FLEET_SPAWN_LOG"
SH
    chmod +x "$TMP/bin/autospec-autonomous"

    cat > "$TMP/bin/autospec" <<'SH'
#!/usr/bin/env bash
case "$*" in *"queue ready"*) printf '{"batch":[{"number":1}]}' ;; *) printf '' ;; esac
SH
    chmod +x "$TMP/bin/autospec"
    export AUTOSPEC_FLEET_QUEUE_BIN="$TMP/bin/autospec"
    export AUTOSPEC_HEARTBEAT_DIR="$TMP/hb"
    mkdir -p "$AUTOSPEC_HEARTBEAT_DIR"

    cat > "$TMP/fleet.yml" <<YML
version: 1
workspace: $WORKSPACE
parallel_repos: 1
repos:
  - url: https://github.com/o/a.git
    enabled: true
YML

    # Before provisioning: fleet-run reaches the repo but finds no checkout.
    run bash "$RUN" --config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    [ -n "$(printf '%s' "$output" | grep -F -- 'checkout not found')" ]
    [ ! -s "$FLEET_SPAWN_LOG" ]

    # Provisioning creates the checkout (stubbed git, no network).
    run bash "$INIT" --workspace "$WORKSPACE" "https://github.com/o/a.git"
    [ "$status" -eq 0 ]
    [ -d "$WORKSPACE/o__a/.git" ]

    # After provisioning: fleet-run finds the checkout and launches.
    run bash "$RUN" --config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    [ -z "$(printf '%s' "$output" | grep -F -- 'checkout not found')" ]
    [ -s "$FLEET_SPAWN_LOG" ]
    grep -q -- '--repo-dir' "$FLEET_SPAWN_LOG"
    grep -q -- '--repo o/a' "$FLEET_SPAWN_LOG"

    echo "produced command: $(cat "$FLEET_SPAWN_LOG")" >&3
}
