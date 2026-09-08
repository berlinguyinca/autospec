#!/usr/bin/env bats
# tests/local-dispatch-credentials.bats — TDD for the R9 guardrails in
# scripts/local-dispatch.sh: no ambient credentials, a cwd pinned to the
# issue's worktree, and no package installation.
#
# The negative path the issue names: a dispatch launched with a token in the
# environment MUST refuse (not warn), and an install attempt must exit
# non-zero with a blocker message.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/local-dispatch.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/local-dispatch-cred-XXXXXX")"
    STUBS="$TMP/bin"
    mkdir -p "$STUBS"
    PROMPT="$TMP/prompt.txt"
    printf 'do the thing\n' > "$PROMPT"
    CAP="$TMP/capability.json"
    ORIG_PATH="$PATH"

    # A Codex that advertises --oss and records how it was invoked.
    cat > "$STUBS/codex" <<EOF
#!/usr/bin/env bash
if [ "\$1" = "--help" ]; then
    printf 'Usage: codex\n      --oss\n      --local-provider <OSS_PROVIDER>\n'
    exit 0
fi
printf '%s\n' "\$*" > "$TMP/codex-args"
exit 0
EOF
    chmod +x "$STUBS/codex"
    export PATH="$STUBS:$ORIG_PATH"

    jq -n --arg m "qwen3:32b" \
        '{accelerator:{usable:true, reason:""},
          local_models:[{model:$m, dispatch_recommended:true}]}' > "$CAP"

    # The issue's worktree: any git checkout with a .git entry.
    WT="$TMP/wt"
    git init -q "$WT"
}

teardown() {
    export PATH="${ORIG_PATH:-$PATH}"
    rm -rf "$TMP"
}

# The script refuses to start with ambient credentials, and this host may
# legitimately carry one (e.g. an API key for the build system). The proceeding
# tests therefore launch it with every credential-bearing variable removed from
# the test process's environment, computed at call time (bats-core does not
# carry setup() unsets into the test body here). The refusal tests inject their
# token explicitly and use the plain script instead.
run_scrubbed() {
    local -a unset_args=()
    local v
    for v in $(compgen -e); do
        case "_${v}_" in
            *_TOKEN_*|*_SECRET_*|*_PASSWORD_*|*_PASSWD_*|*_CREDENTIAL_*|*_APIKEY_*|*_PRIVATE_*|*_AUTHORIZATION_*|*_KEY_*)
                unset_args+=(-u "$v") ;;
        esac
    done
    env "${unset_args[@]}" "$@"
}

# A codex stub that dumps the environment the executor process actually sees.
env_dumping_codex() {
    cat > "$STUBS/codex" <<'EOF'
#!/usr/bin/env bash
if [ "$1" = "--help" ]; then printf '      --oss\n'; exit 0; fi
env | sort
exit 0
EOF
    chmod +x "$STUBS/codex"
}

# ── no ambient credentials ────────────────────────────────────────────────────

@test "a dispatch launched with a token in the environment refuses (exit 3)" {
    run env GITHUB_TOKEN="ghp_deadbeef00" bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 3 ]
    [[ "$output" == *"credential variable"* ]]
    [[ "$output" == *"GITHUB_TOKEN"* ]]
    [ ! -f "$TMP/codex-args" ]
}

@test "an API-key-style variable also refuses, naming it" {
    run env MY_SERVICE_API_KEY="sk-live-123" bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP" \
        --dry-run
    [ "$status" -eq 3 ]
    [[ "$output" == *"credential variable"* ]]
    [[ "$output" == *"MY_SERVICE_API_KEY"* ]]
}

@test "a credential name with a plain-looking value still refuses" {
    # The check is on the NAME: a variable named *_PASSWORD holds a secret
    # whatever its value happens to be at probe time.
    run env DB_PASSWORD="hunter2" bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 3 ]
    [[ "$output" == *"DB_PASSWORD"* ]]
}

@test "non-credential variables do not refuse and are not named" {
    run run_scrubbed env RANDOM_APP_FLAG=1 PATH="$PATH" bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP" \
        --dry-run
    [ "$status" -eq 0 ]
}

# ── only allowlisted variables reach the executor ─────────────────────────────

@test "a non-allowlisted variable does not reach the executor process" {
    env_dumping_codex
    run run_scrubbed env RANDOM_APP_FLAG=1 ANOTHER_PLAIN_VAR=2 bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 0 ]
    [[ "$output" != *"RANDOM_APP_FLAG"* ]]
    [[ "$output" != *"ANOTHER_PLAIN_VAR"* ]]
}

@test "allowlisted variables do reach the executor process" {
    env_dumping_codex
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 0 ]
    [[ "$output" == *"PATH="* ]]
    [[ "$output" == *"HOME="* ]]
}

@test "an allowlist override passes exactly the listed variables through" {
    env_dumping_codex
    run run_scrubbed env AUTOSPEC_LOCAL_ENV_ALLOWLIST="PATH HOME MY_EXTRA" MY_EXTRA=42 \
        bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 0 ]
    [[ "$output" == *"MY_EXTRA=42"* ]]
    [[ "$output" != *"AUTOSPEC_LOCAL_ENV_ALLOWLIST"* ]]
}

