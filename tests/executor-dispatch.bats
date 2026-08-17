#!/usr/bin/env bats
# tests/executor-dispatch.bats — the §16 request/result contract for
# scripts/executor-dispatch.sh.
#
# The point of the abstraction is that orchestration never branches on provider,
# so the assertions that matter most here are the ones about *sameness*: every
# harness, and every failure path, returns one envelope key set.
#
# The "unknown over a fabricated 0" rule and schema conformance live in
# tests/executor-dispatch-metrics.bats. Shared argument-aware harness stubs live
# in tests/executor-dispatch-test-helpers.bash.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/executor-dispatch.sh"

setup() {
    load "${BATS_TEST_DIRNAME}/executor-dispatch-test-helpers.bash"
    executor_dispatch_setup
}

teardown() {
    executor_dispatch_teardown
}

# ── argument surface ──────────────────────────────────────────────────────────

@test "executor-dispatch.sh is executable" {
    run test -x "$SCRIPT"
    [ "$status" -eq 0 ]
}

@test "--help exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
}

@test "a missing --request is a usage error (exit 1)" {
    run bash "$SCRIPT"
    [ "$status" -eq 1 ]
    [[ "$output" == *"--request is required"* ]]
}

@test "an unknown option is a usage error (exit 1)" {
    run bash "$SCRIPT" --request "$REQ" --wat
    [ "$status" -eq 1 ]
}

@test "a trailing --request with no value exits rather than spinning" {
    # `shift 2` on the last argument consumes nothing, so the option loop would
    # never terminate. A hang is worse than a bad exit code: an orchestrator
    # waiting on this dispatch never gets its worktree back. perl's alarm stands
    # in for timeout(1), which a stock macOS does not ship; a spin shows up as
    # 142 (SIGALRM) rather than as a suite that never returns.
    run perl -e 'alarm 10; exec @ARGV or exit 127' bash "$SCRIPT" --request
    [ "$status" -eq 1 ]
}

@test "an unreadable request file is a usage error (exit 1)" {
    run bash "$SCRIPT" --request "$TMP/absent.json"
    [ "$status" -eq 1 ]
}

@test "a missing jq fails closed with exit 2" {
    minbin="$(path_without jq)"
    run env PATH="$minbin" bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 2 ]
    [[ "$output" == *"jq"* ]]
}

@test "--schema-path resolves the result schema from the checkout" {
    run bash "$SCRIPT" --schema-path
    [ "$status" -eq 0 ]
    [[ "$output" == *"autospec-dispatch-result.schema.json"* ]]
    run test -f "$output"
    [ "$status" -eq 0 ]
}

# ── request validation ────────────────────────────────────────────────────────

@test "malformed request JSON yields failure_class=invalid_request and exit 1" {
    printf 'not json at all\n' > "$REQ"
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 1 ]
    run jq -r '.failure_class' <<< "$output"
    [ "$output" = "invalid_request" ]
}

@test "a request missing workspace yields failure_class=invalid_request" {
    jq 'del(.workspace)' "$REQ" > "$TMP/r2"; mv "$TMP/r2" "$REQ"
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 1 ]
    run jq -r '.failure_class' <<< "$output"
    [ "$output" = "invalid_request" ]
}

@test "a workspace that is not a directory yields failure_class=invalid_request" {
    req_with workspace '"/definitely/not/here"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 1 ]
    run jq -r '.failure_class' <<< "$output"
    [ "$output" = "invalid_request" ]
}

@test "a role outside the 14-role vocabulary is rejected" {
    req_with role '"vibe_officer"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 1 ]
    run jq -r '.failure_class' <<< "$output"
    [ "$output" = "invalid_request" ]
}

@test "every one of the 14 snake_case roles is accepted" {
    for role in orchestrator planner architect test_planner implementer \
                code_reviewer test_reviewer qa_verifier documentation_writer \
                documentation_reviewer ui_ux_reviewer security_reviewer \
                researcher advisor; do
        jq --arg r "$role" '.role = $r' "$REQ" > "$TMP/r2"; mv "$TMP/r2" "$REQ"
        run bash "$SCRIPT" --request "$REQ"
        [ "$status" -eq 0 ]
    done
}

@test "a non-integer timeout is rejected" {
    req_with timeout '"soon"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 1 ]
}

# ── the negative case the issue names: unknown provider ───────────────────────

@test "an unknown provider exits 12 with failure_class=unsupported_provider" {
    req_with provider '"skynet"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 12 ]
    run jq -r '.failure_class' <<< "$output"
    [ "$output" = "unsupported_provider" ]
}

@test "the unsupported-provider envelope still carries every §16 key" {
    req_with provider '"skynet"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 12 ]
    run jq -cS 'keys' <<< "$output"
    [ "$output" = "$ENVELOPE_KEYS" ]
}

