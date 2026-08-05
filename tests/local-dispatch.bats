#!/usr/bin/env bats
# tests/local-dispatch.bats — TDD for scripts/local-dispatch.sh
#
# Every refusal path is tested, because the failure this script exists to prevent
# is expensive: an older Codex silently ignoring --oss would bill a PAID cloud
# model while the caller believed the dispatch was local.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/local-dispatch.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/local-dispatch-XXXXXX")"
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

    cap_with() {
        jq -n --arg m "$1" --argjson ok "$2" --arg reason "${3:-}" \
            '{accelerator:{usable:($ok), reason:$reason},
              local_models:[{model:$m, dispatch_recommended:$ok}]}' > "$CAP"
    }
}

teardown() {
    export PATH="${ORIG_PATH:-$PATH}"
    rm -rf "$TMP"
}

# A PATH with the listed tools removed (a stub dir alone cannot hide a real one).
path_without() {
    excluded=" $* "
    minbin="$TMP/minbin"
    rm -rf "$minbin"; mkdir -p "$minbin"
    for tool in bash jq grep date awk sed tr head cat mv rm mkdir dirname mktemp timeout flock codex; do
        case "$excluded" in *" $tool "*) continue ;; esac
        resolved="$(PATH="$ORIG_PATH:$STUBS" command -v "$tool" 2>/dev/null || true)"
        if [ -n "$resolved" ]; then ln -sf "$resolved" "$minbin/$tool"; fi
    done
    printf '%s' "$minbin"
}

# ── argument surface ──────────────────────────────────────────────────────────

@test "local-dispatch.sh is executable" {
    run test -x "$SCRIPT"
    [ "$status" -eq 0 ]
}

@test "--help exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
}

@test "requires --model" {
    run bash "$SCRIPT" --prompt-file "$PROMPT"
    [ "$status" -eq 1 ]
    [[ "$output" == *"--model is required"* ]]
}

@test "requires --prompt-file" {
    run bash "$SCRIPT" --model qwen3:32b
    [ "$status" -eq 1 ]
}

@test "rejects a missing prompt file" {
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$TMP/absent.txt"
    [ "$status" -eq 1 ]
    [[ "$output" == *"prompt file not found"* ]]
}

@test "rejects an unsupported provider" {
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" --provider wat
    [ "$status" -eq 1 ]
    [[ "$output" == *"unsupported provider"* ]]
}

@test "rejects a non-numeric timeout" {
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" --timeout-secs abc
    [ "$status" -eq 1 ]
}

# ── refusal paths: each exits 3 so the caller keeps its cloud tier ────────────

@test "refuses when codex is absent" {
    minbin="$(path_without codex)"
    run env PATH="$minbin" bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --skip-capability-check
    [ "$status" -eq 3 ]
    [[ "$output" == *"codex CLI not found"* ]]
}

@test "refuses when codex does not advertise --oss" {
    # The expensive failure: an older Codex ignores the flag and bills a cloud model.
    cat > "$STUBS/codex" <<'EOF'
#!/usr/bin/env bash
if [ "$1" = "--help" ]; then printf 'Usage: codex\n      --model <M>\n'; exit 0; fi
exit 0
EOF
    chmod +x "$STUBS/codex"
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" --skip-capability-check
    [ "$status" -eq 3 ]
    [[ "$output" == *"does not advertise --oss"* ]]
}

@test "refuses when the capability document is absent" {
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --capability-file "$TMP/nope.json"
    [ "$status" -eq 3 ]
    [[ "$output" == *"no capability document"* ]]
}

@test "refuses a model that is not dispatch_recommended, naming the reason" {
    cap_with "qwen3:32b" false "nvml_driver_library_mismatch"
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 3 ]
    [[ "$output" == *"not dispatch_recommended"* ]]
    [[ "$output" == *"nvml_driver_library_mismatch"* ]]
}

@test "refuses a model absent from the capability document" {
    cap_with "some-other-model" true
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 3 ]
    [[ "$output" == *"not found in the capability document"* ]]
}

@test "refuses when timeout(1) is unavailable rather than dispatching unbounded" {
    cap_with "qwen3:32b" true
    minbin="$(path_without timeout)"
    run env PATH="$minbin" bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --capability-file "$CAP"
    [ "$status" -eq 3 ]
    [[ "$output" == *"timeout"* ]]
}

# ── proceeding ────────────────────────────────────────────────────────────────

@test "proceeds when the model is dispatch_recommended" {
    cap_with "qwen3:32b" true
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --capability-file "$CAP" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"--oss"* ]]
}

@test "--skip-capability-check bypasses the probe gate" {
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --skip-capability-check --dry-run
    [ "$status" -eq 0 ]
}

@test "the dry-run invocation carries provider, model and a bounded timeout" {
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --provider lmstudio --timeout-secs 42 --skip-capability-check --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"--local-provider lmstudio"* ]]
    [[ "$output" == *"--model qwen3:32b"* ]]
    [[ "$output" == *"timeout_secs=42"* ]]
}

@test "a real dispatch invokes codex exec with the oss flags" {
    cap_with "qwen3:32b" true
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" --capability-file "$CAP"
    [ "$status" -eq 0 ]
    run cat "$TMP/codex-args"
    [[ "$output" == *"exec"* ]]
    [[ "$output" == *"--oss"* ]]
    [[ "$output" == *"--local-provider ollama"* ]]
}

@test "a dispatch exceeding its ceiling reports exit 4, not a generic failure" {
    cap_with "qwen3:32b" true
    # A codex that hangs; the 1s ceiling must convert that into the distinct
    # timeout status so a caller can tell 'too slow' from 'wrong answer'.
    cat > "$STUBS/codex" <<'EOF'
#!/usr/bin/env bash
if [ "$1" = "--help" ]; then printf '      --oss\n      --local-provider <P>\n'; exit 0; fi
sleep 30
EOF
    chmod +x "$STUBS/codex"
    run bash "$SCRIPT" --model qwen3:32b --prompt-file "$PROMPT" \
        --capability-file "$CAP" --timeout-secs 1
    [ "$status" -eq 4 ]
}