@test "a credential variable named in the allowlist override still refuses" {
    run env AUTOSPEC_LOCAL_ENV_ALLOWLIST="PATH HOME GITHUB_TOKEN" GITHUB_TOKEN=x \
        bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 3 ]
    [[ "$output" == *"GITHUB_TOKEN"* ]]
}

# ── no package installation ───────────────────────────────────────────────────

@test "an install attempt exits non-zero with a blocker message" {
    # The executor (model) decides it needs a system package and goes to the
    # package manager. The shadowed stub must deny it, and the failed dispatch
    # must surface as a non-zero exit — never a silent self-resolution.
    cat > "$STUBS/codex" <<'EOF'
#!/usr/bin/env bash
if [ "$1" = "--help" ]; then printf '      --oss\n'; exit 0; fi
if apt-get install -y curl; then
    exit 0
else
    printf 'model: install failed, cannot complete the task\n' >&2
    exit 7
fi
EOF
    chmod +x "$STUBS/codex"
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via apt-get is DENIED"* ]]
    [[ "$output" == *"BLOCKER"* ]]
}

# Every common package manager is shadowed, not just the first: each of these
# tests stands in for the model reaching for a specific manager.
_install_stub() {
    local pm="$1"
    cat > "$STUBS/codex" <<EOF
#!/usr/bin/env bash
if [ "\$1" = "--help" ]; then printf '      --oss\n'; exit 0; fi
if $pm install -y curl; then exit 0; else exit 7; fi
EOF
    chmod +x "$STUBS/codex"
}

@test "an dpkg install attempt is denied" {
    _install_stub dpkg
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via dpkg is DENIED"* ]]
}

@test "an dnf install attempt is denied" {
    _install_stub dnf
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via dnf is DENIED"* ]]
}

@test "an yum install attempt is denied" {
    _install_stub yum
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via yum is DENIED"* ]]
}

@test "an zypper install attempt is denied" {
    _install_stub zypper
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via zypper is DENIED"* ]]
}

@test "an pacman install attempt is denied" {
    _install_stub pacman
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via pacman is DENIED"* ]]
}

@test "an apk install attempt is denied" {
    _install_stub apk
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via apk is DENIED"* ]]
}

@test "an brew install attempt is denied" {
    _install_stub brew
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via brew is DENIED"* ]]
}

@test "an nix install attempt is denied" {
    _install_stub nix
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via nix is DENIED"* ]]
}

@test "an port install attempt is denied" {
    _install_stub port
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via port is DENIED"* ]]
}

@test "an opkg install attempt is denied" {
    _install_stub opkg
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 7 ]
    [[ "$output" == *"package installation via opkg is DENIED"* ]]
}

@test "the deny stubs are not visible on the dry-run path" {
    # Dry-run performs no exec, so it must succeed without a runtime and must
    # still report the invocation plan.
    run run_scrubbed bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP" \
        --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"--oss"* ]]
}

# ── --cwd is pinned to the worktree ───────────────────────────────────────────

@test "--cwd that is not an absolute path is refused" {
    run run_scrubbed bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --capability-file "$CAP" --cwd "relative/dir" --dry-run
    [ "$status" -eq 3 ]
    [[ "$output" == *"absolute path"* ]]
}

@test "--cwd outside a git worktree is refused" {
    PLAIN="$TMP/plain"
    mkdir -p "$PLAIN"
    run run_scrubbed bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --capability-file "$CAP" --cwd "$PLAIN" --dry-run
    [ "$status" -eq 3 ]
    [[ "$output" == *"git worktree"* ]]
}

@test "--cwd missing on disk is refused" {
    run run_scrubbed bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --capability-file "$CAP" --cwd "$TMP/absent-dir" --dry-run
    [ "$status" -eq 3 ]
    [[ "$output" == *"not a directory"* ]]
}

@test "--cwd pinned to the worktree itself is accepted" {
    run run_scrubbed bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --capability-file "$CAP" --cwd "$WT" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"cwd=$WT"* ]]
}

@test "--cwd inside a subdirectory of the worktree is accepted" {
    SUB="$WT/src"
    mkdir -p "$SUB"
    run run_scrubbed bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --capability-file "$CAP" --cwd "$SUB" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"cwd=$SUB"* ]]
}

@test "the original three preconditions still refuse as before" {
    # R9 must not have weakened the pre-existing gates.
    minbin="$TMP/minbin"
    mkdir -p "$minbin"
    ln -s "$(command -v bash)" "$minbin/bash"
    ln -s "$(command -v grep)" "$minbin/grep"
    ln -s "$STUBS/codex" "$minbin/codex"
    run run_scrubbed env PATH="$minbin" bash "$SCRIPT" \
        --model qwen3:32b --prompt-file "$PROMPT" --skip-capability-check
    [ "$status" -eq 3 ]
    [[ "$output" == *"timeout"* ]]
    run run_scrubbed bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --capability-file "$TMP/nope.json"
    [ "$status" -eq 3 ]
    [[ "$output" == *"no capability document"* ]]
}