@test "a local runtime name is not silently accepted as a harness" {
    # ollama/lmstudio/vllm/llamacpp are runtimes, not harnesses. Until that
    # wiring lands they must refuse loudly rather than reach a cloud harness.
    for runtime in ollama lmstudio vllm llamacpp; do
        jq --arg p "$runtime" '.provider = $p' "$REQ" > "$TMP/r2"; mv "$TMP/r2" "$REQ"
        run bash "$SCRIPT" --request "$REQ"
        [ "$status" -eq 12 ]
    done
}

@test "the harness vocabulary is spelled exactly claude codex opencode" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"claude"* ]]
    [[ "$output" == *"codex"* ]]
    [[ "$output" == *"opencode"* ]]
    [[ "$output" != *"llama.cpp"* ]]
}

@test "an absent harness binary exits 3 with failure_class=harness_unavailable" {
    minbin="$(path_without claude)"
    run env PATH="$minbin" bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 3 ]
    run jq -r '.failure_class' <<< "$output"
    [ "$output" = "harness_unavailable" ]
}

# ── round-trips: one envelope per harness ─────────────────────────────────────

@test "the claude adapter round-trips a request to a success envelope" {
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.status' <<< "$output"
    [ "$output" = "success" ]
}

@test "the codex adapter round-trips a request to a success envelope" {
    req_with provider '"codex"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.status, .output' <<< "$output"
    [[ "$output" == *"success"* ]]
    [[ "$output" == *"codex did the work"* ]]
}

@test "the opencode adapter round-trips a request to a success envelope" {
    req_with provider '"opencode"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.status, .output' <<< "$output"
    [[ "$output" == *"success"* ]]
    [[ "$output" == *"opencode did the work"* ]]
}

@test "all three harnesses return an identical envelope key set" {
    bash "$SCRIPT" --request "$REQ" > "$TMP/claude.json"
    jq '.provider = "codex"' "$REQ" > "$TMP/req-codex.json"
    bash "$SCRIPT" --request "$TMP/req-codex.json" > "$TMP/codex.json"
    jq '.provider = "opencode"' "$REQ" > "$TMP/req-opencode.json"
    bash "$SCRIPT" --request "$TMP/req-opencode.json" > "$TMP/opencode.json"

    run jq -cS -s 'map(keys) | unique | length' \
        "$TMP/claude.json" "$TMP/codex.json" "$TMP/opencode.json"
    [ "$output" = "1" ]

    run jq -cS 'keys' "$TMP/claude.json"
    [ "$output" = "$ENVELOPE_KEYS" ]
}

@test "the failure envelope carries the same key set as the success envelope" {
    bash "$SCRIPT" --request "$REQ" > "$TMP/ok.json"
    jq '.provider = "skynet"' "$REQ" > "$TMP/req-bad.json"
    bash "$SCRIPT" --request "$TMP/req-bad.json" > "$TMP/bad.json" || true

    run jq -cS -s 'map(keys) | unique | length' "$TMP/ok.json" "$TMP/bad.json"
    [ "$output" = "1" ]
}

# ── timeout ───────────────────────────────────────────────────────────────────

@test "a dispatch exceeding its timeout yields status=timeout and exit 4" {
    cat > "$STUBS/opencode" <<'EOF'
#!/usr/bin/env bash
printf 'partial work before the axe\n'
sleep 30
EOF
    chmod +x "$STUBS/opencode"
    req_with provider '"opencode"'
    req_with timeout '1'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 4 ]
    run jq -r '.status, .failure_class' <<< "$output"
    [[ "$output" == *"timeout"* ]]
}

@test "a timed-out dispatch still returns the partial output it captured" {
    cat > "$STUBS/opencode" <<'EOF'
#!/usr/bin/env bash
printf 'partial work before the axe\n'
sleep 30
EOF
    chmod +x "$STUBS/opencode"
    req_with provider '"opencode"'
    req_with timeout '1'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 4 ]
    run jq -r '.output' <<< "$output"
    [[ "$output" == *"partial work before the axe"* ]]
}

@test "a timeout is applied even when the request omits one" {
    jq 'del(.timeout)' "$REQ" > "$TMP/r2"; mv "$TMP/r2" "$REQ"
    run env AUTOSPEC_EXECUTOR_TIMEOUT_SECS=1 bash "$SCRIPT" --request "$REQ" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"timeout_secs=1"* ]]
}

# ── harness failure ───────────────────────────────────────────────────────────

@test "a harness exiting non-zero yields status=failure and failure_class=harness_error" {
    cat > "$STUBS/opencode" <<'EOF'
#!/usr/bin/env bash
printf 'it went wrong\n'
exit 7
EOF
    chmod +x "$STUBS/opencode"
    req_with provider '"opencode"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 5 ]
    run jq -r '.status, .failure_class' <<< "$output"
    [[ "$output" == *"failure"* ]]
    [[ "$output" == *"harness_error"* ]]
}

@test "a success envelope reports failure_class=none, not an empty string" {
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.failure_class' <<< "$output"
    [ "$output" = "none" ]
}

# ── §52 backward compatibility with the existing cloud path ───────────────────

@test "a cloud-only request dispatches with no routing or capability document" {
    run env AUTOSPEC_MODEL_CAPABILITY="$TMP/definitely-absent.json" \
        bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.status' <<< "$output"
    [ "$output" = "success" ]
}

@test "the dispatch runs in the requested workspace, as dispatch-implementer.sh does" {
    cat > "$STUBS/opencode" <<'EOF'
#!/usr/bin/env bash
pwd -P
EOF
    chmod +x "$STUBS/opencode"
    req_with provider '"opencode"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.output' <<< "$output"
    [[ "$output" == *"$(cd "$WS" && pwd -P)"* ]]
}

# ── patch capture ─────────────────────────────────────────────────────────────

@test "patch is \"unknown\" when the workspace is not a git worktree" {
    req_with provider '"opencode"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.patch' <<< "$output"
    [ "$output" = "unknown" ]
}

@test "patch carries the diff a harness produced in a git workspace" {
    git -C "$WS" init -q
    git -C "$WS" config user.email t@example.com
    git -C "$WS" config user.name t
    printf 'before\n' > "$WS/f.txt"
    git -C "$WS" add f.txt
    git -C "$WS" commit -qm init
    cat > "$STUBS/opencode" <<'EOF'
#!/usr/bin/env bash
printf 'after\n' > f.txt
EOF
    chmod +x "$STUBS/opencode"
    req_with provider '"opencode"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.patch' <<< "$output"
    [[ "$output" == *"-before"* ]]
    [[ "$output" == *"+after"* ]]
}

# ── adapter argument construction (--dry-run) ─────────────────────────────────

@test "--dry-run prints the codex invocation with --skip-git-repo-check" {
    req_with provider '"codex"'
    run bash "$SCRIPT" --request "$REQ" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"exec"* ]]
    [[ "$output" == *"--skip-git-repo-check"* ]]
    [[ "$output" == *"$WS"* ]]
}

@test "--dry-run carries the requested model into the claude invocation" {
    run bash "$SCRIPT" --request "$REQ" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"--model"* ]]
    [[ "$output" == *"test-model"* ]]
}

@test "requested tools reach the claude adapter's allow list" {
    run bash "$SCRIPT" --request "$REQ" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"--allowedTools"* ]]
    [[ "$output" == *"Read,Edit"* ]]
}

@test "--dry-run prints the opencode run subcommand" {
    req_with provider '"opencode"'
    run bash "$SCRIPT" --request "$REQ" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"run"* ]]
}

@test "--dry-run does not invoke the harness" {
    rm -f "$TMP/claude-args"
    run bash "$SCRIPT" --request "$REQ" --dry-run
    [ "$status" -eq 0 ]
    run test -f "$TMP/claude-args"
    [ "$status" -ne 0 ]
}

@test "the codex adapter really is refused without --skip-git-repo-check" {
    # Guards the argument-aware stub itself: if the flag were dropped, this test
    # must go red rather than the refusal being silently absorbed.
    req_with provider '"codex"'
    bash "$SCRIPT" --request "$REQ" >/dev/null || true
    run grep -c -- '--skip-git-repo-check' "$TMP/codex-args"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

# ── real harness binaries, skipped when absent ────────────────────────────────

@test "a real claude binary resolves to a dispatchable invocation" {
    real="$(PATH="$ORIG_PATH" command -v claude 2>/dev/null || true)"
    if [ -z "$real" ]; then skip "claude not installed"; fi
    run env PATH="$ORIG_PATH" AUTOSPEC_EXECUTOR_CLAUDE_BIN="$real" \
        bash "$SCRIPT" --request "$REQ" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"$real"* ]]
}

@test "a real codex binary resolves to a dispatchable invocation" {
    real="$(PATH="$ORIG_PATH" command -v codex 2>/dev/null || true)"
    if [ -z "$real" ]; then skip "codex not installed"; fi
    jq '.provider = "codex"' "$REQ" > "$TMP/r2"; mv "$TMP/r2" "$REQ"
    run env PATH="$ORIG_PATH" AUTOSPEC_EXECUTOR_CODEX_BIN="$real" \
        bash "$SCRIPT" --request "$REQ" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"--skip-git-repo-check"* ]]
}

@test "a real opencode binary resolves to a dispatchable invocation" {
    real="$(PATH="$ORIG_PATH" command -v opencode 2>/dev/null || true)"
    if [ -z "$real" ]; then skip "opencode not installed"; fi
    jq '.provider = "opencode"' "$REQ" > "$TMP/r2"; mv "$TMP/r2" "$REQ"
    run env PATH="$ORIG_PATH" AUTOSPEC_EXECUTOR_OPENCODE_BIN="$real" \
        bash "$SCRIPT" --request "$REQ" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"run"* ]]
}
